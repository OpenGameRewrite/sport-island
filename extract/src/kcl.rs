//! KCL collision -> flat triangle list.
//! Output (little endian): u32 count, then per triangle 9 x f32 (a, b, c) + u32 flags.

use crate::disc::{be16, be32, bef};

type V = [f32; 3];
fn cross(a: V, b: V) -> V { [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]] }
fn dot(a: V, b: V) -> f32 { a[0] * b[0] + a[1] * b[1] + a[2] * b[2] }

pub fn to_tris(d: &[u8]) -> Vec<u8> {
    let (pos, nrm, prisms, octree) = (be32(d, 0) as usize, be32(d, 4) as usize, be32(d, 8) as usize + 0x10, be32(d, 12) as usize);
    let v = |base: usize, i: usize| -> V { [bef(d, base + i * 12), bef(d, base + i * 12 + 4), bef(d, base + i * 12 + 8)] };
    let n = (octree.saturating_sub(prisms)) / 0x10;
    let mut out = Vec::with_capacity(4 + n * 40);
    out.extend_from_slice(&0u32.to_le_bytes());
    let mut count = 0u32;
    for i in 0..n {
        let p = prisms + i * 0x10;
        let len = bef(d, p);
        let a = v(pos, be16(d, p + 4) as usize);
        let f = v(nrm, be16(d, p + 6) as usize);
        let (ea, eb, ec) = (v(nrm, be16(d, p + 8) as usize), v(nrm, be16(d, p + 10) as usize), v(nrm, be16(d, p + 12) as usize));
        let (ca, cb) = (cross(ea, f), cross(eb, f));
        let (da, db) = (dot(ca, ec), dot(cb, ec));
        if da.abs() < 1e-6 || db.abs() < 1e-6 {
            continue;
        }
        let b: V = std::array::from_fn(|k| a[k] + cb[k] * (len / db));
        let c: V = std::array::from_fn(|k| a[k] + ca[k] * (len / da));
        if [b, c].iter().flatten().any(|x| !x.is_finite()) {
            continue;
        }
        for x in a.iter().chain(&b).chain(&c) {
            out.extend_from_slice(&x.to_le_bytes());
        }
        out.extend_from_slice(&(be16(d, p + 14) as u32).to_le_bytes());
        count += 1;
    }
    out[..4].copy_from_slice(&count.to_le_bytes());
    out
}
