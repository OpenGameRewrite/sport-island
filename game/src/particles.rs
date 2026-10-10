//! Player for the original NW4R particle emitters (REFF), exported by the
//! extractor to `effects/pln.json`. Rewritten from the public NW4R behaviour
//! (emitter forms, curve keys, fields, child emitters, TEV draw settings).
//!
//! Each [`Effect`] entity owns a tree of emitters. Emitters own particle groups
//! (NW4R particle managers) whose space follows the emitter by the resource's
//! inheritance settings. Simulation runs at the original 60 Hz; groups are
//! rebuilt into camera-facing meshes every rendered frame.

use bevy::asset::{RenderAssetUsages, embedded_asset};
use bevy::camera::visibility::NoFrustumCulling;
use bevy::light::NotShadowCaster;
use bevy::math::Affine3A;
use bevy::mesh::{Indices, MeshVertexAttribute, MeshVertexBufferLayoutRef, PrimitiveTopology, VertexFormat};
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::render_resource::{AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError};
use bevy::render::storage::ShaderBuffer;
use bevy::shader::ShaderRef;
use serde::Deserialize;
use std::collections::HashMap;
use std::f32::consts::{PI, TAU};
use std::sync::Arc;

// ---------------------------------------------------------------- resources

#[derive(Deserialize)]
pub struct EmitterRes {
    common: u32,
    emit_flag: u32,
    emit_life: u16,
    ptcl_life: u16,
    life_rnd: i8,
    interval_rnd: i8,
    emit_rnd: i8,
    emit: f32,
    emit_start: u16,
    interval: u16,
    inherit_ptcl_t: i8,
    inherit_child_emit_t: i8,
    params: [f32; 6],
    emit_div: u16,
    vel_init_rnd: i8,
    vel_momentum_rnd: i8,
    /// Radial, Y axis, random, normal, normal diffusion, specified, specified diffusion.
    vel: [f32; 7],
    spec_dir: [f32; 3],
    scale: [f32; 3],
    rotate: [f32; 3],
    translate: [f32; 3],
    seed: u32,
    draw: DrawRes,
    ptcl: PtclRes,
    ptcl_init: u16,
    ptcl_tracks: Vec<Curve>,
}

#[derive(Deserialize)]
struct DrawRes {
    flags: u16,
    tevs: u8,
    tev_tex: [u8; 4],
    tev_color: [[u8; 4]; 4],
    color_op: [[u8; 5]; 4],
    tev_alpha: [[u8; 4]; 4],
    alpha_op: [[u8; 5]; 4],
    kcsel: [u8; 4],
    kasel: [u8; 4],
    blend: [u8; 4],
    color_in: [u8; 8],
    alpha_in: [u8; 8],
    /// Type, cycle, random, amplitude.
    flick: [u16; 4],
    pivot: [i8; 2],
    /// Particle type, option, direction, axis, ...
    ptype: [u8; 7],
}

#[derive(Deserialize)]
struct PtclRes {
    colors: [[u8; 4]; 4],
    size: [f32; 2],
    scale: [f32; 2],
    rotate: [f32; 3],
    tex_scale: [[f32; 2]; 3],
    tex_rot: [f32; 3],
    tex_trans: [[f32; 2]; 3],
    wrap: u16,
    reverse: u8,
    rot_offset_rnd: [u8; 3],
    rot_offset: [f32; 3],
    textures: [String; 3],
}

#[derive(Deserialize)]
struct Curve {
    /// Byte offset of the animated particle parameter.
    kind: u8,
    /// 0 U8, 3 F32, 4 texture, 5 child, 6 rotation, 7 field.
    ty: u8,
    enable: u8,
    proc: u8,
    loops: u8,
    seed: u16,
    frames: u16,
    #[serde(default)]
    keys: Vec<Key>,
    #[serde(default)]
    ranges: Vec<Range>,
    #[serde(default)]
    random: Vec<Range>,
    #[serde(default)]
    names: Vec<String>,
    #[serde(default)]
    field: Option<Field>,
    #[serde(default)]
    baked: Vec<Vec<f32>>,
}

const PROC_SYNC: u8 = 0x04;
const PROC_STOP: u8 = 0x08;
const PROC_TIMING: u8 = 0x10;
const PROC_LOOP: u8 = 0x20;
const PROC_TURN: u8 = 0x40;
const PROC_FIT: u8 = 0x80;

#[derive(Deserialize, Clone, Copy, Default)]
#[serde(default)]
struct Child {
    speed: i16,
    scale: u8,
    alpha: u8,
    color: u8,
    /// 0 one-shot burst at the particle, 1 persistent emitter.
    kind: u8,
    /// 1 follow the particle's direction, 2 inherit its rotation.
    flag: u8,
    name: u16,
}

#[derive(Deserialize)]
struct Key {
    frame: u16,
    mode: u8,
    #[serde(default)]
    interp: [u8; 4],
    #[serde(default)]
    values: Vec<f32>,
    #[serde(default)]
    index: u16,
    #[serde(flatten)]
    child: Child,
}

#[derive(Deserialize)]
struct Range {
    /// Per component (base, span); span is scaled for a 16-bit random value.
    v: Vec<[f32; 2]>,
    /// Rotation entries may also flip the sign at random.
    sign: bool,
}

#[derive(Deserialize)]
struct Field {
    /// 0 emitter, 1 world, 2 particle space.
    space: u8,
    /// 0 velocity, 1 position.
    target: u8,
    power: f32,
    params: Vec<f32>,
}

#[derive(Resource)]
pub struct ParticleLib(HashMap<String, Arc<EmitterRes>>);

// ---------------------------------------------------------------- runtime

/// NW4R's linear congruential generator.
#[derive(Clone, Copy)]
struct Rng(u32);
impl Rng {
    fn next(&mut self) -> u32 {
        self.0 = lcg(self.0);
        self.0 >> 16
    }
    fn float(&mut self) -> f32 { self.next() as f32 / 65536.0 }
}
fn lcg(x: u32) -> u32 { x.wrapping_mul(0x343FD).wrapping_add(0x269EC3) }

/// Per-key random seed: particle seed, curve seed, loop and range index, byte-folded.
fn key_hash(ptcl: u16, curve: u16, lp: u32, index: u16) -> u32 {
    let h = (ptcl as u32).wrapping_mul(0x3F81F635)
        .wrapping_add((curve as u32).wrapping_mul(0x30A74193))
        .wrapping_add(lp.wrapping_mul(0x7B929))
        .wrapping_add((index as u32).wrapping_mul(0x371097E7))
        .wrapping_add(0x4BF53);
    let [b0, b1, b2, b3] = h.to_be_bytes();
    let c2 = b2 ^ b3;
    let c1 = b1 ^ c2;
    u32::from_be_bytes([b0 ^ c1, c1, c2, b3])
}

