//! BRRES (MDL0 + TEX0/PLT0 + CHR0) -> GLB conversion.

use crate::disc::{be16, be32, bef, cstr};
use crate::gltf::Gltf;
use crate::tex;
use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::collections::HashMap;

type M34 = [[f32; 4]; 3];

pub fn group(d: &[u8], o: usize) -> Vec<(String, usize)> {
    let n = be32(d, o + 4) as usize;
    (1..=n)
        .map(|i| {
            let e = o + 8 + i * 16;
            (cstr(d, o + be32(d, e + 8) as usize), o + be32(d, e + 12) as usize)
        })
        .collect()
}

/// Top-level BRRES folders, e.g. "3DModels(NW4R)" -> [(name, offset)].
pub fn brres(d: &[u8]) -> HashMap<String, Vec<(String, usize)>> {
    let root = be16(d, 0xC) as usize;
    group(d, root + 8).into_iter().map(|(n, o)| (n, group(d, o))).collect()
}

fn rel(d: &[u8], base: usize, at: usize) -> Option<usize> {
    let v = be32(d, base + at) as i32;
    (v != 0).then(|| (base as i64 + v as i64) as usize)
}

fn mul(a: &M34, b: &M34) -> M34 {
    let mut r = [[0.0; 4]; 3];
    for i in 0..3 {
        for j in 0..4 {
            r[i][j] = (0..3).map(|k| a[i][k] * b[k][j]).sum::<f32>() + if j == 3 { a[i][3] } else { 0.0 };
        }
    }
    r
}

fn xform(m: &M34, p: [f32; 3], w: f32) -> [f32; 3] {
    std::array::from_fn(|i| m[i][0] * p[0] + m[i][1] * p[1] + m[i][2] * p[2] + m[i][3] * w)
}

fn inverse(m: &M34) -> M34 {
    let [a, b, c] = [m[0], m[1], m[2]];
    let det = a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0]) + a[2] * (b[0] * c[1] - b[1] * c[0]);
    let id = 1.0 / det;
    let r = [
        [(b[1] * c[2] - b[2] * c[1]) * id, (a[2] * c[1] - a[1] * c[2]) * id, (a[1] * b[2] - a[2] * b[1]) * id],
        [(b[2] * c[0] - b[0] * c[2]) * id, (a[0] * c[2] - a[2] * c[0]) * id, (a[2] * b[0] - a[0] * b[2]) * id],
        [(b[0] * c[1] - b[1] * c[0]) * id, (a[1] * c[0] - a[0] * c[1]) * id, (a[0] * b[1] - a[1] * b[0]) * id],
    ];
    let t = [m[0][3], m[1][3], m[2][3]];
    std::array::from_fn(|i| {
        let tt = -(r[i][0] * t[0] + r[i][1] * t[1] + r[i][2] * t[2]);
        [r[i][0], r[i][1], r[i][2], tt]
    })
}

/// NW4R bone transform: T * Rz * Ry * Rx * S, rotation in degrees.
pub fn trs(s: [f32; 3], r: [f32; 3], t: [f32; 3]) -> M34 {
    let (sx, cx) = r[0].to_radians().sin_cos();
    let (sy, cy) = r[1].to_radians().sin_cos();
    let (sz, cz) = r[2].to_radians().sin_cos();
    let m = [
        [cy * cz, sx * sy * cz - cx * sz, cx * sy * cz + sx * sz],
        [cy * sz, sx * sy * sz + cx * cz, cx * sy * sz - sx * cz],
        [-sy, sx * cy, cx * cy],
    ];
    std::array::from_fn(|i| [m[i][0] * s[0], m[i][1] * s[1], m[i][2] * s[2], t[i]])
}

/// Euler XYZ (degrees, NW4R order) to quaternion [x, y, z, w].
pub fn quat(r: [f32; 3]) -> [f32; 4] {
    let (sx, cx) = (r[0].to_radians() / 2.0).sin_cos();
    let (sy, cy) = (r[1].to_radians() / 2.0).sin_cos();
    let (sz, cz) = (r[2].to_radians() / 2.0).sin_cos();
    [
        sx * cy * cz - cx * sy * sz,
        cx * sy * cz + sx * cy * sz,
        cx * cy * sz - sx * sy * cz,
        cx * cy * cz + sx * sy * sz,
    ]
}

