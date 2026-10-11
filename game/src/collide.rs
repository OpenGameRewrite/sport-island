//! Static island collision from the disc's KCL files, in a uniform XZ grid,
//! plus moving platforms (the add-on's landing-pad blimp).

use bevy::math::Affine3A;
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
    pub platforms: Vec<Platform>,
}

/// Triangles in their own space that move as a whole.
pub struct Platform {
    tris: Vec<Tri>,
    to_world: Affine3A,
    /// The last update's motion and its duration, which carry whatever
    /// stands on the platform.
    pub moved: Affine3A,
    dt: f32,
}

impl Platform {
    pub fn new(tris: Vec<[Vec3; 3]>, to_world: Affine3A) -> Self {
        Self { tris: tris.into_iter().filter_map(|[a, b, c]| tri(a, b, c, 0)).collect(), to_world, moved: Affine3A::IDENTITY, dt: 1.0 }
    }
    pub fn set(&mut self, to_world: Affine3A, dt: f32) {
        self.moved = to_world * self.to_world.inverse();
        self.to_world = to_world;
        self.dt = dt.max(1e-4);
    }
    /// How far a point riding the platform moves in `dt` seconds.
    pub fn carry(&self, p: Vec3, dt: f32) -> Vec3 {
        (self.moved.transform_point3(p) - p) * (dt / self.dt)
    }
}

pub struct Hit {
    pub t: f32,
    pub pos: Vec3,
    pub n: Vec3,
    pub flags: u32,
    pub platform: Option<usize>,
}

/// Triangles of a collision file: a count, then three corners and a flags word each.
pub fn read_tris(bytes: &[u8]) -> Vec<([Vec3; 3], u32)> {
    let f = |o: usize| f32::from_le_bytes(bytes[o..o + 4].try_into().unwrap());
    let n = bytes.get(0..4).map_or(0, |b| u32::from_le_bytes(b.try_into().unwrap()) as usize).min(bytes.len().saturating_sub(4) / 40);
    (0..n).map(|i| {
        let o = 4 + i * 40;
        let v = |k: usize| Vec3::new(f(o + k * 12), f(o + k * 12 + 4), f(o + k * 12 + 8));
        ([v(0), v(1), v(2)], u32::from_le_bytes(bytes[o + 36..o + 40].try_into().unwrap()))
    }).collect()
}

fn tri(a: Vec3, b: Vec3, c: Vec3, flags: u32) -> Option<Tri> {
    let n = (b - a).cross(c - a).normalize_or_zero();
    (n != Vec3::ZERO).then_some(Tri { a, b, c, n, flags })
}

/// Möller–Trumbore, double sided: distance along `d` (any length) to the triangle.
fn ray_tri(o: Vec3, d: Vec3, len: f32, t: &Tri) -> Option<f32> {
    let (e1, e2) = (t.b - t.a, t.c - t.a);
    let p = d.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-12 {
        return None;
    }
    let s = o - t.a;
    let u = s.dot(p) / det;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = d.dot(q) / det;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let dist = e2.dot(q) / det;
    (dist >= 0.0 && dist <= len).then_some(dist)
}

fn cell(v: f32) -> i32 { (v / CELL).floor() as i32 }

impl Collision {
    fn add_file(&mut self, bytes: &[u8]) {
        for ([a, b, c], flags) in read_tris(bytes) {
            self.add(a, b, c, flags);
        }
    }

    /// Adds one static triangle.
    pub fn add(&mut self, a: Vec3, b: Vec3, c: Vec3, flags: u32) {
        let Some(t) = tri(a, b, c, flags) else { return };
        let idx = self.tris.len() as u32;
        let (lo, hi) = (a.min(b).min(c), a.max(b).max(c));
        for x in cell(lo.x)..=cell(hi.x) {
            for z in cell(lo.z)..=cell(hi.z) {
                self.grid.entry((x, z)).or_default().push(idx);
            }
        }
        self.tris.push(t);
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
            if let Some(dist) = ray_tri(o, d, len, t) && best.as_ref().is_none_or(|b| dist < b.t) {
                best = Some(Hit { t: dist, pos: o + d * dist, n: t.n, flags: t.flags, platform: None });
            }
        }
        // Platforms: the same ray in each platform's own space (the scaled
        // direction keeps distances in world units).
        for (k, p) in self.platforms.iter().enumerate() {
            let inv = p.to_world.inverse();
            let (lo, ld) = (inv.transform_point3(o), inv.transform_vector3(d));
            for t in &p.tris {
                if let Some(dist) = ray_tri(lo, ld, len, t) && best.as_ref().is_none_or(|b| dist < b.t) {
                    let n = p.to_world.transform_vector3(t.n).normalize_or_zero();
                    best = Some(Hit { t: dist, pos: o + d * dist, n, flags: t.flags, platform: Some(k) });
                }
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

    #[test]
    fn moving_platform_is_hit_where_it_is_and_reports_its_motion() {
        let mut c = Collision::default();
        let floor = vec![[Vec3::new(-1., 0., -1.), Vec3::new(-1., 0., 1.), Vec3::new(1., 0., 0.)]];
        // Ten times larger and 100 up.
        let at = |x: f32| Affine3A::from_scale_rotation_translation(Vec3::splat(10.), Quat::IDENTITY, Vec3::new(x, 100., 0.));
        c.platforms.push(Platform::new(floor, at(0.)));
        let h = c.ground(Vec3::new(0., 105., 0.), 0., 20.).unwrap();
        assert!((h.pos.y - 100.).abs() < 1e-3 && (h.t - 5.).abs() < 1e-3 && h.platform == Some(0));
        c.platforms[0].set(at(30.), 0.5);
        assert!(c.ground(Vec3::new(0., 105., 0.), 0., 20.).is_none());
        assert!((c.platforms[0].carry(Vec3::ZERO, 0.25) - Vec3::X * 15.).length() < 1e-3);
    }
}
