//! CHR0 (bone animation) decoding, sampled per frame into glTF animations.

use crate::disc::{be16, be32, bef, cstr};
use crate::gltf::Gltf;
use crate::mdl::{Model, group, quat};
use serde_json::json;
use std::collections::HashMap;

/// Keyframed or per-frame track for one scalar.
#[derive(Clone)]
pub enum Track {
    Const(f32),
    /// (frame, value, tangent), hermite-interpolated.
    Keys(Vec<(f32, f32, f32)>),
    /// Value for every frame.
    Frames(Vec<f32>),
}

impl Track {
    pub fn at(&self, f: f32) -> f32 {
        match self {
            Track::Const(v) => *v,
            Track::Frames(v) => {
                let i = (f.floor() as usize).min(v.len() - 1);
                let j = (i + 1).min(v.len() - 1);
                let t = f - i as f32;
                v[i] + (v[j] - v[i]) * t
            }
            Track::Keys(k) => {
                if f <= k[0].0 {
                    return k[0].1;
                }
                for w in k.windows(2) {
                    let ((f0, v0, t0), (f1, v1, t1)) = (w[0], w[1]);
                    if f <= f1 {
                        let span = f1 - f0;
                        if span <= 0.0 {
                            return v1;
                        }
                        let t = (f - f0) / span;
                        let (t2, t3) = (t * t, t * t * t);
                        return v0 * (2.0 * t3 - 3.0 * t2 + 1.0)
                            + v1 * (-2.0 * t3 + 3.0 * t2)
                            + t0 * span * (t3 - 2.0 * t2 + t)
                            + t1 * span * (t3 - t2);
                    }
                }
                k.last().unwrap().1
            }
        }
    }
}

/// Reads keyframe data in the interpolated/linear CHR0/SRT0/CLR0 encodings.
pub fn read_track(d: &[u8], o: usize, fmt: u32, rot: bool, frames: usize) -> Track {
    match (fmt, rot) {
        (1, _) => {
            let (n, step, base) = (be16(d, o) as usize, bef(d, o + 8), bef(d, o + 12));
            Track::Keys(
                (0..n)
                    .map(|i| {
                        let v = be32(d, o + 16 + 4 * i);
                        let tan = ((v & 0xFFF) as i32) << 20 >> 20;
                        ((v >> 24) as f32, base + step * ((v >> 12) & 0xFFF) as f32, tan as f32 / 32.0)
                    })
                    .collect(),
            )
        }
        (2, _) => {
            let (n, step, base) = (be16(d, o) as usize, bef(d, o + 8), bef(d, o + 12));
            Track::Keys(
                (0..n)
                    .map(|i| {
                        let e = o + 16 + 6 * i;
                        (be16(d, e) as f32 / 32.0, base + step * be16(d, e + 2) as f32, be16(d, e + 4) as i16 as f32 / 256.0)
                    })
                    .collect(),
            )
        }
        (3, _) => {
            let n = be16(d, o) as usize;
            Track::Keys((0..n).map(|i| (bef(d, o + 8 + 12 * i), bef(d, o + 12 + 12 * i), bef(d, o + 16 + 12 * i))).collect())
        }
        (4, true) | (5, true) => {
            let (step, base) = (bef(d, o), bef(d, o + 4));
            Track::Frames(
                (0..=frames)
                    .map(|i| base + step * if fmt == 4 { d[o + 8 + i] as f32 } else { be16(d, o + 8 + 2 * i) as f32 })
                    .collect(),
            )
        }
        (6, true) => Track::Frames((0..=frames).map(|i| bef(d, o + 4 * i)).collect()),
        _ => Track::Const(0.0),
    }
}

pub struct BoneAnim {
    pub bone: String,
    /// scale, rotation (degrees), translation; None = use model's bind value.
    pub s: [Option<Track>; 3],
    pub r: [Option<Track>; 3],
    pub t: [Option<Track>; 3],
}

pub struct Chr {
    pub name: String,
    pub frames: usize,
    pub looped: bool,
    pub bones: Vec<BoneAnim>,
}