/// Animatable particle parameters, addressed by the curves' byte offsets:
/// colours (0-15), then floats from offset 16 (size, scale, rotation, texture SRTs).
#[derive(Clone)]
struct Params {
    colors: [u8; 16],
    f: [f32; 22],
}
const SIZE: usize = 0;
const SCALE: usize = 2;
const ROT: usize = 4;
const TEX_SCALE: usize = 7;
const TEX_ROT: usize = 13;
const TEX_TRANS: usize = 16;

impl Params {
    fn new(p: &PtclRes) -> Self {
        let mut colors = [0; 16];
        for (i, c) in p.colors.iter().enumerate() { colors[i * 4..i * 4 + 4].copy_from_slice(c); }
        let mut f = [0.0; 22];
        f[SIZE..SIZE + 2].copy_from_slice(&p.size);
        f[SCALE..SCALE + 2].copy_from_slice(&p.scale);
        f[ROT..ROT + 3].copy_from_slice(&p.rotate);
        for i in 0..3 {
            f[TEX_SCALE + 2 * i..TEX_SCALE + 2 * i + 2].copy_from_slice(&p.tex_scale[i]);
            f[TEX_ROT + i] = p.tex_rot[i];
            f[TEX_TRANS + 2 * i..TEX_TRANS + 2 * i + 2].copy_from_slice(&p.tex_trans[i]);
        }
        Self { colors, f }
    }
}

#[derive(Clone)]
struct Particle {
    pos: Vec3,
    prev: Vec3,
    vel: Vec3,
    life: u32,
    tick: u32,
    seed: u16,
    momentum: f32,
    p: Params,
    rot_offset: Vec3,
    flick: i32,
}

/// One NW4R particle manager: particles in a space derived from its emitter.
struct Group {
    res: Arc<EmitterRes>,
    name: String,
    inherit: (bool, bool, f32),
    mtx: Affine3A,
    particles: Vec<Particle>,
    entity: Option<Entity>,
}

struct Emitter {
    res: Arc<EmitterRes>,
    parent: Option<usize>,
    local: Affine3A,
    global: Affine3A,
    tick: u32,
    wait: u32,
    interval_wait: u32,
    count: f32,
    first: bool,
    rng: Rng,
    seed: u16,
    active: bool,
    groups: Vec<Group>,
}

/// A running effect. Its owner moves it with `root` and ends it with `stop`;
/// the entity despawns itself once every particle has died.
#[derive(Component)]
pub struct Effect {
    pub root: Affine3A,
    emitters: Vec<Emitter>,
    rng: Rng,
    /// Replaces colour layer 0's primary and secondary RGB, as the game does for player-coloured effects.
    tint: Option<[[u8; 3]; 2]>,
}

impl Effect {
    /// Stops all emission; live particles finish their lives.
    pub fn stop(&mut self) {
        for e in &mut self.emitters { e.active = false; }
    }
    fn finished(&self) -> bool {
        self.emitters.iter().all(|e| !e.active && e.groups.iter().all(|g| g.particles.is_empty()))
    }
}

fn euler(r: [f32; 3]) -> Quat { Quat::from_euler(EulerRot::ZYX, r[2], r[1], r[0]) }

/// An emitter-derived space keeping only the inherited scale, rotation and part of the translation.
fn restructure(m: Affine3A, scale: bool, rot: bool, trans: f32) -> Affine3A {
    let (s, r, t) = m.to_scale_rotation_translation();
    Affine3A::from_scale_rotation_translation(if scale { s } else { Vec3::ONE }, if rot { r } else { Quat::IDENTITY }, t * trans)
}

impl Emitter {
    fn new(res: Arc<EmitterRes>, name: &str, parent: Option<usize>, local: Affine3A, global: Affine3A, rng: &mut Rng) -> Self {
        let seed = if res.seed != 0 { res.seed } else { rng.next() };
        let c = res.common;
        let inherit = (c & 0x20 != 0, c & 0x40 != 0, res.inherit_ptcl_t as f32 / 100.0);
        let group = Group { res: res.clone(), name: name.into(), inherit, mtx: restructure(global, inherit.0, inherit.1, inherit.2), particles: vec![], entity: None };
        Self { wait: res.emit_start as u32, res, parent, local, global, tick: 0, interval_wait: 0, count: 0.0, first: true, rng: Rng(seed), seed: seed as u16, active: true, groups: vec![group] }
    }
    /// The space children of this emitter follow.
    fn child_space(&self) -> Affine3A {
        let c = self.res.common;
        restructure(self.global, c & 0x80 != 0, c & 0x100 != 0, self.res.inherit_child_emit_t as f32 / 100.0)
    }
}

pub fn spawn(commands: &mut Commands, lib: &ParticleLib, name: &str, root: Affine3A) -> Option<Entity> {
    spawn_tinted(commands, lib, name, root, None)
}

pub fn spawn_tinted(commands: &mut Commands, lib: &ParticleLib, name: &str, root: Affine3A, tint: Option<[[u8; 3]; 2]>) -> Option<Entity> {
    let Some(res) = lib.0.get(name) else { warn!("Missing particle emitter {name}"); return None };
    let mut rng = Rng(name.bytes().fold(0x1234_5678, |h, b| h.rotate_left(5) ^ b as u32));
    let local = local_mtx(res);
    let emitter = Emitter::new(res.clone(), name, None, local, root * local, &mut rng);
    Some(commands.spawn((Name::new(format!("Effect {name}")), Effect { root, emitters: vec![emitter], rng, tint }, Transform::default(), Visibility::default())).id())
}

fn local_mtx(res: &EmitterRes) -> Affine3A {
    Affine3A::from_scale_rotation_translation(Vec3::from(res.scale), euler(res.rotate), Vec3::from(res.translate))
}

// ---------------------------------------------------------------- curves

