//! Pilotwings Resort vehicles (optional add-on). Their physics follow the 3DS
//! game: a rigid body in metres stepped at its 59.94 Hz, with each vehicle's
//! own control and aerodynamics. Positions convert to the Wii world through
//! `pwr::world`, which also mirrors the 3DS game's Z axis.

use crate::collide::Collision;
use crate::input::Controls;
use crate::player::{CamRig, Player};
use crate::plane::Flying;
use crate::pwr::{self, SCALE};
use crate::save::Save;
use crate::world::MainCamera;
use crate::{AppState, Mode};
use bevy::prelude::*;
use serde_json::Value;

/// One 3DS frame.
const DT: f32 = 1.0 / 59.94;
const GRAVITY: f32 = 9.81;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Plane,
    Jet,
    Glider,
}

/// Selectable add-on vehicles, in menu order (after the two Wii planes).
pub const KINDS: &[Kind] = &[Kind::Plane, Kind::Jet, Kind::Glider];

impl Kind {
    pub fn container(self) -> &'static str {
        match self { Kind::Plane => "plane", Kind::Jet => "jet", Kind::Glider => "glider" }
    }
    /// The vehicle's name in the 3DS game's text.
    pub fn name_key(self) -> &'static str {
        match self { Kind::Plane => "Vehicle:PLANE", Kind::Jet => "Vehicle:JET", Kind::Glider => "Vehicle:GLIDER" }
    }
    /// Flown by the Wii plane's physics (`plane.rs`) rather than this module.
    pub fn wii_physics(self) -> bool { matches!(self, Kind::Plane | Kind::Jet) }
}

/// The add-on vehicle selected in the save, if any.
pub fn selected(save: &Save) -> Option<Kind> {
    (save.plane >= 2 && pwr::available()).then(|| KINDS.get(save.plane as usize - 2).copied()).flatten()
}

/// Remaps `x` from `a..b` to `c..d`, clamped (the 3DS game's ubiquitous curve).
fn remap(a: f32, b: f32, c: f32, d: f32, x: f32) -> f32 {
    c + (d - c) * ((x - a) / (b - a)).clamp(0.0, 1.0)
}

/// Rigid body in 3DS space (metres, its handedness). `rot` maps the body's
/// right/up/forward (X/Y/Z) axes into the world.
#[derive(Clone, Debug)]
pub struct Body {
    pub pos: Vec3,
    pub vel: Vec3,
    /// World-space angular velocity, radians per second.
    pub ang: Vec3,
    pub rot: Quat,
    mass: f32,
    inertia: Vec3,
    force: Vec3,
    torque: Vec3,
}

impl Body {
    fn new(mass: f32, inertia: Vec3) -> Self {
        Self { pos: Vec3::ZERO, vel: Vec3::ZERO, ang: Vec3::ZERO, rot: Quat::IDENTITY, mass, inertia, force: Vec3::ZERO, torque: Vec3::ZERO }
    }
    fn axes(&self) -> (Vec3, Vec3, Vec3) { (self.rot * Vec3::X, self.rot * Vec3::Y, self.rot * Vec3::Z) }
    fn accelerate(&mut self, a: Vec3) { self.force += a * self.mass; }
    /// A flat plate through the centre resisting motion along its normal.
    fn plate(&mut self, normal: Vec3, k: f32) {
        let n = self.rot * normal;
        let vn = self.vel.dot(n);
        self.force += n * (-k * vn * vn.abs().min(50.0));
    }
    /// Gravity, then velocity (300 m/s cap), position and rotation (10 rad/s cap).
    fn integrate(&mut self) {
        self.force.y -= self.mass * GRAVITY;
        self.vel += self.force / self.mass * DT;
        if self.vel.length_squared() > 300.0 * 300.0 { self.vel = self.vel.normalize() * 300.0; }
        self.pos += self.vel * DT;
        let local = self.rot.inverse() * self.torque;
        self.ang += self.rot * (local / self.inertia) * DT;
        if self.ang.length_squared() > 100.0 { self.ang = self.ang.normalize() * 10.0; }
        let turn = self.ang * DT;
        if turn.length() > 1e-4 { self.rot = (Quat::from_scaled_axis(turn) * self.rot).normalize(); }
        self.force = Vec3::ZERO;
        self.torque = Vec3::ZERO;
    }
    /// Wii-world transform (the model itself is already mirrored).
    fn transform(&self) -> Transform {
        let q = self.rot;
        Transform { translation: pwr::world(self.pos.to_array()), rotation: Quat::from_xyzw(-q.x, -q.y, q.z, q.w), scale: Vec3::splat(SCALE) }
    }
}