/// Maya-style texture SRT matrix (2x3) as used by NW4R.
pub fn tex_mtx(s: [f32; 2], rot: f32, t: [f32; 2]) -> [[f32; 3]; 2] {
    let (sr, cr) = rot.to_radians().sin_cos();
    [
        [s[0] * cr, -s[0] * sr, s[0] * ((-0.5 * cr) - (0.5 * sr - 0.5) - t[0])],
        [s[1] * sr, s[1] * cr, s[1] * ((-0.5 * cr) + (0.5 * sr - 0.5) + t[1]) + 1.0],
    ]
}

pub struct Bone {
    pub name: String,
    pub parent: Option<usize>,
    pub s: [f32; 3],
    pub r: [f32; 3],
    pub t: [f32; 3],
    pub billboard: u32,
    pub world: M34,
}

struct Array {
    comps: usize,
    fmt: u32,
    div: u32,
    stride: usize,
    data: usize,
    count: usize,
}

impl Array {
    fn get(&self, d: &[u8], i: usize, n: usize) -> Vec<f32> {
        let o = self.data + i.min(self.count.saturating_sub(1)) * self.stride;
        let sc = 1.0 / (1u32 << self.div) as f32;
        (0..n)
            .map(|k| match self.fmt {
                0 => d[o + k] as f32 * sc,
                1 => d[o + k] as i8 as f32 * sc,
                2 => be16(d, o + 2 * k) as f32 * sc,
                3 => be16(d, o + 2 * k) as i16 as f32 * sc,
                _ => bef(d, o + 4 * k),
            })
            .collect()
    }
}

fn color(d: &[u8], a: &Array, i: usize) -> [f32; 4] {
    let o = a.data + i.min(a.count.saturating_sub(1)) * a.stride;
    let c = |v: u32, bits: u32| v as f32 / ((1 << bits) - 1) as f32;
    match a.fmt {
        0 => {
            let v = be16(d, o) as u32;
            [c(v >> 11, 5), c((v >> 5) & 63, 6), c(v & 31, 5), 1.0]
        }
        1 | 2 => [c(d[o] as u32, 8), c(d[o + 1] as u32, 8), c(d[o + 2] as u32, 8), 1.0],
        3 => {
            let v = be16(d, o) as u32;
            [c(v >> 12, 4), c((v >> 8) & 15, 4), c((v >> 4) & 15, 4), c(v & 15, 4)]
        }
        4 => {
            let v = (d[o] as u32) << 16 | (d[o + 1] as u32) << 8 | d[o + 2] as u32;
            [c(v >> 18, 6), c((v >> 12) & 63, 6), c((v >> 6) & 63, 6), c(v & 63, 6)]
        }
        _ => [c(d[o] as u32, 8), c(d[o + 1] as u32, 8), c(d[o + 2] as u32, 8), c(d[o + 3] as u32, 8)],
    }
}

fn arrays(d: &[u8], mdl: usize, sec: usize, is_color: bool) -> Vec<Array> {
    let Some(g) = rel(d, mdl, 0x10 + sec * 4) else { return vec![] };
    group(d, g)
        .into_iter()
        .map(|(_, o)| Array {
            comps: be32(d, o + 0x14) as usize,
            fmt: be32(d, o + 0x18),
            div: if is_color { 0 } else { d[o + 0x1C] as u32 },
            stride: if is_color { d[o + 0x1C] as usize } else { d[o + 0x1D] as usize },
            data: o + be32(d, o + 8) as usize,
            count: be16(d, o + 0x1E) as usize,
        })
        .collect()
}

pub struct TexRef {
    pub name: String,
    pub palette: String,
    pub wrap: [u32; 2],
    pub min_filter: u32,
}

pub struct Layer {
    pub tex: TexRef,
    pub map_mode: u8,
    pub srt: ([f32; 2], f32, [f32; 2]),
}

pub struct Material {
    pub name: String,
    pub xlu: bool,
    pub cull: u32,
    pub layers: Vec<Layer>,
    pub mat_color: [u8; 4],
    pub amb_color: [u8; 4],
    pub vtx_color: bool,
    pub lit: bool,
    pub alpha_ref: Option<u8>,
    pub blend: bool,
    pub tev_colors: Vec<[i16; 4]>,
    pub konst: Vec<[u8; 4]>,
}