/// Curve-local frame for a tick: (integer frame, exact frame, loop index).
fn key_time(c: &Curve, tick: u32, life: u32) -> (u32, f32, u32) {
    let frames = c.frames as u32;
    if frames < 2 { return (0, 0.0, tick); }
    let (inf, turn, fit, loops) = (c.proc & PROC_LOOP != 0, c.proc & PROC_TURN != 0, c.proc & PROC_FIT != 0, c.loops as u32);
    let span = frames - 1;
    if !inf && loops < 2 {
        if !fit { let k = tick.min(span); return (k, k as f32, 0); }
        let f = if life <= 1 { span as f32 } else { (tick as f32 * span as f32 / (life - 1) as f32).min(span as f32) };
        return (f as u32, f, 0);
    }
    if fit {
        if tick + 1 < life {
            let scaled = tick as f32 * (loops as f32 * span as f32 / (life - 1) as f32);
            let lp = (scaled / span as f32) as u32;
            let mut f = scaled - (lp * span) as f32;
            if turn && lp & 1 == 1 { f = span as f32 - f; }
            return (f as u32, f, lp);
        }
        let k = if !turn || loops & 1 == 1 { span } else { 0 };
        return (k, k as f32, loops.saturating_sub(1));
    }
    let lp = tick / span;
    if !inf && loops <= lp {
        let k = if !turn || loops & 1 == 1 { span } else { 0 };
        return (k, k as f32, loops.saturating_sub(1));
    }
    let k = if turn && lp & 1 == 1 { span * (lp + 1) - tick } else { tick - lp * span };
    (k, k as f32, lp)
}

/// Values of one key: inline, or drawn from its range / random table.
fn key_values(c: &Curve, key: &Key, seed: u16, lp: u32, u8s: bool) -> [f32; 4] {
    let mut out = [0.0; 4];
    if key.mode == 0 {
        for (o, v) in out.iter_mut().zip(&key.values) { *o = *v; }
        return out;
    }
    let mut h = key_hash(seed, c.seed, lp, key.index);
    let entry = if key.mode & 2 != 0 {
        if c.random.is_empty() { return out; }
        let i = (h >> 16) as usize % c.random.len();
        h = lcg(h);
        &c.random[i]
    } else {
        match c.ranges.get(key.index as usize) { Some(r) => r, None => return out }
    };
    let flip = entry.sign && h & 0x10000 == 0;
    let mut r = if entry.sign { lcg(h) } else { h };
    for (o, [base, span]) in out.iter_mut().zip(&entry.v) {
        *o = if u8s {
            (*base as i32 + (*span as i32 * (r >> 16) as u16 as i16 as i32) / 32768).clamp(0, 255) as f32
        } else {
            base + span * (r >> 16) as f32
        };
        if flip { *o = -*o; }
        if c.proc & PROC_SYNC == 0 { r = lcg(r); }
    }
    out
}

fn interpolate(a: f32, b: f32, t: f32, mode: u8) -> f32 {
    if a == b { return a; }
    match mode & 3 {
        0 => a + t * (b - a),
        1 if mode == 1 => a + t * t * (3.0 - 2.0 * t) * (b - a),
        1 => {
            let s0 = if mode & 8 != 0 { 1.5 } else { 0.0 };
            let s1 = if mode & 4 != 0 { 1.5 } else { 0.0 };
            a + t * (b - a) * (s0 + t * t * (s0 + s1 - 2.0) + t * (3.0 - 2.0 * s0 - s1))
        }
        2 => a,
        _ => 0.0,
    }
}

/// Evaluates a U8/F32/rotation curve into `out` (one slot per enable bit).
fn evaluate(c: &Curve, tick: u32, life: u32, seed: u16, mut write: impl FnMut(usize, f32)) {
    let (k, f, lp) = key_time(c, tick, life);
    let bits = (0..4).filter(|b| c.enable & (1 << b) != 0);
    if !c.baked.is_empty() {
        let frame = &c.baked[(k as usize).min(c.baked.len() - 1)];
        for (b, v) in bits.zip(frame) { write(b, *v); }
        return;
    }
    let keys = &c.keys;
    if keys.is_empty() { return; }
    let u8s = c.ty == 0;
    let last = keys.len() - 1;
    let i = keys.iter().rposition(|key| key.frame as u32 <= k).unwrap_or(0);
    let exact = k < keys[0].frame as u32 || i == last || (keys[i].frame as u32 == k && f == k as f32);
    let a = key_values(c, &keys[i], seed, lp, u8s);
    if exact {
        for (n, b) in bits.enumerate() { write(b, a[n]); }
        return;
    }
    let b_vals = key_values(c, &keys[i + 1], seed, lp, u8s);
    let t = (f - keys[i].frame as f32) / (keys[i + 1].frame - keys[i].frame) as f32;
    for (n, b) in bits.enumerate() {
        let v = interpolate(a[n], b_vals[n], t, keys[i].interp[b]);
        write(b, if u8s { v.floor().clamp(0.0, 255.0) } else { v });
    }
}

/// Child-curve keys firing on this tick.
fn child_keys(c: &Curve, tick: u32) -> impl Iterator<Item = &Key> {
    let frames = c.frames.max(1) as u32;
    let frame = if c.proc & PROC_LOOP != 0 || c.loops > 1 { Some(tick % frames) } else { (tick < frames).then_some(tick) };
    c.keys.iter().filter(move |k| Some(k.frame as u32) == frame)
}

// ---------------------------------------------------------------- emission

/// A child emitter request raised by a particle's child curve.
struct Spawn {
    emitter: usize,
    res: Arc<EmitterRes>,
    name: String,
    child: Child,
    pos: Vec3,
    vel: Vec3,
    dir: Vec3,
    size: Vec2,
    colors: [u8; 16],
    rotate: Vec3,
}

fn calc_velocity(res: &EmitterRes, rng: &mut Rng, normal: Vec3, from_origin: Vec3, from_y: Vec3) -> Vec3 {
    let [radial, yaxis, random, normal_p, _, spec, spec_diff] = res.vel;
    let mut v = from_origin * radial + from_y * yaxis;
    if random != 0.0 {
        let (rx, ry, rz) = (rng.float() * TAU, rng.float() * TAU, rng.float() * TAU);
        let (sr, cr) = rx.sin_cos();
        let (sp, cp) = ry.sin_cos();
        let (sh, ch) = rz.sin_cos();
        v += Vec3::new(cr * sp * ch + sr * sh, cr * sp * sh - sr * ch, cr * cp) * random;
    }
    v += normal * normal_p;
    if spec != 0.0 {
        let dir = if spec_diff == 0.0 {
            euler(res.spec_dir) * Vec3::Y
        } else {
            let spread = Quat::from_euler(EulerRot::ZYX, 0.0, rng.float() * TAU, rng.float() * spec_diff);
            euler(res.spec_dir) * spread * Vec3::Y
        };
        v += dir * spec;
    }
    if res.vel_init_rnd != 0 {
        v *= 1.0 - res.vel_init_rnd as f32 / 100.0 * (rng.float() * 2.0 - 1.0);
    }
    v
}