/// Inputs in the 3DS game's sense: `pitch` > 0 raises the nose, `steer` > 0 turns right.
#[derive(Default, Clone, Copy)]
pub struct Input { pub pitch: f32, pub steer: f32, pub brake: bool }

/// Hang glider (3DS `GliderBase` flight state).
#[derive(Clone, Debug, Default)]
pub struct Glider {
    pitch: f32,
    steer: f32,
    brake: f32,
    /// Thermal lift felt, eased towards the air's.
    pub thermal: f32,
}

impl Glider {
    /// One frame of flight. `lift` is the thermal strength at the glider.
    pub fn step(&mut self, b: &mut Body, input: Input, lift: f32) {
        // Stick and brake ease in.
        self.pitch += (input.pitch - self.pitch).clamp(-0.0834168, 0.0834168);
        self.steer += (input.steer - self.steer).clamp(-0.0834168, 0.0834168);
        let target = if input.brake { 1.0 } else { 0.0 };
        let rate = remap(0.0, 1.0, if input.brake { 3.5 } else { 2.0 }, 2.0, self.brake);
        let step = remap(0.15, 0.0, rate * DT, 0.0, (target - self.brake).abs());
        self.brake += (target - self.brake).clamp(-step, step);
        // Thermals carry the glider up directly.
        self.thermal += (lift - self.thermal).clamp(-0.00583917, 0.00583917);
        b.pos.y += self.thermal * 53.3333 * DT;

        let (x, y, z) = b.axes();
        let speed = b.vel.dot(z);
        // Attitude control: bank to the steer, yaw with it, and hold a nose-down
        // angle that grows as the glider slows; the stick and brake shift it.
        let pitch = self.pitch + self.brake.clamp(-1.0, 1.0) * 0.25;
        let want = remap(22.2222, 4.44444, 0.00523599, 0.959931, speed) - pitch * remap(6.66667, 2.22222, 0.959931, 0.0349066, speed);
        let rate = remap(22.2222, 6.66667, 1.0, 0.0, speed) * pitch * -0.6 * remap(0.75, 0.95, 1.0, 0.0, z.y.abs());
        let nose_down = (-z.y).atan2(Vec2::new(z.x, z.z).length());
        let roll = ((x.y + 0.785398 * self.steer) * -5.0 - b.ang.dot(z) * 4.0).clamp(-10.0, 10.0);
        let pitch_t = ((rate - b.ang.dot(x)) * 4.0 + (want - nose_down) * 2.5).clamp(-10.0, 10.0);
        let yaw = ((0.4 * self.steer - b.ang.dot(y)) * 4.0).clamp(-10.0, 10.0);
        b.torque += (z * roll * b.inertia.z + x * pitch_t * b.inertia.x + y * yaw * b.inertia.y) * 10.0;
        // The wing resists motion through its plane; a small side area and drag.
        b.plate(Vec3::Y, 1050.0);
        b.plate(Vec3::X, 50.0);
        let mut accel = b.vel * (-0.01 - 0.24 * self.brake);
        if b.pos.y < 0.0 {
            accel.y += remap(0.0, 5.0, 0.0, 39.24, -b.pos.y);
            accel -= b.vel * remap(0.0, 0.25, 0.0, 0.5, -b.pos.y);
        }
        // Soft ceiling.
        if b.pos.y > 1372.5 { accel.y += (1372.5 - b.pos.y) * GRAVITY; }
        b.accelerate(accel);
    }
}

/// Air columns that lift gliders: cylinders fitted to the 3DS bounds.
#[derive(Resource, Default)]
pub struct Thermals(Vec<Thermal>);

#[derive(Clone, Copy, Debug)]
struct Thermal { center: Vec3, radius: f32, half: f32, bottom: f32, top: f32 }