fn material(d: &[u8], o: usize) -> Material {
    let name = cstr(d, o + be32(d, o + 8) as usize);
    let flags = be32(d, o + 0x10);
    let cull = be32(d, o + 0x18);
    let ntex = be32(d, o + 0x2C) as usize;
    let texs = o + be32(d, o + 0x30) as usize;
    let mut layers = vec![];
    for i in 0..ntex {
        let t = texs + i * 0x34;
        let nm = |at: usize| match be32(d, t + at) {
            0 => String::new(),
            off => cstr(d, t + off as usize),
        };
        let tex = TexRef { name: nm(0), palette: nm(4), wrap: [be32(d, t + 0x18), be32(d, t + 0x1C)], min_filter: be32(d, t + 0x20) };
        let id = be32(d, t + 0x10) as usize; // texmap slot
        let srt = o + 0x1B0 + id.min(7) * 20;
        let eff = o + 0x250 + id.min(7) * 0x34;
        layers.push(Layer {
            tex,
            map_mode: d[eff + 2],
            srt: ([bef(d, srt), bef(d, srt + 4)], bef(d, srt + 8), [bef(d, srt + 12), bef(d, srt + 16)]),
        });
    }
    let ch = o + 0x3F0;
    let ctrl = be32(d, ch + 12);
    let rgba = |at: usize| [d[at], d[at + 1], d[at + 2], d[at + 3]];
    let mut m = Material {
        name,
        xlu: flags & 0x8000_0000 != 0,
        cull,
        layers,
        mat_color: rgba(ch + 4),
        amb_color: rgba(ch + 8),
        vtx_color: ctrl & 1 != 0,
        lit: ctrl & 2 != 0,
        alpha_ref: None,
        blend: false,
        tev_colors: vec![[0; 4]; 4],
        konst: vec![[255; 4]; 4],
    };
    // Material display list: BP registers for alpha test, blending and TEV colours.
    let dl = o + be32(d, o + 0x3C) as usize;
    let mut p = dl;
    let end = (dl + 0x180).min(o + be32(d, o) as usize);
    let mut pending_ra = [[0i16; 2]; 4];
    while p + 5 <= end {
        match d[p] {
            0x61 => {
                let reg = d[p + 1];
                let v = (d[p + 2] as u32) << 16 | (d[p + 3] as u32) << 8 | d[p + 4] as u32;
                match reg {
                    0xF3 => {
                        let (r0, c0) = ((v & 0xFF) as u8, (v >> 16) & 7);
                        // GX_GREATER=4, GX_GEQUAL=6 with a non-zero reference => cutout.
                        if (c0 == 4 || c0 == 6) && r0 > 0 {
                            m.alpha_ref = Some(r0);
                        }
                    }
                    0x41 => m.blend = v & 1 != 0,
                    0xE0..=0xE7 => {
                        let idx = ((reg - 0xE0) / 2) as usize;
                        let lo = (v & 0x7FF) as i16;
                        let hi = ((v >> 12) & 0x7FF) as i16;
                        let sx = |x: i16| if x & 0x400 != 0 { x - 0x800 } else { x };
                        let konst = v & 0x80_0000 != 0;
                        if reg % 2 == 0 {
                            pending_ra[idx] = [sx(lo), sx(hi)]; // R, A
                            if konst {
                                m.konst[idx][0] = lo as u8;
                                m.konst[idx][3] = hi as u8;
                            }
                        } else if konst {
                            m.konst[idx][2] = lo as u8;
                            m.konst[idx][1] = hi as u8;
                        } else {
                            m.tev_colors[idx] = [pending_ra[idx][0], sx(hi), sx(lo), pending_ra[idx][1]];
                        }
                    }
                    _ => {}
                }
                p += 5;
            }
            0x10 => {
                let n = be16(d, p + 1) as usize + 1;
                p += 5 + 4 * n;
            }
            0x08 => p += 6,
            _ => p += 1,
        }
    }
    m
}

struct Vtx {
    pos: [f32; 3],
    nrm: [f32; 3],
    clr: [f32; 4],
    uv: [[f32; 2]; 2],
    mtx: u16,
}

struct Shape {
    single: Option<u16>,
    verts: Vec<Vtx>,
    tris: Vec<u32>,
    has: [bool; 4], // nrm, clr, uv0, uv1
}