/// Emitter-form positions: (position, normal, direction from origin, direction from the Y axis).
fn form(res: &EmitterRes, rng: &mut Rng, count: u32) -> Vec<(Vec3, Vec3, Vec3, Vec3)> {
    let p = res.params;
    let flags = res.emit_flag;
    let size = |v: f32| if v.abs() > f32::EPSILON { v } else { f32::EPSILON };
    let same = flags & (1 << 25) != 0;
    let fixed_angle = flags & (1 << 18) != 0;
    let even = flags & (1 << 17) != 0;
    let uniform = flags & (1 << 24) != 0;
    let mut out = vec![];
    match res.emit_flag & 0xFF {
        // Disc.
        0 => {
            let (sx, sz) = (size(p[0]), if same { size(p[0]) } else { size(p[4]) });
            let offset = if fixed_angle { p[2] } else { rng.float() * TAU };
            let step = if even { (p[3] - p[2]) / res.emit_div.max(1) as f32 } else { 0.0 };
            let mut angle = 0.0;
            for _ in 0..count {
                let inner = p[1] / 100.0;
                let mut dist = rng.float();
                dist = if uniform { (dist + inner * inner * (1.0 - dist)).sqrt() } else { dist + inner * (1.0 - dist) };
                if !even { angle = (p[3] - p[2]) * rng.float(); }
                let (sa, ca) = (offset + angle).sin_cos();
                let from_y = Vec3::new(sa, 0.0, -ca);
                let pos = Vec3::new(from_y.x * dist * sx, 0.0, from_y.z * dist * sz);
                let normal = if res.vel[4] == 0.0 { Vec3::Y } else {
                    let cone = dist * res.vel[4];
                    Vec3::new(cone.sin() * sa, cone.cos(), cone.sin() * -ca)
                };
                out.push((pos, normal, from_y, from_y));
                if even { angle += step; }
            }
        }
        // Sphere.
        8 => {
            let (sx, sy, sz) = if same { (size(p[0]), size(p[0]), size(p[0])) } else { (size(p[0]), size(p[4]), size(p[5])) };
            let inner = p[1] / 100.0;
            let start = p[2] + if fixed_angle { 0.0 } else { rng.float() * TAU };
            let push = |out: &mut Vec<_>, pos: Vec3, mut from_y: Vec3| {
                let normal = pos.normalize_or_zero();
                if from_y != Vec3::ZERO { from_y = from_y.normalize(); }
                out.push((pos, normal, normal, from_y));
            };
            if even {
                // Rings of latitude, widening then narrowing.
                let max_x = count * 2 + 1;
                let (mut count_x, mut max_angle, mut count_angle) = (0, 1u32, 0);
                let sweep = p[3] - p[2];
                let circle = !(0.0001..TAU - 0.0001).contains(&(sweep % TAU));
                for _ in 0..count * count * 4 + 2 {
                    let x = count_x as f32 / (max_x - 1) as f32 * PI + PI / 2.0;
                    let angle = if max_angle == 1 { start } else if circle { start + count_angle as f32 / max_angle as f32 * sweep } else { start + count_angle as f32 / (max_angle - 1) as f32 * sweep };
                    count_angle += 1;
                    if count_angle == max_angle {
                        count_angle = 0;
                        count_x += 1;
                        max_angle = if count_x * 2 < max_x { if max_angle != 1 { max_angle + 4 } else { max_angle + 3 } } else if max_angle != 4 { max_angle.saturating_sub(4).max(1) } else { 1 };
                    }
                    let mut radius = rng.float();
                    if uniform { radius = 1.0 - radius * radius * radius; }
                    radius = radius * (1.0 - inner) + inner;
                    let mut from_y = Vec3::new(-angle.sin(), 0.0, angle.cos());
                    let pos = Vec3::new(sx * radius * x.cos() * from_y.x, sy * radius * -x.sin(), sz * radius * x.cos() * from_y.z);
                    if x.cos() < 0.0 { from_y = -from_y; }
                    push(&mut out, pos, from_y);
                }
            } else {
                for _ in 0..count {
                    let mut dist = rng.float();
                    dist = if uniform { let d = 1.0 - dist * dist * dist; d + inner * (1.0 - d) } else { dist + inner * (1.0 - dist) };
                    let angle = (p[3] - p[2]) * rng.float() + start;
                    let x = rng.float() * PI + PI / 2.0;
                    let pos = Vec3::new(sx * dist * -x.cos() * angle.sin(), sy * dist * -x.sin(), sz * dist * x.cos() * angle.cos());
                    push(&mut out, pos, Vec3::new(pos.x, 0.0, pos.z));
                }
            }
        }
        // Point; other forms (line, cube, cylinder, torus) are not used by this game's flight effects.
        _ => for _ in 0..count {
            let rx = rng.float() * 2.0 - 1.0;
            let nx = if rx >= 0.0 { (0.66 + 0.34 * rx) * rx } else { (0.66 - 0.34 * rx) * rx };
            let radius = (1.0 - nx * nx).sqrt();
            let a = rng.float() * TAU;
            let normal = Vec3::new(nx, a.cos() * radius, a.sin() * radius);
            let from_y = Vec3::new(normal.x, 0.0, normal.z).normalize_or_zero();
            out.push((Vec3::ZERO, normal, normal, from_y));
        },
    }
    out
}

/// Creates particles from `res` at `space` (emitter to group space) into `group`.
fn emit(res: &EmitterRes, rng: &mut Rng, count: u32, space: Affine3A, group: &mut Group, reference: Option<&Spawn>) {
    let to_group = group.mtx.inverse();
    for (pos, normal, from_origin, from_y) in form(res, rng, count) {
        let vel = calc_velocity(res, rng, normal, from_origin, from_y);
        let mut life = res.ptcl_life as f32;
        if res.life_rnd != 0 { life -= (life * rng.float() * res.life_rnd as f32 / 100.0).floor(); }
        let momentum = 1.0 + res.vel_momentum_rnd as f32 / 100.0 * rng.float();
        let mut p = Params::new(&res.ptcl);
        let mut rot_offset = Vec3::ZERO;
        for i in 0..3 {
            let base = res.ptcl.rot_offset[i];
            let rnd = res.ptcl.rot_offset_rnd[i] as f32;
            rot_offset[i] = base + if rnd > 0.0 { rnd * (base * (2.0 * rng.float() - 1.0)) / 100.0 } else { 0.0 };
        }
        let seed = rng.next() as u16;
        let flick = (rng.next() % 254) as i32 - 127;
        let mut vel = space.transform_vector3(vel);
        if let Some(r) = reference {
            let c = r.child;
            if c.speed != 0 { vel += to_group.transform_vector3(r.vel) * c.speed as f32 / 100.0; }
            if c.scale != 0 { p.f[SIZE] = r.size.x * c.scale as f32 / 100.0; p.f[SIZE + 1] = r.size.y * c.scale as f32 / 100.0; }
            for i in 0..16 {
                let inherit = if i % 4 == 3 { c.alpha } else { c.color };
                // u8 arithmetic as in the original: percentages above 100 wrap.
                if inherit != 0 { p.colors[i] = (inherit as u32 * r.colors[i] as u32 / 100) as u8; }
            }
            if c.flag & 2 != 0 { rot_offset = r.rotate; }
        }
        let pos = space.transform_point3(pos);
        group.particles.push(Particle { pos, prev: pos, vel, life: (life as u32).max(1), tick: 0, seed, momentum, p, rot_offset, flick });
    }
}