impl Thermals {
    fn load(table: &Value) -> Self {
        let f = |v: &Value| v.as_f64().unwrap_or(0.0) as f32;
        Self(table["bound"].as_array().into_iter().flatten().filter_map(|b| {
            let pts: Vec<Vec2> = (0..8).map_while(|i| b[format!("bound{i}")].as_array().map(|p| Vec2::new(f(&p[0]), f(&p[1])))).collect();
            if pts.is_empty() { return None; }
            let c = pts.iter().sum::<Vec2>() / pts.len() as f32;
            let radius = (pts.iter().map(|p| p.distance_squared(c)).sum::<f32>() / pts.len() as f32).sqrt();
            let (lo, hi) = (f(&b["y_bottom"]), f(&b["y_top"]));
            let strength = b["strength"].as_f64().map_or(1.0, |s| s as f32);
            Some(Thermal { center: Vec3::new(c.x, (lo + hi) * 0.5, c.y), radius, half: (hi - lo) * 0.5, bottom: strength, top: f(&b["top_strength"]) * strength })
        }).collect())
    }
    /// Strongest lift at a point (bottom strength fading to the top's).
    pub fn lift(&self, p: Vec3) -> f32 {
        self.0.iter().filter(|t| (p.y - t.center.y).abs() < t.half && Vec2::new(p.x - t.center.x, p.z - t.center.z).length_squared() < t.radius * t.radius)
            .map(|t| t.top + (t.bottom - t.top) * (((p.y - t.center.y) - t.half) / (-2.0 * t.half)).clamp(0.0, 1.0))
            .fold(0.0, f32::max)
    }
}

#[derive(Component)]
pub struct Vehicle {
    pub kind: Kind,
    pub body: Body,
    glider: Glider,
}

pub fn plugin(app: &mut App) {
    if !pwr::available() { return; }
    let thermals = std::fs::read(crate::asset_dir().join("pwr/data/wuhu_free_glider/thermal.val.json")).ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok()).map_or_else(Thermals::default, |t| Thermals::load(&t));
    app.insert_resource(thermals)
        .add_systems(Update, board.before(crate::plane::board).run_if(crate::menu::playing))
        .add_systems(FixedUpdate, fly.run_if(crate::menu::playing.and_then(in_state(Mode::Fly))))
        .add_systems(Update, camera.run_if(crate::menu::playing.and_then(in_state(Mode::Fly))))
        .add_systems(OnEnter(AppState::Play), spawn_thermals)
        .add_systems(Update, show_thermals)
        .add_systems(OnExit(Mode::Fly), despawn)
        .add_systems(OnExit(AppState::Play), despawn);
}

/// Boards the selected add-on vehicle, or steps off it.
fn board(
    mut c: ResMut<Controls>,
    mode: Res<State<Mode>>,
    mut next: ResMut<NextState<Mode>>,
    mut commands: Commands,
    assets: Res<AssetServer>,
    save: Res<Save>,
    rig: Res<CamRig>,
    mut player: Single<(&mut Player, &mut Transform, &mut Visibility), Without<Vehicle>>,
    vehicles: Query<&Vehicle>,
) {
    if !c.board { return; }
    match mode.get() {
        Mode::Walk => {
            let Some(kind) = selected(&save).filter(|k| !k.wii_physics()) else { return };
            // The Wii plane's boarding code must not also react.
            c.board = false;
            let mut body = match kind { Kind::Glider => Body::new(454.5, Vec3::splat(422.799)), _ => return };
            // Launch 80 m above the player, facing where the camera looks.
            let feet = player.1.translation;
            body.pos = Vec3::new(feet.x, feet.y, -feet.z) / SCALE + Vec3::Y * 80.0;
            body.rot = Quat::from_rotation_y(-rig.yaw);
            body.vel = body.rot * Vec3::Z * 11.1111;
            let t = body.transform();
            commands.spawn((Vehicle { kind, body, glider: Glider::default() }, Flying, t, Visibility::default())).with_children(|v| {
                v.spawn(WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(format!("pwr/models/{}_lod1.glb", kind.container())))));
            });
            *player.2 = Visibility::Hidden;
            next.set(Mode::Fly);
        }
        Mode::Fly => {
            let Ok(v) = vehicles.single() else { return };
            c.board = false;
            step_off(&mut *player, &v.body, true);
            next.set(Mode::Walk);
        }
    }
}

/// Leaves the vehicle: with a parachute in the air, on foot after landing.
fn step_off(player: &mut (Mut<Player>, Mut<Transform>, Mut<Visibility>), body: &Body, airborne: bool) {
    let (p, t, v) = player;
    t.translation = pwr::world(body.pos.to_array());
    let vel = Vec3::new(body.vel.x, body.vel.y, -body.vel.z) * SCALE / 60.0;
    p.vel = if airborne { vel * 60.0 * 0.3 + Vec3::Y * 300.0 } else { Vec3::ZERO };
    p.parachute = airborne;
    **v = Visibility::Inherited;
}