fn shape(d: &[u8], o: usize, arr: &[Vec<Array>; 4]) -> Result<Shape> {
    let mtx = be32(d, o + 8) as i32;
    let vcd = be32(d, o + 0xC) as u64 | (be32(d, o + 0x10) as u64) << 17;
    let prim_size = be32(d, o + 0x24) as usize;
    let prim = o + 0x24 + be32(d, o + 0x2C) as usize;
    let idx = |at: usize| be16(d, o + at) as i16;
    let (pos_a, nrm_a, clr_a) = (idx(0x48), idx(0x4A), idx(0x4C));
    let uv_a: Vec<i16> = (0..8).map(|i| idx(0x50 + i * 2)).collect();
    fn get(v: &[Array], i: i16) -> Option<&Array> { (i >= 0).then(|| v.get(i as usize)).flatten() }
    let (pa, na, ca) = (get(&arr[0], pos_a).context("shape without positions")?, get(&arr[1], nrm_a), get(&arr[2], clr_a));
    let ua: Vec<Option<&Array>> = uv_a.iter().map(|&i| get(&arr[3], i)).collect();
    // Attribute order in the vertex stream, with index size: 0 none, 1 direct, 2 u8, 3 u16.
    let kind = |bit: u32| ((vcd >> bit) & 3) as u8;
    let mut attrs: Vec<(u8, u8)> = vec![]; // (attr id, kind)
    if vcd & 1 != 0 {
        attrs.push((0, 1));
    }
    for t in 0..8 {
        if vcd >> (1 + t) & 1 != 0 {
            attrs.push((20 + t as u8, 1));
        }
    }
    attrs.push((1, kind(9))); // pos
    attrs.push((2, kind(11))); // nrm
    attrs.push((3, kind(13))); // clr0
    attrs.push((4, kind(15))); // clr1
    for t in 0..8 {
        attrs.push((10 + t, kind(17 + 2 * t as u32)));
    }
    let has = [kind(11) != 0 && na.is_some(), kind(13) != 0 && ca.is_some(), kind(17) != 0, kind(19) != 0];
    let nbt = na.is_some_and(|a| a.comps == 2);
    let mut slots = [0u16; 10];
    let mut verts = vec![];
    let mut tris = vec![];
    let mut dedup: HashMap<Vec<u32>, u32> = HashMap::new();
    let mut p = prim;
    let end = prim + prim_size;
    while p < end {
        let op = d[p];
        p += 1;
        match op {
            0x00 => {}
            0x08 => p += 5,
            0x10 => p += 4 + 4 * (be16(d, p) as usize + 1),
            0x20 => {
                let (node, addr) = (be16(d, p), be16(d, p + 2) & 0xFFF);
                if (addr / 12) < 10 {
                    slots[(addr / 12) as usize] = node;
                }
                p += 4;
            }
            0x28 | 0x30 | 0x38 => p += 4,
            0x61 => p += 4,
            0x80..=0xBF => {
                let n = be16(d, p) as usize;
                p += 2;
                let mut ids = Vec::with_capacity(n);
                for _ in 0..n {
                    let mut key = Vec::with_capacity(attrs.len());
                    for &(_, k) in &attrs {
                        let v = match k {
                            0 => 0,
                            1 | 2 => {
                                p += 1;
                                d[p - 1] as u32
                            }
                            _ => {
                                p += 2;
                                be16(d, p - 2) as u32
                            }
                        };
                        key.push(v);
                    }
                    let mut full = key.clone();
                    let pnmtx = attrs.iter().position(|a| a.0 == 0).map(|i| key[i] / 3).unwrap_or(0);
                    full.push(slots[pnmtx.min(9) as usize] as u32);
                    let id = *dedup.entry(full).or_insert_with(|| {
                        let val = |a: u8| attrs.iter().position(|x| x.0 == a).map(|i| key[i] as usize);
                        let pos = pa.get(d, val(1).unwrap_or(0), 3);
                        let nrm = match (na, val(2)) {
                            (Some(a), Some(i)) if kind(11) != 0 => a.get(d, if nbt { i * 3 } else { i }, 3),
                            _ => vec![0.0, 1.0, 0.0],
                        };
                        let clr = match (ca, val(3)) {
                            (Some(a), Some(i)) if kind(13) != 0 => color(d, a, i),
                            _ => [1.0; 4],
                        };
                        let uv = std::array::from_fn(|t| match (ua[t], val(10 + t as u8)) {
                            (Some(a), Some(i)) if kind(17 + 2 * t as u32) != 0 => {
                                let v = a.get(d, i, if a.comps == 0 { 1 } else { 2 });
                                [v[0], *v.get(1).unwrap_or(&0.0)]
                            }
                            _ => [0.0, 0.0],
                        });
                        verts.push(Vtx { pos: [pos[0], pos[1], pos[2]], nrm: [nrm[0], nrm[1], nrm[2]], clr, uv, mtx: slots[pnmtx.min(9) as usize] });
                        (verts.len() - 1) as u32
                    });
                    ids.push(id);
                }
                // GX is clockwise-front; emit counter-clockwise triangles.
                let mut tri = |a: u32, b: u32, c: u32| tris.extend_from_slice(&[a, c, b]);
                match op & 0xF8 {
                    0x80 => {
                        for q in ids.chunks_exact(4) {
                            tri(q[0], q[1], q[2]);
                            tri(q[0], q[2], q[3]);
                        }
                    }
                    0x90 => ids.chunks_exact(3).for_each(|t| tri(t[0], t[1], t[2])),
                    0x98 => {
                        for i in 2..n {
                            if i % 2 == 0 {
                                tri(ids[i - 2], ids[i - 1], ids[i]);
                            } else {
                                tri(ids[i - 1], ids[i - 2], ids[i]);
                            }
                        }
                    }
                    0xA0 => (2..n).for_each(|i| tri(ids[0], ids[i - 1], ids[i])),
                    _ => {}
                }
            }
            _ => break,
        }
    }
    Ok(Shape { single: (mtx >= 0).then_some(mtx as u16), verts, tris, has })
}