/// The emitter's per-frame emission count (rate, randomness, interval).
fn emission_count(e: &mut Emitter) -> u32 {
    if e.interval_wait > 0 { e.interval_wait -= 1; return 0; }
    let res = &e.res;
    e.interval_wait = res.interval as u32;
    if res.interval_rnd != 0 {
        e.interval_wait += ((res.interval as f32 * res.interval_rnd as f32 / 100.0 - 1.0) * e.rng.float()).ceil().max(0.0) as u32;
    }
    if res.emit_flag & (1 << 17) != 0 {
        e.count = res.emit_div as f32;
    } else {
        let mut count = res.emit;
        if res.emit_rnd != 0 { count += res.emit * res.emit_rnd as f32 / 100.0 * (2.0 * e.rng.float() - 1.0); }
        e.count += count;
        if e.first && res.emit != 0.0 && e.count < 1.0 { e.count = 1.0; }
    }
    e.first = false;
    let n = e.count.max(0.0) as u32;
    e.count -= n as f32;
    n
}

// ---------------------------------------------------------------- simulation

fn step_effect(fx: &mut Effect, lib: &ParticleLib) {
    for i in 0..fx.emitters.len() {
        let parent = match fx.emitters[i].parent { Some(p) => fx.emitters[p].child_space(), None => fx.root };
        let e = &mut fx.emitters[i];
        e.global = parent * e.local;
        for g in &mut e.groups { g.mtx = restructure(e.global, g.inherit.0, g.inherit.1, g.inherit.2); }
        if e.active && e.res.common & 4 == 0 && e.wait == 0 && e.tick >= e.res.emit_life as u32 { e.active = false; }
    }
    let mut spawns = vec![];
    for (ei, e) in fx.emitters.iter_mut().enumerate() {
        let timing = (e.tick, if e.res.common & 4 != 0 { u32::MAX } else { e.res.emit_life as u32 }, e.seed);
        let emitter_rot = e.global.to_scale_rotation_translation().1;
        for g in &mut e.groups {
            let to_group = g.mtx.inverse();
            let res = g.res.clone();
            g.particles.retain_mut(|p| {
                if p.tick >= p.life { return false; }
                let (mut add_vel, mut add_pos) = (Vec3::ZERO, Vec3::ZERO);
                let start = if p.tick == 0 { 0 } else { res.ptcl_init as usize };
                for c in res.ptcl_tracks.iter().skip(start) {
                    if c.proc & PROC_STOP != 0 { continue; }
                    let (tick, life, seed) = if c.proc & PROC_TIMING != 0 { timing } else { (p.tick, p.life, p.seed) };
                    match c.ty {
                        0 => { let base = c.kind as usize; evaluate(c, tick, life, seed, |b, v| if let Some(x) = p.p.colors.get_mut(base + b) { *x = v as u8 }); }
                        3 | 6 if c.kind >= 16 => { let base = (c.kind as usize - 16) / 4; evaluate(c, tick, life, seed, |b, v| if let Some(x) = p.p.f.get_mut(base + b) { *x = v }); }
                        5 => for key in child_keys(c, tick) {
                            let Some(name) = c.names.get(key.child.name as usize) else { continue };
                            let Some(child) = lib.0.get(name) else { continue };
                            let world = g.mtx.transform_point3(p.pos);
                            spawns.push(Spawn { emitter: ei, res: child.clone(), name: name.clone(), child: key.child, pos: world,
                                vel: g.mtx.transform_vector3(p.vel), dir: g.mtx.transform_vector3(p.pos - p.prev),
                                size: size(p, &res), colors: p.p.colors, rotate: Vec3::new(p.p.f[ROT], p.p.f[ROT + 1], p.p.f[ROT + 2]) + p.rot_offset });
                        },
                        7 => if let Some(f) = &c.field {
                            // ponytail: gravity and speed fields only; magnet/newton/vortex/spin/random/tail are unused by the flight effects.
                            let v = match c.kind {
                                0 => euler([f.params.first().copied().unwrap_or(0.0), f.params.get(1).copied().unwrap_or(0.0), f.params.get(2).copied().unwrap_or(0.0)]) * Vec3::Y * f.power,
                                1 => p.vel * (f.power - 1.0),
                                _ => continue,
                            };
                            let v = match (c.kind, f.space) {
                                (1, _) | (_, 2) => v,
                                (_, 1) => to_group.transform_vector3(v),
                                _ => to_group.transform_vector3(emitter_rot * v),
                            };
                            if f.target == 1 { add_pos += v } else { add_vel += v }
                        },
                        _ => {}
                    }
                }
                p.vel += add_vel;
                p.prev = p.pos;
                p.pos += (add_pos + p.vel) * p.momentum;
                p.tick += 1;
                true
            });
        }
    }
    for e in fx.emitters.iter_mut().filter(|e| e.active) {
        if e.wait > 0 { e.wait -= 1; continue; }
        let n = emission_count(e);
        if n > 0 {
            let space = e.groups[0].mtx.inverse() * e.global;
            let res = e.res.clone();
            emit(&res, &mut e.rng, n, space, &mut e.groups[0], None);
        }
        e.tick += 1;
    }
    for s in spawns { spawn_child(fx, s); }
}

/// Drawn half-size of a particle (size times scale, with the "same size/scale" flags).
fn size(p: &Particle, res: &EmitterRes) -> Vec2 {
    let f = &p.p.f;
    let (sx, sy) = (f[SIZE], f[SIZE + 1]);
    let (cx, cy) = (f[SCALE], f[SCALE + 1]);
    let y = match res.draw.flags & 0x6000 {
        0x4000 => sy * cx,
        0x2000 => sx * cy,
        0x6000 => sx * cx,
        _ => sy * cy,
    };
    Vec2::new(sx * cx, y)
}