pub fn chr(d: &[u8], o: usize, name: &str) -> Chr {
    let ver = be32(d, o + 8);
    let (data, fr, lp) = if ver >= 5 { (0x10, 0x20, 0x24) } else { (0x10, 0x1C, 0x20) };
    let frames = be16(d, o + fr) as usize;
    let looped = be32(d, o + lp) != 0;
    let mut bones = vec![];
    for (bname, e) in group(d, o + be32(d, o + data) as usize) {
        let code = be32(d, e + 4);
        let mut p = e + 8;
        let mut read = |has: bool, iso: bool, fixed_bit: u32, fmt: u32, rot: bool| -> [Option<Track>; 3] {
            if !has {
                return [None, None, None];
            }
            let mut out: [Option<Track>; 3] = [None, None, None];
            for axis in 0..if iso { 1 } else { 3 } {
                let tr = if code & (1 << (fixed_bit + axis)) != 0 {
                    Track::Const(bef(d, p))
                } else {
                    read_track(d, e + be32(d, p) as usize, fmt, rot, frames)
                };
                p += 4;
                out[axis as usize] = Some(tr);
            }
            if iso {
                out[1] = out[0].clone();
                out[2] = out[0].clone();
            }
            out
        };
        let s = read(code & (1 << 22) != 0, code & (1 << 4) != 0, 13, (code >> 25) & 3, false);
        let r = read(code & (1 << 23) != 0, code & (1 << 5) != 0, 16, (code >> 27) & 7, true);
        let t = read(code & (1 << 24) != 0, code & (1 << 6) != 0, 19, (code >> 30) & 3, false);
        // Identity / zero flags without tracks still override the bind pose.
        let (s, r, t) = if code & 2 != 0 {
            (one(), zero(), zero())
        } else {
            (if code & 8 != 0 && s[0].is_none() { one() } else { s }, if code & 4 != 0 && r[0].is_none() { zero() } else { r }, if code & 4 != 0 && t[0].is_none() { zero() } else { t })
        };
        bones.push(BoneAnim { bone: bname, s, r, t });
    }
    Chr { name: name.to_string(), frames, looped, bones }
}

fn one() -> [Option<Track>; 3] { std::array::from_fn(|_| Some(Track::Const(1.0))) }
fn zero() -> [Option<Track>; 3] { std::array::from_fn(|_| Some(Track::Const(0.0))) }

pub fn chr_to_gltf(g: &mut Gltf, a: &Chr, m: &Model, base_node: usize) {
    let by_name: HashMap<&str, usize> = m.bones.iter().enumerate().map(|(i, b)| (b.name.as_str(), i)).collect();
    let times: Vec<f32> = (0..=a.frames).map(|f| f as f32 / 60.0).collect();
    let mut samplers = vec![];
    let mut channels = vec![];
    let mut input = None;
    for ba in &a.bones {
        let Some(&bi) = by_name.get(ba.bone.as_str()) else { continue };
        let bone = &m.bones[bi];
        let animated = |t: &[Option<Track>; 3]| t.iter().any(|x| x.is_some());
        let eval = |t: &[Option<Track>; 3], bind: [f32; 3], f: f32| -> [f32; 3] { std::array::from_fn(|i| t[i].as_ref().map_or(bind[i], |tr| tr.at(f))) };
        let inp = *input.get_or_insert_with(|| g.floats(&times, "SCALAR", 1, true, None));
        for (path, tr, bind) in [("scale", &ba.s, bone.s), ("rotation", &ba.r, bone.r), ("translation", &ba.t, bone.t)] {
            if !animated(tr) {
                continue;
            }
            let mut out = vec![];
            for f in 0..=a.frames {
                let v = eval(tr, bind, f as f32);
                if path == "rotation" {
                    out.extend_from_slice(&quat(v));
                } else {
                    out.extend_from_slice(&v);
                }
            }
            let (ty, n) = if path == "rotation" { ("VEC4", 4) } else { ("VEC3", 3) };
            let acc = g.floats(&out, ty, n, false, None);
            samplers.push(json!({ "input": inp, "output": acc, "interpolation": "LINEAR" }));
            channels.push(json!({ "sampler": samplers.len() - 1, "target": { "node": base_node + bi, "path": path } }));
        }
    }
    if !channels.is_empty() {
        g.animations.push(json!({ "name": a.name, "samplers": samplers, "channels": channels, "extras": { "loop": a.looped } }));
    }
}