/// Decodes every TEX0 in the archive into an image, keyed by name.
pub fn textures(d: &[u8], folders: &HashMap<String, Vec<(String, usize)>>) -> HashMap<String, tex::Image> {
    let mut pals = HashMap::new();
    for (name, o) in folders.get("Palettes(NW4R)").into_iter().flatten() {
        let (fmt, n, data) = (be32(d, o + 0x18), be16(d, o + 0x1C) as usize, o + be32(d, o + 0x10) as usize);
        pals.insert(name.clone(), (0..n).map(|i| tex::palette_color(fmt, be16(d, data + i * 2))).collect::<Vec<_>>());
    }
    let mut out = HashMap::new();
    for (name, o) in folders.get("Textures(NW4R)").into_iter().flatten() {
        let (w, h, fmt) = (be16(d, o + 0x1C) as usize, be16(d, o + 0x1E) as usize, be32(d, o + 0x20));
        let data = o + be32(d, o + 0x10) as usize;
        let pal = pals.get(name).or_else(|| pals.values().next());
        match tex::decode(&d[data..], fmt, w, h, pal.map(|v| v.as_slice())) {
            Ok(img) => {
                out.insert(name.clone(), img);
            }
            Err(e) => eprintln!("  texture {name}: {e}"),
        }
    }
    out
}

pub struct Model {
    pub name: String,
    pub bones: Vec<Bone>,
    pub materials: Vec<Material>,
    /// (material, shape, opaque=false/xlu=true)
    draws: Vec<(usize, Shape, bool)>,
    mtx_bone: Vec<i32>,
    envelopes: HashMap<u16, Vec<(u16, f32)>>,
}