fn fly(
    c: Res<Controls>,
    save: Res<Save>,
    col: Res<Collision>,
    thermals: Res<Thermals>,
    mut next: ResMut<NextState<Mode>>,
    mut q: Query<(&mut Vehicle, &mut Transform)>,
    mut player: Single<(&mut Player, &mut Transform, &mut Visibility), Without<Vehicle>>,
) {
    let Ok((mut v, mut t)) = q.single_mut() else { return };
    let stick = c.stick * save.flight_sensitivity;
    // The Wii controls push the stick up to dive; the 3DS pitch input raises the nose.
    let input = Input { pitch: if save.invert_pitch { stick.y } else { -stick.y }, steer: stick.x, brake: c.slow > 0.5 || c.cut };
    let v = &mut *v;
    let from = pwr::world(v.body.pos.to_array());
    match v.kind {
        Kind::Glider => {
            let lift = thermals.lift(v.body.pos);
            v.glider.step(&mut v.body, input, lift);
        }
        Kind::Plane | Kind::Jet => {}
    }
    v.body.integrate();
    let to = pwr::world(v.body.pos.to_array());
    // Ground contact: land gently, otherwise the pilot falls off (3DS limits:
    // 13.3 m/s into the ground or 26.7 m/s overall).
    let travel = to - from;
    let hit = col.raycast(from, travel.normalize_or_zero(), travel.length() + 0.95 * SCALE);
    let below = col.ground(to, 0.0, 0.95 * SCALE);
    if let Some(h) = hit.or(below) {
        let normal = Vec3::new(h.n.x, h.n.y, -h.n.z) * h.n.y.signum();
        let water = h.flags & 0x1f == 6;
        let gentle = !water && v.body.vel.dot(normal) > -13.3333 && v.body.vel.length() < 26.6667 && h.n.y.abs() > 0.6;
        step_off(&mut *player, &v.body, !gentle);
        if gentle { player.1.translation = h.pos + Vec3::Y * 2.0; }
        next.set(Mode::Walk);
        return;
    }
    *t = v.body.transform();
}

/// Chase camera from the vehicle's own settings (distance and height behind
/// the pilot), following the direction of travel.
fn camera(
    c: Res<Controls>,
    time: Res<Time>,
    col: Res<Collision>,
    mut look: Local<Vec2>,
    mut heading: Local<Vec3>,
    q: Query<(&Vehicle, &Transform)>,
    mut cam: Single<&mut Transform, (With<MainCamera>, Without<Vehicle>)>,
) {
    let Ok((v, t)) = q.single() else { return };
    let dt = time.delta_secs();
    if c.recenter { *look = Vec2::ZERO; }
    if c.look.length() > 0.0005 {
        *look = (*look + c.look * Vec2::new(-1.0, 1.0)).clamp(Vec2::new(-2.5, -1.2), Vec2::new(2.5, 1.2));
    } else {
        *look *= (-0.7 * dt).exp();
    }
    let travel = Vec3::new(v.body.vel.x, 0.0, -v.body.vel.z);
    let nose = t.rotation * Vec3::NEG_Z;
    let flat = if travel.length() > 1.0 { travel } else { nose.with_y(0.0) };
    if *heading == Vec3::ZERO { *heading = flat.normalize_or(Vec3::NEG_Z); }
    *heading = heading.lerp(flat.normalize_or(*heading), 1.0 - (-3.0 * dt).exp()).normalize_or(Vec3::NEG_Z);
    let yaw = (-heading.x).atan2(-heading.z);
    let fwd = Quat::from_euler(EulerRot::YXZ, yaw + look.x, -look.y - 0.12, 0.0) * Vec3::NEG_Z;
    // glider_/vehicle.val camera: chase_dist 9.9 m, chase_dy 2.2 m.
    let target = t.translation + Vec3::Y * 2.2 * SCALE;
    let dir = (-fwd).normalize();
    let mut dist = 9.9 * SCALE;
    if let Some(h) = col.raycast(target, dir, dist) { dist = (h.t - 10.0).max(20.0); }
    let want = target + dir * dist;
    cam.translation = cam.translation.lerp(want, 1.0 - (-10.0 * dt).exp());
    cam.look_at(target + fwd * 6.0 * SCALE, Vec3::Y);
}

