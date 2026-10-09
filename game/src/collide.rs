//! Static island collision from the disc's KCL files, in a uniform XZ grid.

use bevy::prelude::*;
use std::collections::HashMap;

const CELL: f32 = 400.0;

pub struct Tri {
    pub a: Vec3,
    pub b: Vec3,
    pub c: Vec3,
    pub n: Vec3,
    pub flags: u32,
}

#[derive(Resource, Default)]
pub struct Collision {
    pub tris: Vec<Tri>,
    grid: HashMap<(i32, i32), Vec<u32>>,
}

pub struct Hit {
    pub t: f32,
    pub pos: Vec3,
    pub n: Vec3,
    pub flags: u32,
}

fn cell(v: f32) -> i32 { (v / CELL).floor() as i32 }

impl Collision {
    fn add_file(&mut self, bytes: &[u8]) {
        let f = |o: usize| f32::from_le_bytes(bytes[o..o + 4].try_into().unwrap());
        let n = u32::from_le_bytes(bytes[0..4].try_into().unwrap()) as usize;
        for i in 0..n {
            let o = 4 + i * 40;
            let v = |k: usize| Vec3::new(f(o + k * 12), f(o + k * 12 + 4), f(o + k * 12 + 8));
            let (a, b, c) = (v(0), v(1), v(2));
            let nrm = (b - a).cross(c - a).normalize_or_zero();
            if nrm == Vec3::ZERO {
                continue;
            }
            let idx = self.tris.len() as u32;
            let (lo, hi) = (a.min(b).min(c), a.max(b).max(c));
            for x in cell(lo.x)..=cell(hi.x) {
                for z in cell(lo.z)..=cell(hi.z) {
                    self.grid.entry((x, z)).or_default().push(idx);
                }
            }
            self.tris.push(Tri { a, b, c, n: nrm, flags: u32::from_le_bytes(bytes[o + 36..o + 40].try_into().unwrap()) });
        }
    }

    fn candidates(&self, lo: Vec3, hi: Vec3) -> Vec<u32> {
        let mut out = vec![];
        for x in cell(lo.x)..=cell(hi.x) {
            for z in cell(lo.z)..=cell(hi.z) {
                if let Some(v) = self.grid.get(&(x, z)) {
                    out.extend_from_slice(v);
                }
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    /// First hit along a segment from `o` in direction `d` (unit) up to `len`.
    pub fn raycast(&self, o: Vec3, d: Vec3, len: f32) -> Option<Hit> {
        let e = o + d * len;
        let mut best: Option<Hit> = None;
        for i in self.candidates(o.min(e), o.max(e)) {
            let t = &self.tris[i as usize];
            // Möller–Trumbore, double sided.
            let (e1, e2) = (t.b - t.a, t.c - t.a);
            let p = d.cross(e2);
            let det = e1.dot(p);
            if det.abs() < 1e-8 {
                continue;
            }
            let s = o - t.a;
            let u = s.dot(p) / det;
            if !(0.0..=1.0).contains(&u) {
                continue;
            }
            let q = s.cross(e1);
            let v = d.dot(q) / det;
            if v < 0.0 || u + v > 1.0 {
                continue;
            }
            let dist = e2.dot(q) / det;
            if dist >= 0.0 && dist <= len && best.as_ref().is_none_or(|b| dist < b.t) {
                best = Some(Hit { t: dist, pos: o + d * dist, n: t.n, flags: t.flags });
            }
        }
        best
    }

    /// Highest walkable surface below `p` (searching from `up` above it).
    pub fn ground(&self, p: Vec3, up: f32, down: f32) -> Option<Hit> {
        self.raycast(p + Vec3::Y * up, Vec3::NEG_Y, up + down)
    }

    /// Pushes a sphere out of steep geometry (walls); returns the corrected centre.
    pub fn push_sphere(&self, mut c: Vec3, r: f32, max_normal_y: f32) -> Vec3 {
        for i in self.candidates(c - Vec3::splat(r), c + Vec3::splat(r)) {
            let t = &self.tris[i as usize];
            if t.n.y.abs() > max_normal_y {
                continue;
            }
            let q = closest_on_tri(c, t.a, t.b, t.c);
            let d = c - q;
            let dist = d.length();
            if dist < r && dist > 1e-4 {
                c += d / dist * (r - dist);
            }
        }
        c
    }
}

fn closest_on_tri(p: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Vec3 {
    let (ab, ac, ap) = (b - a, c - a, p - a);
    let (d1, d2) = (ab.dot(ap), ac.dot(ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = p - b;
    let (d3, d4) = (ab.dot(bp), ac.dot(bp));
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return a + ab * (d1 / (d1 - d3));
    }
    let cp = p - c;
    let (d5, d6) = (ab.dot(cp), ac.dot(cp));
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return a + ac * (d2 / (d2 - d6));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        return b + (c - b) * ((d4 - d3) / ((d4 - d3) + (d5 - d6)));
    }
    let denom = 1.0 / (va + vb + vc);
    a + ab * (vb * denom) + ac * (vc * denom)
}

pub fn plugin(app: &mut App) {
    let mut col = Collision::default();
    for i in 1..=8 {
        if let Ok(b) = std::fs::read(crate::asset_dir().join(format!("data/WS2_pln_island{i}.col"))) {
            col.add_file(&b);
        }
    }
    info!("collision: {} triangles", col.tris.len());
    app.insert_resource(col);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ray_hits_floor_and_sphere_leaves_wall() {
        let mut c = Collision::default();
        let tri = |a: Vec3, b: Vec3, cc: Vec3| {
            let mut v = vec![];
            for p in [a, b, cc] {
                v.extend(p.to_array().iter().flat_map(|f| f.to_le_bytes()));
            }
            v.extend(7u32.to_le_bytes());
            v
        };
        let mut bytes = 2u32.to_le_bytes().to_vec();
        bytes.extend(tri(Vec3::new(-100., 0., -100.), Vec3::new(-100., 0., 100.), Vec3::new(100., 0., 0.)));
        bytes.extend(tri(Vec3::new(50., -50., -100.), Vec3::new(50., 100., 0.), Vec3::new(50., -50., 100.)));
        c.add_file(&bytes);
        let h = c.ground(Vec3::new(0., 10., 0.), 5., 50.).unwrap();
        assert!((h.pos.y).abs() < 1e-3 && h.flags == 7);
        let p = c.push_sphere(Vec3::new(45., 0., 0.), 10., 0.7);
        assert!((p.x - 40.).abs() < 1e-3);
    }
}