pub fn model(d: &[u8], mdl: usize, name: &str) -> Result<Model> {
    let bones_g = rel(d, mdl, 0x14).context("no bones")?;
    let mut bones = vec![];
    let mut offs = vec![];
    for (bname, o) in group(d, bones_g) {
        let v3 = |at: usize| [bef(d, o + at), bef(d, o + at + 4), bef(d, o + at + 8)];
        offs.push(o);
        bones.push((be32(d, o + 0xC) as usize, be32(d, o + 0x10), Bone {
            name: bname,
            parent: None,
            s: v3(0x20),
            r: v3(0x2C),
            t: v3(0x38),
            billboard: be32(d, o + 0x18),
            world: [[0.0; 4]; 3],
        }, rel(d, o, 0x5C)));
    }
    let parents: Vec<Option<usize>> = bones.iter().map(|b| b.3.and_then(|p| offs.iter().position(|&x| x == p)).map(|i| bones[i].0)).collect();
    let mut sorted: Vec<_> = bones.into_iter().zip(parents).collect();
    sorted.sort_by_key(|((i, ..), _)| *i);
    let mut mtx_bone = vec![];
    let mut bones: Vec<Bone> = vec![];
    for ((idx, mtx, mut b, _), parent) in sorted {
        b.parent = parent;
        let local = trs(b.s, b.r, b.t);
        b.world = match parent {
            Some(p) if p < bones.len() => mul(&bones[p].world, &local),
            _ => local,
        };
        if mtx as usize >= mtx_bone.len() {
            mtx_bone.resize(mtx as usize + 1, -1);
        }
        mtx_bone[mtx as usize] = idx as i32;
        bones.push(b);
    }
    let arr = [arrays(d, mdl, 2, false), arrays(d, mdl, 3, false), arrays(d, mdl, 4, true), arrays(d, mdl, 5, false)];
    let mats: Vec<_> = rel(d, mdl, 0x30).map(|g| group(d, g)).unwrap_or_default();
    let shps: Vec<_> = rel(d, mdl, 0x38).map(|g| group(d, g)).unwrap_or_default();
    let materials: Vec<Material> = mats.iter().map(|(_, o)| material(d, *o)).collect();
    let mut draws = vec![];
    let mut envelopes = HashMap::new();
    for (dname, o) in group(d, rel(d, mdl, 0x10).context("no definitions")?) {
        let mut p = o;
        loop {
            match d[p] {
                0x00 => p += 1,
                0x02 => p += 5,
                0x03 => {
                    let (mtx, n) = (be16(d, p + 1), d[p + 3] as usize);
                    envelopes.insert(mtx, (0..n).map(|i| (be16(d, p + 4 + i * 6), bef(d, p + 6 + i * 6))).collect::<Vec<_>>());
                    p += 4 + 6 * n;
                }
                0x04 => {
                    let (m, s) = (be16(d, p + 1) as usize, be16(d, p + 3) as usize);
                    if let Some((_, so)) = shps.get(s) {
                        match shape(d, *so, &arr) {
                            Ok(sh) => { draws.push((m, sh, dname == "DrawXlu")) }
                            Err(e) => eprintln!("  {name}: shape {s}: {e}"),
                        }
                    }
                    p += 8;
                }
                0x05 => p += 5,
                _ => break,
            }
        }
    }
    Ok(Model { name: name.to_string(), bones, materials, draws, mtx_bone, envelopes })
}

pub struct Converted {
    pub glb: Vec<u8>,
    /// Per-material info the game needs (texture animations, flags).
    pub info: Value,
}