fn spawn_child(fx: &mut Effect, s: Spawn) {
    let parent = &fx.emitters[s.emitter];
    let (scale, rot, _) = parent.global.to_scale_rotation_translation();
    let own = euler(s.res.rotate);
    let mut rotation = rot * own;
    if s.child.flag & 1 != 0 && s.child.speed != 0 && s.dir.length_squared() > 0.0 {
        let d = s.dir.normalize() * s.child.speed.signum() as f32;
        rotation = Quat::from_rotation_arc(Vec3::Y, d) * own;
    }
    let global = Affine3A::from_scale_rotation_translation(scale * Vec3::from(s.res.scale), rotation, s.pos + rot * Vec3::from(s.res.translate));
    if s.child.kind != 0 {
        let local = parent.child_space().inverse() * global;
        let e = Emitter::new(s.res.clone(), &s.name, Some(s.emitter), local, global, &mut fx.rng);
        fx.emitters.push(e);
        return;
    }
    // One-shot burst into a group of the parent emitter.
    let (pc, cc) = (parent.res.common, s.res.common);
    let (pt, ct) = (parent.res.inherit_child_emit_t as i32, s.res.inherit_ptcl_t as i32);
    let t = if pt == 0 || ct == 0 { 0 } else if pt == 100 { ct } else if ct == 100 { pt } else { pt * ct / 100 };
    let inherit = (pc & 0x80 != 0 && cc & 0x20 != 0, pc & 0x100 != 0 && cc & 0x40 != 0, t as f32 / 100.0);
    let seed = if s.res.seed != 0 { s.res.seed } else { fx.rng.next() };
    let parent = &mut fx.emitters[s.emitter];
    let gi = match parent.groups.iter().position(|g| g.name == s.name && g.inherit == inherit) {
        Some(i) => i,
        None => {
            let mtx = restructure(parent.global, inherit.0, inherit.1, inherit.2);
            parent.groups.push(Group { res: s.res.clone(), name: s.name.clone(), inherit, mtx, particles: vec![], entity: None });
            parent.groups.len() - 1
        }
    };
    let mut temp = Emitter { res: s.res.clone(), parent: None, local: global, global, tick: 0, wait: 0, interval_wait: 0, count: 0.0, first: true, rng: Rng(seed), seed: seed as u16, active: true, groups: vec![] };
    let n = emission_count(&mut temp);
    let group = &mut parent.groups[gi];
    let space = group.mtx.inverse() * global;
    let reference = (s.child.speed != 0 || s.child.scale != 0 || s.child.alpha != 0 || s.child.color != 0 || s.child.flag & 2 != 0).then_some(&s);
    emit(&s.res, &mut temp.rng, n, space, group, reference);
}

fn step(mut commands: Commands, lib: Res<ParticleLib>, mut effects: Query<(Entity, &mut Effect)>) {
    for (entity, mut fx) in &mut effects {
        step_effect(&mut fx, &lib);
        if fx.finished() { commands.entity(entity).despawn(); }
    }
}

// ---------------------------------------------------------------- drawing

const ATTRIBUTE_C1: MeshVertexAttribute = MeshVertexAttribute::new("ParticleColor1", 0x5150_0001, VertexFormat::Float32x4);
const ATTRIBUTE_C2: MeshVertexAttribute = MeshVertexAttribute::new("ParticleColor2", 0x5150_0002, VertexFormat::Float32x4);
const ATTRIBUTE_C3: MeshVertexAttribute = MeshVertexAttribute::new("ParticleColor3", 0x5150_0003, VertexFormat::Float32x4);