/// SRT0 texture animation: material -> per-frame [sx, sy, rot, tx, ty] for texture slot 0.
pub fn srt0(d: &[u8], o: usize) -> serde_json::Value {
    let ver = be32(d, o + 8);
    let (fr, lp) = if ver >= 5 { (0x20, 0x28) } else { (0x1C, 0x24) };
    let frames = be16(d, o + fr) as usize;
    let looped = be32(d, o + lp) != 0;
    let mut mats = serde_json::Map::new();
    for (mname, e) in group(d, o + be32(d, o + 0x10) as usize) {
        let flags = be32(d, e + 4);
        let mut slots = serde_json::Map::new();
        for (k, slot) in (0..8).filter(|i| flags & (1 << i) != 0).enumerate() {
            let mut p = e + be32(d, e + 12 + 4 * k) as usize;
            let code = be32(d, p);
            p += 4;
            // code: bit1 scale one, bit2 rot zero, bit3 trans zero, bit4 scale iso,
            // bits 5-9 fixed (sx, sy, rot, tx, ty); animated values point to I12 keys.
            let mut vals: Vec<Track> = vec![];
            let mut take = |i: u32| -> Track {
                let t = if code & (1 << (5 + i)) != 0 { Track::Const(bef(d, p)) } else { read_track(d, p + be32(d, p) as usize, 3, false, frames) };
                p += 4;
                t
            };
            if code & 2 != 0 {
                vals.extend([Track::Const(1.0), Track::Const(1.0)]);
            } else {
                let sx = take(0);
                let sy = if code & 0x10 != 0 { sx.clone() } else { take(1) };
                vals.extend([sx, sy]);
            }
            vals.push(if code & 4 != 0 { Track::Const(0.0) } else { take(2) });
            if code & 8 != 0 {
                vals.extend([Track::Const(0.0), Track::Const(0.0)]);
            } else {
                let tx = take(3);
                vals.extend([tx, take(4)]);
            }
            let seq: Vec<[f32; 5]> = (0..=frames).map(|f| std::array::from_fn(|i| vals[i].at(f as f32))).collect();
            slots.insert(slot.to_string(), json!(seq));
        }
        mats.insert(mname, serde_json::Value::Object(slots));
    }
    json!({ "name": cstr(d, o + be32(d, o + if ver >= 5 { 0x18 } else { 0x14 }) as usize), "loop": looped, "materials": mats })
}

/// PAT0 texture swaps: material -> slot -> texture name for every frame.
pub fn pat0(d: &[u8], o: usize) -> serde_json::Value {
    let frames = be16(d, o + 0x30) as usize;
    let names = o + be32(d, o + 0x14) as usize;
    let tex = |i: u16| cstr(d, names + be32(d, names + 4 * i as usize) as usize);
    let mut mats = serde_json::Map::new();
    for (mname, e) in group(d, o + be32(d, o + 0x10) as usize) {
        let flags = be32(d, e + 4);
        let mut slots = serde_json::Map::new();
        let mut p = e + 8;
        for slot in (0..8).filter(|s| (flags >> (4 * s)) & 1 != 0) {
            // Per slot: bit1 constant (texture index inline), else offset to (frame, texture) keys.
            let seq: Vec<String> = if (flags >> (4 * slot)) & 2 != 0 {
                vec![tex(be16(d, p)); frames + 1]
            } else {
                let k = e + be32(d, p) as usize;
                let keys: Vec<(f32, u16)> = (0..be16(d, k) as usize).map(|i| (bef(d, k + 8 + 8 * i), be16(d, k + 12 + 8 * i))).collect();
                (0..=frames).map(|f| tex(keys.iter().rev().find(|(kf, _)| *kf <= f as f32).or(keys.first()).map_or(0, |k| k.1))).collect()
            };
            p += 4;
            slots.insert(slot.to_string(), json!(seq));
        }
        mats.insert(mname, serde_json::Value::Object(slots));
    }
    json!({ "name": cstr(d, o + be32(d, o + 0x28) as usize), "loop": be32(d, o + 0x38) != 0, "materials": mats })
}

/// CLR0 colour animation: material -> target -> {mask, values per frame}. Targets
/// 0..3 are material/ambient colours, 4..6 TEV C0..C2, 7..10 TEV konst 0..3; the
/// mask keeps original bits (RGBA8, big-endian packed).
pub fn clr0(d: &[u8], o: usize) -> serde_json::Value {
    let frames = be16(d, o + 0x20) as usize;
    let mut mats = serde_json::Map::new();
    for (mname, e) in group(d, o + be32(d, o + 0x10) as usize) {
        let flags = be32(d, e + 4);
        let mut targets = serde_json::Map::new();
        let mut p = e + 8;
        for t in (0..11).filter(|t| (flags >> (2 * t)) & 1 != 0) {
            let values: Vec<u32> = if (flags >> (2 * t + 1)) & 1 != 0 {
                vec![be32(d, p + 4); frames + 1]
            } else {
                let v = p + 4 + be32(d, p + 4) as usize;
                (0..=frames).map(|f| be32(d, v + 4 * f)).collect()
            };
            targets.insert(t.to_string(), json!({ "mask": be32(d, p), "values": values }));
            p += 8;
        }
        mats.insert(mname, serde_json::Value::Object(targets));
    }
    json!({ "name": cstr(d, o + be32(d, o + 0x18) as usize), "loop": be32(d, o + 0x24) != 0, "materials": mats })
}