/// Builds one GLB from several models of the same BRRES (each becomes a root node).
pub fn to_glb(models: &[Model], images: &HashMap<String, tex::Image>, anims: &[crate::anim::Chr]) -> Converted {
    let mut g = Gltf::default();
    let mut tex_index: HashMap<(String, [u32; 2]), usize> = HashMap::new();
    let mut img_index: HashMap<String, usize> = HashMap::new();
    let mut info = serde_json::Map::new();
    for m in models {
        let base_node = g.nodes.len();
        // Bones become nodes.
        for (i, b) in m.bones.iter().enumerate() {
            let q = quat(b.r);
            let mut n = json!({ "name": b.name, "translation": b.t, "rotation": q, "scale": b.s });
            if b.billboard != 0 {
                n["extras"] = json!({ "billboard": b.billboard });
            }
            let children: Vec<usize> = m.bones.iter().enumerate().filter(|(_, c)| c.parent == Some(i)).map(|(j, _)| base_node + j).collect();
            if !children.is_empty() {
                n["children"] = json!(children);
            }
            g.nodes.push(n);
        }
        let model_root = g.nodes.len();
        let roots: Vec<usize> = m.bones.iter().enumerate().filter(|(_, b)| b.parent.is_none()).map(|(i, _)| base_node + i).collect();
        g.nodes.push(json!({ "name": m.name, "children": roots }));
        g.scenes.push((m.name.clone(), vec![model_root]));
        // Materials.
        let mut mat_ids = vec![];
        for mat in &m.materials {
            let base_layer = mat.layers.iter().position(|l| l.map_mode == 0);
            let mut pbr = json!({ "metallicFactor": 0.0, "roughnessFactor": 1.0 });
            let mut factor = [1.0f32; 4];
            if !mat.vtx_color {
                factor = mat.mat_color.map(|c| c as f32 / 255.0);
            }
            if let Some(li) = base_layer {
                let l = &mat.layers[li];
                let key = (l.tex.name.clone(), l.tex.wrap);
                let ti = if let Some(&t) = tex_index.get(&key) {
                    Some(t)
                } else if let Some(img) = images.get(&l.tex.name) {
                    let ii = *img_index.entry(l.tex.name.clone()).or_insert_with(|| g.image_png(&l.tex.name, &img.png().unwrap()));
                    let wrap = |w: u32| match w {
                        0 => 33071,
                        2 => 33648,
                        _ => 10497,
                    };
                    g.samplers.push(json!({ "wrapS": wrap(l.tex.wrap[0]), "wrapT": wrap(l.tex.wrap[1]), "magFilter": 9729, "minFilter": 9987 }));
                    g.textures.push(json!({ "source": ii, "sampler": g.samplers.len() - 1 }));
                    tex_index.insert(key, g.textures.len() - 1);
                    Some(g.textures.len() - 1)
                } else {
                    None
                };
                if let Some(ti) = ti {
                    pbr["baseColorTexture"] = json!({ "index": ti });
                }
            }
            pbr["baseColorFactor"] = json!(factor);
            let has_alpha = base_layer.and_then(|li| images.get(&mat.layers[li].tex.name)).is_some_and(|i| i.has_alpha());
            let mut mj = json!({ "name": mat.name, "pbrMetallicRoughness": pbr, "doubleSided": mat.cull == 0 });
            if (mat.xlu || mat.blend) && (has_alpha || factor[3] < 1.0) {
                mj["alphaMode"] = "BLEND".into();
            } else if let Some(r) = mat.alpha_ref.filter(|_| has_alpha) {
                mj["alphaMode"] = "MASK".into();
                mj["alphaCutoff"] = json!(r as f32 / 255.0);
            } else if has_alpha && (mat.xlu || mat.alpha_ref.is_some()) {
                mj["alphaMode"] = "MASK".into();
            }
            if !mat.lit {
                mj["extensions"] = json!({ "KHR_materials_unlit": {} });
            }
            mj["extras"] = json!({
                "lit": mat.lit, "xlu": mat.xlu, "amb": mat.amb_color, "mat": mat.mat_color,
                "layers": mat.layers.iter().map(|l| json!({"tex": l.tex.name, "map": l.map_mode})).collect::<Vec<_>>(),
                "konst": mat.konst, "tev": mat.tev_colors,
            });
            g.materials.push(mj);
            mat_ids.push(g.materials.len() - 1);
            let srt = base_layer.map(|li| mat.layers[li].srt).unwrap_or(([1.0, 1.0], 0.0, [0.0, 0.0]));
            info.insert(mat.name.clone(), json!({ "srt": [srt.0[0], srt.0[1], srt.1, srt.2[0], srt.2[1]] }));
        }
        // Meshes: one glTF mesh per draw call.
        let mut skin_joints: Option<usize> = None;
        for (mi, sh, _xlu) in &m.draws {
            if sh.tris.is_empty() {
                continue;
            }
            let mat = &m.materials[*mi];
            let base_layer = mat.layers.iter().position(|l| l.map_mode == 0);
            // Bake the static texture SRT of the base layer into its UVs.
            let tm = base_layer.map(|li| {
                let (s, r, t) = mat.layers[li].srt;
                tex_mtx(s, r, t)
            });
            let uv_set = base_layer.unwrap_or(0).min(1);
            let bone_of = |mtx: u16| m.mtx_bone.get(mtx as usize).copied().unwrap_or(-1);
            let single_bone = sh.single.map(bone_of).filter(|&b| b >= 0).map(|b| b as usize);
            let mut pos = vec![];
            let mut nrm = vec![];
            let mut clr = vec![];
            let mut uv = vec![];
            let mut joints = vec![];
            let mut weights = vec![];
            for v in &sh.verts {
                let (p, n) = if single_bone.is_some() || sh.single.is_some() {
                    (v.pos, v.nrm)
                } else {
                    let b = bone_of(v.mtx);
                    if b >= 0 {
                        let w = &m.bones[b as usize].world;
                        (xform(w, v.pos, 1.0), xform(w, v.nrm, 0.0))
                    } else {
                        (v.pos, v.nrm)
                    }
                };
                pos.extend_from_slice(&p);
                let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-6);
                nrm.extend_from_slice(&[n[0] / l, n[1] / l, n[2] / l]);
                clr.extend_from_slice(&v.clr);
                let [s, t] = v.uv[uv_set];
                let (s, t) = match tm {
                    Some(m) => (m[0][0] * s + m[0][1] * t + m[0][2], m[1][0] * s + m[1][1] * t + m[1][2]),
                    None => (s, t),
                };
                uv.extend_from_slice(&[s, t]);
                if sh.single.is_none() {
                    let mut jw: Vec<(u16, f32)> = match m.envelopes.get(&v.mtx) {
                        Some(env) => env.iter().map(|&(mtx, w)| (bone_of(mtx).max(0) as u16, w)).collect(),
                        None => vec![(bone_of(v.mtx).max(0) as u16, 1.0)],
                    };
                    jw.sort_by(|a, b| b.1.total_cmp(&a.1));
                    jw.resize(4, (0, 0.0));
                    let sum: f32 = jw.iter().map(|x| x.1).sum::<f32>().max(1e-6);
                    joints.extend(jw.iter().map(|x| x.0));
                    weights.extend(jw.iter().map(|x| x.1 / sum));
                }
            }
            let mut attrs = json!({
                "POSITION": g.floats(&pos, "VEC3", 3, true, Some(34962)),
                "NORMAL": g.floats(&nrm, "VEC3", 3, false, Some(34962)),
                "TEXCOORD_0": g.floats(&uv, "VEC2", 2, false, Some(34962)),
            });
            if sh.has[1] {
                attrs["COLOR_0"] = g.floats(&clr, "VEC4", 4, false, Some(34962)).into();
            }
            if sh.single.is_none() {
                attrs["JOINTS_0"] = g.u16x4(&joints).into();
                attrs["WEIGHTS_0"] = g.floats(&weights, "VEC4", 4, false, Some(34962)).into();
            }
            let indices = g.u32s(&sh.tris);
            g.meshes.push(json!({ "name": mat.name, "primitives": [{ "attributes": attrs, "indices": indices, "material": mat_ids[*mi] }] }));
            let mesh = g.meshes.len() - 1;
            let mut node = json!({ "name": mat.name, "mesh": mesh });
            if sh.single.is_none() {
                let skin = *skin_joints.get_or_insert_with(|| {
                    let ibm: Vec<f32> = m
                        .bones
                        .iter()
                        .flat_map(|b| {
                            let i = inverse(&b.world);
                            // column-major 4x4
                            [i[0][0], i[1][0], i[2][0], 0.0, i[0][1], i[1][1], i[2][1], 0.0, i[0][2], i[1][2], i[2][2], 0.0, i[0][3], i[1][3], i[2][3], 1.0]
                        })
                        .collect();
                    let acc = g.floats(&ibm, "MAT4", 16, false, None);
                    g.skins.push(json!({ "joints": (0..m.bones.len()).map(|i| base_node + i).collect::<Vec<_>>(), "inverseBindMatrices": acc }));
                    g.skins.len() - 1
                });
                node["skin"] = skin.into();
                g.nodes.push(node);
                let id = g.nodes.len() - 1;
                g.nodes[model_root]["children"].as_array_mut().unwrap().push(id.into());
            } else {
                g.nodes.push(node);
                let id = g.nodes.len() - 1;
                let parent = single_bone.map(|b| base_node + b).unwrap_or(model_root);
                let ch = g.nodes[parent].get_mut("children");
                match ch {
                    Some(c) => c.as_array_mut().unwrap().push(id.into()),
                    None => g.nodes[parent]["children"] = json!([id]),
                }
            }
        }
        // Skeletal animations targeting this model's bones.
        for a in anims {
            crate::anim::chr_to_gltf(&mut g, a, m, base_node);
        }
    }
    Converted { glb: g.glb(), info: Value::Object(info) }
}