#[derive(Clone, Default, ShaderType, Debug)]
pub struct DrawUniform {
    stages: [UVec4; 4],
    info: UVec4,
    inputs: UVec4,
    wrap: UVec4,
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct ParticleMaterial {
    #[uniform(0)]
    draw: DrawUniform,
    #[texture(1)]
    #[sampler(2)]
    t1: Option<Handle<Image>>,
    #[texture(3)]
    #[sampler(4)]
    t2: Option<Handle<Image>>,
    #[storage(5, read_only)]
    globals: Handle<ShaderBuffer>,
    alpha_mode: AlphaMode,
}

impl Material for ParticleMaterial {
    fn vertex_shader() -> ShaderRef { "embedded://sport_island/particle.wesl".into() }
    fn fragment_shader() -> ShaderRef { "embedded://sport_island/particle.wesl".into() }
    fn alpha_mode(&self) -> AlphaMode { self.alpha_mode }
    fn specialize(_: &MaterialPipeline, descriptor: &mut RenderPipelineDescriptor, layout: &MeshVertexBufferLayoutRef, _: MaterialPipelineKey<Self>) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.vertex.buffers = vec![layout.0.get_layout(&[
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            Mesh::ATTRIBUTE_UV_0.at_shader_location(1),
            Mesh::ATTRIBUTE_UV_1.at_shader_location(2),
            Mesh::ATTRIBUTE_COLOR.at_shader_location(3),
            ATTRIBUTE_C1.at_shader_location(4),
            ATTRIBUTE_C2.at_shader_location(5),
            ATTRIBUTE_C3.at_shader_location(6),
        ])?];
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

fn material(res: &EmitterRes, assets: &AssetServer, globals: Handle<ShaderBuffer>) -> ParticleMaterial {
    let d = &res.draw;
    let pack = |abcd: [u8; 4], op: [u8; 5]| abcd[0] as u32 | (abcd[1] as u32) << 4 | (abcd[2] as u32) << 8 | (abcd[3] as u32) << 12
        | (op[0] as u32 & 3) << 16 | (op[1] as u32 & 3) << 18 | (op[2] as u32 & 3) << 20 | (op[3] as u32 & 1) << 22 | (op[4] as u32 & 3) << 23;
    let mut draw = DrawUniform::default();
    for i in 0..4 {
        let layer = d.tev_tex[i] as u32;
        let tex = if layer < 2 && d.flags & (0x10 << layer) != 0 { layer } else { 255 };
        draw.stages[i] = UVec4::new(pack(d.tev_color[i], d.color_op[i]), pack(d.tev_alpha[i], d.alpha_op[i]), tex, d.kcsel[i] as u32 | (d.kasel[i] as u32) << 8);
    }
    let bytes = |b: &[u8]| b.iter().enumerate().fold(0u32, |acc, (i, v)| acc | (*v as u32) << (8 * i));
    draw.inputs = UVec4::new(bytes(&d.color_in[1..4]), bytes(&d.color_in[4..8]), bytes(&d.alpha_in[1..4]), bytes(&d.alpha_in[4..8]));
    let w = res.ptcl.wrap as u32;
    draw.wrap = UVec4::new(w & 3, (w >> 2) & 3, (w >> 4) & 3, (w >> 6) & 3);
    // GX blend: type 1 blends; destination ONE adds. Bevy's Add is premultiplied,
    // so the shader applies the source factor itself and outputs zero alpha.
    let [ty, src, dst, _] = d.blend;
    let additive = ty == 1 && dst == 1;
    let mut flags = if d.flags & 0x1000 != 0 { 1 } else { 0 };
    if additive { flags |= 4 | if src == 4 { 2 } else { 0 }; }
    draw.info = UVec4::new(d.tevs.clamp(1, 4) as u32, flags, 0, 0);
    let tex = |name: &String| (!name.is_empty()).then(|| assets.load(format!("effects/{}.png", name.replace('#', "~"))));
    ParticleMaterial { draw, t1: tex(&res.ptcl.textures[0]), t2: tex(&res.ptcl.textures[1]), globals, alpha_mode: if additive { AlphaMode::Add } else { AlphaMode::Blend } }
}

#[derive(Resource, Default)]
struct Materials(HashMap<String, Handle<ParticleMaterial>>);

/// NW4R particle texture matrix: translate, rotate and scale about the centre, then mirror/reverse.
fn tex_uv(st: Vec2, p: &Params, layer: usize, res: &PtclRes) -> Vec2 {
    let f = &p.f;
    let mut v = st + Vec2::new(f[TEX_TRANS + 2 * layer], f[TEX_TRANS + 2 * layer + 1]);
    let rot = f[TEX_ROT + layer];
    if rot != 0.0 { v = Vec2::from_angle(rot).rotate(v - 0.5) + 0.5; }
    v = (v - 0.5) * Vec2::new(f[TEX_SCALE + 2 * layer], f[TEX_SCALE + 2 * layer + 1]) + 0.5;
    let wrap = res.wrap >> (layer * 4);
    let reverse = res.reverse >> (layer * 2);
    let axis = |mirror: bool, rev: bool, x: f32| {
        let (mut scale, mut trans) = (if mirror { 2.0 } else { 1.0 }, 0.0);
        if rev { scale = -scale; trans = if mirror { 2.0 } else { 1.0 }; }
        x * scale + trans
    };
    Vec2::new(axis(wrap & 3 == 2, reverse & 1 != 0, v.x), axis((wrap >> 2) & 3 == 2, reverse & 2 != 0, v.y))
}

/// Alpha flicker multiplier (0-255) from the draw setting's wave.
fn flick(d: &DrawRes, p: &Particle) -> Option<i32> {
    let [ty, cycle, random, amp] = d.flick.map(|v| v as i32);
    if ty == 0 || cycle == 0 { return None; }
    let cycle = (cycle + cycle * p.flick * random / 12700).clamp(1, 0xFFFF);
    let pos = (p.tick as i32 - 1).rem_euclid(cycle);
    let a = match ty {
        1 => if pos * 2 <= cycle { 128 - amp + amp * pos * 4 / cycle } else { 128 + amp * 3 - amp * pos * 4 / cycle },
        2 => 128 + amp - amp * pos * 2 / cycle,
        3 => 128 - amp + amp * pos * 2 / cycle,
        4 => if pos * 2 <= cycle { 128 + amp } else { 128 - amp },
        5 => 128 + (amp as f32 * (TAU * pos as f32 / cycle as f32).sin()) as i32,
        _ => return None,
    };
    Some(a.clamp(0, 255))
}

#[allow(clippy::too_many_arguments)]
fn draw(
    mut commands: Commands,
    cam: Single<&GlobalTransform, With<crate::world::MainCamera>>,
    fixed: Res<Time<Fixed>>,
    assets: Res<AssetServer>,
    globals: Res<crate::gx::GxGlobals>,
    mut cache: ResMut<Materials>,
    mut materials: ResMut<Assets<ParticleMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut effects: Query<(Entity, &mut Effect)>,
    mut targets: Query<(&Mesh3d, &mut Transform, &mut GlobalTransform, &mut Visibility), (Without<Effect>, Without<crate::world::MainCamera>)>,
) {
    let alpha = fixed.overstep_fraction();
    let cam_pos = cam.translation();
    let fwd = cam.forward().as_vec3();
    // NW4R keeps billboards upright against camera roll.
    let up = (Vec3::Y - fwd * fwd.dot(Vec3::Y)).try_normalize().unwrap_or(cam.up().as_vec3());
    let right = fwd.cross(up).normalize();
    for (owner, mut fx) in &mut effects {
        let tint = fx.tint;
        for e in &mut fx.emitters {
            let emitter_pos: Vec3 = e.global.translation.into();
            let emitter_y = e.global.transform_vector3(Vec3::Y).normalize_or_zero();
            for g in &mut e.groups {
                let res = g.res.clone();
                let anchor: Vec3 = g.mtx.translation.into();
                let (pm_scale, _, _) = g.mtx.to_scale_rotation_translation();
                let pivot = Vec2::new(res.draw.pivot[0] as f32, res.draw.pivot[1] as f32) / 100.0;
                let n = g.particles.len();
                let (mut pos, mut uv0, mut uv1) = (Vec::with_capacity(4 * n), Vec::with_capacity(4 * n), Vec::with_capacity(4 * n));
                let mut cols: [Vec<[f32; 4]>; 4] = std::array::from_fn(|_| Vec::with_capacity(4 * n));
                let mut indices = Vec::with_capacity(6 * n);
                for p in &g.particles {
                    let s = size(p, &res) * pm_scale.truncate();
                    if s.x < f32::EPSILON || s.y < f32::EPSILON { continue; }
                    let world = g.mtx.transform_point3(p.prev.lerp(p.pos, alpha));
                    let ptype = res.draw.ptype;
                    let (x_axis, y_axis) = if ptype[0] == 4 || (ptype[0] == 3 && ptype[1] == 2) {
                        // Directional: the quad's Y follows the chosen direction, facing the camera around it.
                        let dir = match ptype[2] {
                            0 => g.mtx.transform_vector3(p.vel),
                            1 => world - emitter_pos,
                            2 => emitter_y,
                            5 => Vec3::Y,
                            _ => g.mtx.transform_vector3(p.pos - p.prev),
                        }.try_normalize().unwrap_or(up);
                        let x = dir.cross(cam_pos - world).try_normalize().unwrap_or(right);
                        (x, dir)
                    } else if ptype[0] == 3 && ptype[1] == 1 {
                        // Y billboard: upright, turning about world Y.
                        (fwd.cross(Vec3::Y).try_normalize().unwrap_or(right), Vec3::Y)
                    } else {
                        let rz = p.p.f[ROT + 2] + p.rot_offset.z;
                        let (sr, cr) = (-rz).sin_cos();
                        (right * cr + up * sr, up * cr - right * sr)
                    };
                    let (x_axis, y_axis) = (x_axis * s.x, y_axis * s.y);
                    let center = world - anchor - x_axis * pivot.x - y_axis * pivot.y;
                    let base = pos.len() as u32;
                    for (cx, cy, st) in [(-1.0, 1.0, Vec2::new(0.0, 0.0)), (1.0, 1.0, Vec2::new(1.0, 0.0)), (1.0, -1.0, Vec2::new(1.0, 1.0)), (-1.0, -1.0, Vec2::new(0.0, 1.0))] {
                        pos.push((center + x_axis * cx + y_axis * cy).to_array());
                        uv0.push(tex_uv(st, &p.p, 0, &res.ptcl).to_array());
                        uv1.push(tex_uv(st, &p.p, 1, &res.ptcl).to_array());
                    }
                    let fl = flick(&res.draw, p);
                    for (layer, list) in cols.iter_mut().enumerate() {
                        let mut c: [u8; 4] = p.p.colors[layer * 4..layer * 4 + 4].try_into().unwrap();
                        if let Some(t) = tint.filter(|_| layer < 2) { c[..3].copy_from_slice(&t[layer]); }
                        let a = fl.map_or(c[3] as i32, |f| (c[3] as i32 * f + 128) >> 8);
                        let v = [c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0, a as f32 / 255.0];
                        list.extend([v; 4]);
                    }
                    indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
                }
                // Stripes (5, 6) are not drawn: no flight effect the player uses needs them.
                if indices.is_empty() || res.draw.ptype[0] >= 5 {
                    if let Some((_, _, _, mut vis)) = g.entity.and_then(|e| targets.get_mut(e).ok()) { *vis = Visibility::Hidden; }
                    continue;
                }
                let [c0, c1, c2, c3] = cols;
                let fill = |mesh: &mut Mesh| {
                    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
                    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv0);
                    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, uv1);
                    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, c0);
                    mesh.insert_attribute(ATTRIBUTE_C1, c1);
                    mesh.insert_attribute(ATTRIBUTE_C2, c2);
                    mesh.insert_attribute(ATTRIBUTE_C3, c3);
                    mesh.insert_indices(Indices::U32(indices));
                };
                let transform = Transform::from_translation(anchor);
                match g.entity.and_then(|e| targets.get_mut(e).ok()) {
                    Some((handle, mut t, mut gt, mut vis)) => {
                        *vis = Visibility::Inherited;
                        *t = transform;
                        *gt = GlobalTransform::from(transform);
                        if let Some(mut mesh) = meshes.get_mut(&handle.0) { fill(&mut mesh); }
                    }
                    None => {
                        let material = cache.0.entry(g.name.clone()).or_insert_with(|| materials.add(material(&res, &assets, globals.0.clone()))).clone();
                        let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
                        fill(&mut mesh);
                        g.entity = Some(commands.spawn((Mesh3d(meshes.add(mesh)), MeshMaterial3d(material), transform, NoFrustumCulling, NotShadowCaster, ChildOf(owner))).id());
                    }
                }
            }
        }
    }
}