#[derive(Component)]
struct ThermalColumn;

/// The 3DS game's thermal column model, stretched over each thermal.
fn spawn_thermals(mut commands: Commands, assets: Res<AssetServer>, thermals: Res<Thermals>, existing: Query<(), With<ThermalColumn>>) {
    if !existing.is_empty() { return; }
    let scene = assets.load(GltfAssetLabel::Scene(0).from_asset("pwr/models/race_thermal_lg.glb"));
    for t in &thermals.0 {
        let transform = Transform::from_translation(pwr::world(t.center.to_array())).with_scale(Vec3::new(t.radius, t.half, t.radius) * SCALE);
        commands.spawn((WorldAssetRoot(scene.clone()), transform, Visibility::Hidden, ThermalColumn, DespawnOnExit(AppState::Play)));
    }
}

/// Thermals only matter (and show) while gliding.
fn show_thermals(vehicles: Query<&Vehicle>, mut columns: Query<&mut Visibility, With<ThermalColumn>>) {
    let gliding = vehicles.iter().any(|v| v.kind == Kind::Glider);
    for mut vis in &mut columns {
        vis.set_if_neq(if gliding { Visibility::Inherited } else { Visibility::Hidden });
    }
}

fn despawn(mut commands: Commands, q: Query<Entity, With<Vehicle>>) {
    for e in &q { commands.entity(e).despawn(); }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn glider() -> (Glider, Body) {
        let mut b = Body::new(454.5, Vec3::splat(422.799));
        b.pos = Vec3::new(0.0, 300.0, 0.0);
        b.vel = Vec3::Z * 11.1111;
        (Glider::default(), b)
    }

    fn fly(g: &mut Glider, b: &mut Body, input: Input, lift: f32, seconds: f32) {
        for _ in 0..(seconds / DT) as usize { g.step(b, input, lift); b.integrate(); }
    }

    #[test]
    fn hands_off_glide_settles_into_a_shallow_descent() {
        let (mut g, mut b) = glider();
        fly(&mut g, &mut b, Input::default(), 0.0, 30.0);
        let (sink, speed) = (-b.vel.y, b.vel.length());
        assert!(sink > 0.5 && sink < 6.0, "sink {sink}");
        assert!(speed > 8.0 && speed < 30.0, "speed {speed}");
        assert!(b.vel.dot(b.rot * Vec3::Z) > 0.0, "flies forward");
    }

    #[test]
    fn a_thermal_carries_the_glider_up() {
        let (mut g, mut b) = glider();
        fly(&mut g, &mut b, Input::default(), 1.0, 10.0);
        assert!(b.pos.y > 400.0, "rose to {}", b.pos.y);
    }

    #[test]
    fn steering_right_turns_right() {
        let (mut g, mut b) = glider();
        fly(&mut g, &mut b, Input { steer: 1.0, ..default() }, 0.0, 4.0);
        let heading = b.rot * Vec3::Z;
        // Right is +X in the 3DS frame.
        assert!(heading.x > 0.3, "heading {heading}");
    }

    #[test]
    fn thermal_lift_fades_from_bottom_to_top_strength() {
        let t = Thermals(vec![Thermal { center: Vec3::new(0.0, 100.0, 0.0), radius: 50.0, half: 100.0, bottom: 1.0, top: 0.5 }]);
        assert!((t.lift(Vec3::new(0.0, 0.5, 0.0)) - 1.0).abs() < 0.01);
        assert!((t.lift(Vec3::new(0.0, 199.5, 0.0)) - 0.5).abs() < 0.01);
        assert_eq!(t.lift(Vec3::new(60.0, 100.0, 0.0)), 0.0);
    }

    #[test]
    fn mirrored_transform_keeps_heading() {
        let mut b = Body::new(1.0, Vec3::ONE);
        b.rot = Quat::from_rotation_y(0.4);
        // 3DS forward (+Z) mirrors to the Wii's +Z flipped: the model faces -Z after mirroring.
        let fwd3ds = b.rot * Vec3::Z;
        let wii = b.transform().rotation * Vec3::NEG_Z;
        assert!(wii.distance(Vec3::new(fwd3ds.x, fwd3ds.y, -fwd3ds.z)) < 1e-4);
    }
}