pub fn plugin(app: &mut App) {
    embedded_asset!(app, "particle.wesl");
    let lib: HashMap<String, EmitterRes> = crate::read_json("effects/pln.json");
    app.add_plugins(MaterialPlugin::<ParticleMaterial>::default())
        .insert_resource(ParticleLib(lib.into_iter().map(|(k, v)| (k, Arc::new(v))).collect()))
        .init_resource::<Materials>()
        .add_systems(FixedUpdate, step.run_if(crate::menu::playing))
        .add_systems(PostUpdate, draw.after(TransformSystems::Propagate));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn curve(proc: u8, loops: u8, frames: u16) -> Curve {
        Curve { kind: 16, ty: 3, enable: 1, proc, loops, seed: 0, frames, keys: vec![], ranges: vec![], random: vec![], names: vec![], field: None, baked: vec![] }
    }

    #[test]
    fn key_times_clamp_loop_and_fit() {
        assert_eq!(key_time(&curve(0, 0, 10), 25, 100).0, 9);
        assert_eq!(key_time(&curve(PROC_LOOP, 0, 11), 25, 100).0, 5);
        assert_eq!(key_time(&curve(PROC_LOOP | PROC_TURN, 0, 11), 15, 100).0, 5);
        let (k, f, _) = key_time(&curve(PROC_FIT, 1, 5), 50, 101);
        assert_eq!((k, f), (2, 2.0));
    }

    #[test]
    fn keys_interpolate_and_ranges_stay_in_bounds() {
        let mut c = curve(0, 0, 11);
        c.keys = vec![
            Key { frame: 0, mode: 0, interp: [0; 4], values: vec![1.0], index: 0, child: Child::default() },
            Key { frame: 10, mode: 1, interp: [0; 4], values: vec![], index: 0, child: Child::default() },
        ];
        c.ranges = vec![Range { v: vec![[3.0, 1.0 / 65536.0]], sign: false }];
        let mut v = 0.0;
        evaluate(&c, 10, 100, 7, |_, x| v = x);
        assert!((3.0..4.0).contains(&v));
        evaluate(&c, 0, 100, 7, |_, x| v = x);
        assert_eq!(v, 1.0);
    }

    #[test]
    fn mirrored_textures_cover_two_tiles() {
        let mut res = PtclRes { colors: [[255; 4]; 4], size: [1.0; 2], scale: [1.0; 2], rotate: [0.0; 3], tex_scale: [[1.0; 2]; 3], tex_rot: [0.0; 3],
            tex_trans: [[0.0; 2]; 3], wrap: 0b1010, reverse: 0, rot_offset_rnd: [0; 3], rot_offset: [0.0; 3], textures: Default::default() };
        let p = Params::new(&res);
        assert_eq!(tex_uv(Vec2::ONE, &p, 0, &res), Vec2::splat(2.0));
        res.reverse = 1;
        assert_eq!(tex_uv(Vec2::ZERO, &p, 0, &res).x, 2.0);
    }
}
