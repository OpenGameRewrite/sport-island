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
use bevy::world_serialization::WorldInstanceReady;
use serde_json::Value;

/// One 3DS frame.
const DT: f32 = 1.0 / 59.94;
const GRAVITY: f32 = 9.81;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Plane,
    Jet,
    Glider,
    PGlider,
    Rocket,
    SuperRocket,
}

/// Selectable add-on vehicles, in menu order (after the two Wii planes).
pub const KINDS: &[Kind] = &[Kind::Plane, Kind::Jet, Kind::Glider, Kind::PGlider, Kind::Rocket, Kind::SuperRocket];

impl Kind {
    pub fn container(self) -> &'static str {
        match self { Kind::Plane => "plane", Kind::Jet => "jet", Kind::Glider => "glider", Kind::PGlider => "pglider", Kind::Rocket => "rocket", Kind::SuperRocket => "superrocket" }
    }
    /// The vehicle's name in the 3DS game's text.
    pub fn name_key(self) -> &'static str {
        match self { Kind::Plane => "Vehicle:PLANE", Kind::Jet => "Vehicle:JET", Kind::Glider => "Vehicle:GLIDER", Kind::PGlider => "Vehicle:PGLIDER", Kind::Rocket => "Vehicle:ROCKET", Kind::SuperRocket => "Vehicle:SUPERROCKET" }
    }
    fn gliding(self) -> bool { matches!(self, Kind::Glider | Kind::PGlider) }
    pub fn plane(self) -> bool { matches!(self, Kind::Plane | Kind::Jet) }
    /// `vehicle.val` chase camera: distance behind and height above, metres.
    fn camera(self) -> (f32, f32) {
        match self { Kind::PGlider => (8.0, 0.5), Kind::Rocket | Kind::SuperRocket => (3.5, 0.0), _ => (9.9, 2.2) }
    }
    /// Flown by the Wii plane's physics (`plane.rs`) rather than this module:
    /// the 3DS plane and jet when the Wii-style handling setting is on.
    pub fn wii_physics(self, save: &Save) -> bool { self.plane() && save.plane_wii_style }
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

/// Tuning of the 3DS plane and Turbo Jet: the `plane` node of their
/// `vehicle.val`, with the game's defaults for missing keys.
#[derive(Clone, Copy, Debug)]
pub struct PlaneParams { power: f32, turbo: f32, drag: f32, water_drag: f32, brake_drag: f32, low_speed_brake_drag: f32 }

impl Default for PlaneParams {
    fn default() -> Self { Self { power: 1.0, turbo: 1.0, drag: 1.0, water_drag: 0.0, brake_drag: 0.0, low_speed_brake_drag: 0.0 } }
}

impl PlaneParams {
    fn load(kind: Kind) -> Self {
        let d = Self::default();
        let table = pwr::data(&format!("{}_/vehicle.val.json", kind.container()));
        let (plane, physics) = (&table["plane"][0], &table["physics"][0]);
        let f = |node: &Value, key: &str, or: f32| node[key].as_f64().map_or(or, |v| v as f32);
        Self {
            power: f(physics, "power_factor", d.power),
            turbo: f(plane, "turbo_power_factor", d.turbo),
            drag: f(plane, "drag_factor", d.drag),
            water_drag: f(plane, "water_extra_drag", d.water_drag),
            brake_drag: f(plane, "brake_extra_drag", d.brake_drag),
            low_speed_brake_drag: f(plane, "low_speed_brake_extra_drag", d.low_speed_brake_drag),
        }
    }
}

/// 3DS plane controls: `steer` > 0 banks right, `pitch` > 0 climbs; the
/// shoulder buttons lean (both: fly inverted) and double-tapped barrel-roll.
#[derive(Default, Clone, Copy)]
pub struct PlaneInput { pub steer: f32, pub pitch: f32, pub lean_left: bool, pub lean_right: bool, pub brake: bool, pub turbo: bool }

/// The 3DS plane's flight state: eased stick, coordinated turn, path hold,
/// leans, barrel rolls and the turbo meter.
#[derive(Clone, Debug)]
pub struct Flight {
    pub pitch: f32,
    pub roll: f32,
    raw_roll: f32,
    yaw: f32,
    flown: f32,
    /// Barrel roll under way: -1 left, 1 right, 0 none.
    pub barrel: f32,
    /// The previous barrel roll's direction and the seconds since it ended.
    pub last_barrel: f32,
    pub since_barrel: f32,
    barrel_time: f32,
    past_inverted: bool,
    tap_left: f32,
    tap_right: f32,
    held: (bool, bool),
    since_brake: f32,
    since_release: f32,
    /// Turbo: 0 ready, 1 burning, 2 fading, 3 recharging; `meter` 0..1.
    pub turbo: u8,
    turbo_time: f32,
    pub meter: f32,
    pub on_water: bool,
}

impl Default for Flight {
    fn default() -> Self {
        Self { pitch: 0.0, roll: 0.0, raw_roll: 0.0, yaw: 0.0, flown: 0.0, barrel: 0.0, last_barrel: 0.0, since_barrel: 10.0, barrel_time: 0.0, past_inverted: false,
            tap_left: 10.0, tap_right: 10.0, held: (false, false), since_brake: 10.0, since_release: 10.0, turbo: 0, turbo_time: 0.0, meter: 1.0, on_water: false }
    }
}

impl Flight {
    /// Bank angle about the nose, measured from the level heading frame
    /// (negative with the right wing down).
    fn bank(x: Vec3, z: Vec3) -> f32 {
        let heading = Vec3::new(z.x, 0.0, z.z).normalize_or(Vec3::new(-x.z, 0.0, x.x).normalize_or(Vec3::Z));
        x.dot(Vec3::Y).atan2(x.dot(Vec3::Y.cross(heading)))
    }

    /// One frame of the 3DS plane flight state.
    pub fn step(&mut self, b: &mut Body, input: PlaneInput, p: PlaneParams) {
        use std::f32::consts::{FRAC_PI_2, PI};
        self.flown += DT;
        let (x, y, z) = b.axes();
        // Eased stick; a full deflection counts fully, a half one at 3/8.
        let mag = Vec2::new(input.steer, input.pitch).length().min(1.0);
        let k = 0.5 + 0.5 * mag;
        self.pitch += (input.pitch * k - self.pitch).clamp(-0.0500501, 0.0500501);
        if self.flown > 1.0 { self.roll += (input.steer * k - self.roll).clamp(-0.0375375, 0.0375375); } else { self.roll *= 0.9; }
        self.raw_roll += (input.steer * mag - self.raw_roll).clamp(-0.0375375, 0.0375375);

        // Shoulder taps: a second tap within half a second barrel-rolls that way.
        for t in [&mut self.tap_left, &mut self.tap_right, &mut self.since_barrel] { *t += DT; }
        let (left_edge, right_edge) = (input.lean_left && !self.held.0, input.lean_right && !self.held.1);
        self.held = (input.lean_left, input.lean_right);
        if self.barrel == 0.0 {
            let start = if left_edge && !input.lean_right {
                if self.tap_left > 0.5 { self.tap_left = 0.0; 0.0 } else { -1.0 }
            } else if right_edge && !input.lean_left {
                if self.tap_right > 0.5 { self.tap_right = 0.0; 0.0 } else { 1.0 }
            } else { 0.0 };
            if start != 0.0 {
                (self.barrel, self.barrel_time, self.past_inverted, self.since_barrel, self.tap_left, self.tap_right) = (start, 0.0, false, 0.0, 0.0, 0.0);
            }
        }
        if self.barrel != 0.0 {
            // Spin about the nose at two turns a second, easing to half once
            // past upside down; stop when upright again (or after a second).
            self.barrel_time += DT;
            let speed = if self.past_inverted { remap(-0.25, 0.25, 1.0, 0.5, y.y) } else { 1.0 };
            b.torque += z * ((-self.barrel * speed * 12.5664 - b.ang.dot(z)) * b.inertia.z * 50.0);
            if !self.past_inverted {
                self.past_inverted = y.y < 0.25 || self.barrel_time > 1.0;
            } else if y.y > 0.25 || self.barrel_time > 1.0 {
                (self.last_barrel, self.barrel, self.since_barrel) = (self.barrel, 0.0, 0.0);
            }
        }

        // Bank to the stick (90° at full), or hold a lean / fly inverted.
        let bank = Self::bank(x, z);
        let mut target = FRAC_PI_2 * self.roll;
        let mut gain = 1.0;
        let mut turn_bank = bank;
        if self.barrel == 0.0 {
            let left = input.lean_left || self.tap_left < 0.33;
            let right = input.lean_right || self.tap_right < 0.33;
            if left != right {
                target = if left { -FRAC_PI_2 } else { FRAC_PI_2 };
                if x.y * target < 0.0 { gain = 2.0; }
            } else if left && right {
                if y.y < -0.1 { gain = 2.0; }
                target = if bank <= 0.0 { PI + target * 0.85 } else { target * 0.85 - PI };
                // Upside down, the turn follows the bank as seen from the cockpit.
                turn_bank = if bank > FRAC_PI_2 { PI - bank } else if bank < -FRAC_PI_2 { -PI - bank } else { bank };
            }
        }
        let fade = if self.barrel != 0.0 { 0.0 } else { remap(0.0, 0.7, 0.0, 1.0, self.since_barrel) };
        let roll = (gain * -7.5 * (bank + target) - 4.0 * b.ang.dot(z)).clamp(-10.0, 10.0);
        b.torque += z * roll * b.inertia.z * 10.0 * fade;

        // Banking turns about the vertical (on water the stick steers).
        if self.on_water {
            self.yaw = 0.3 * self.raw_roll + (self.yaw - 0.3 * self.raw_roll) * 0.95;
        } else {
            let err = turn_bank.clamp(-FRAC_PI_2, FRAC_PI_2) * -1.2 - self.yaw;
            let step = remap(0.5, 0.0, 0.0333667, 0.0, err.abs());
            self.yaw += err.clamp(-step, step);
        }
        let yaw = ((self.yaw * 0.5 - b.ang.dot(Vec3::Y)) * 2.0).clamp(-10.0, 10.0);
        b.torque += Vec3::Y * yaw * b.inertia.y * 10.0;

        // Path hold: the flight path (the nose when slow) climbs at up to
        // about 23° with the stick and levels out when it is released.
        let fwd = b.vel.dot(z);
        let path = b.vel.lerp(z, remap(8.88889, 17.7778, 1.0, 0.0, fwd));
        let path = path / path.length().max(0.01);
        let level_right = Vec3::Y.cross(Vec3::new(z.x, 0.0, z.z).normalize_or(Vec3::Z));
        let pitch = ((path.y - 0.785398 * self.pitch * 0.5) * 5.0 - b.ang.dot(level_right) * 2.0).clamp(-10.0, 10.0);
        b.torque += Vec3::Y.cross(z) * pitch * b.inertia.x * 10.0;

        // Wings carry the weight once flying; the airframe resists sideslip
        // and sinking through its plates.
        let mut accel = Vec3::Y * (1.0 - z.y.abs()) * GRAVITY * remap(4.44444, 15.5556, 0.0, 1.0, b.vel.length());
        let plates = 900.0 * remap(11.1111, 20.0, 0.01, 1.0, fwd);
        b.plate(Vec3::Y, plates);
        b.plate(Vec3::X, plates);

        // Engine: constant power (thrust falls with speed), cut to a quarter
        // while braking and restored over a second after; quadratic drag.
        if input.brake { self.since_brake = 0.0; self.since_release = 0.0; } else { self.since_brake += DT; self.since_release += DT; }
        let held_back = if input.brake { 1.0 } else { remap(0.25, 1.0, 1.0, 0.0, self.since_brake) };
        let thrust = (200.0 - 150.0 * held_back) * if self.on_water { 0.5 } else { 1.0 };
        accel += z * (p.power * thrust / fwd.max(1.0));
        let fast = remap(26.6667, 35.5556, 0.0, 1.0, fwd);
        let base = (p.drag + (1.0 - p.drag) * fast) * 0.02;
        let mut drag = base;
        if self.on_water { drag += p.water_drag * (1.0 - fast) * base; }
        if input.brake { drag += (p.brake_drag + (p.low_speed_brake_drag - p.brake_drag) * fast) * base; }
        accel -= b.vel * (drag * b.vel.length()).min(25.0);
        if self.on_water {
            let s = b.vel.length();
            accel += b.vel * (s.min(0.888889) * -0.9375 / s.max(0.1));
            let side = Vec3::new(x.x, 0.0, x.z);
            accel += side * (b.vel.dot(side) * -10.0).clamp(-1.0, 1.0) * GRAVITY;
        }

        // Turbo: 2.5 s at five g along the nose, a one-second fade, then the
        // meter recharges over four seconds.
        self.turbo_time += DT;
        match self.turbo {
            0 => { self.meter = 1.0; if input.turbo { (self.turbo, self.turbo_time, self.since_release) = (1, 0.0, 10.0); } }
            1 if self.turbo_time > 2.5 => (self.turbo, self.turbo_time) = (2, 0.0),
            1 => self.meter = 1.0 - self.turbo_time * 0.4,
            2 => { self.meter = self.turbo_time * 0.25; if self.turbo_time > 1.0 { (self.turbo, self.turbo_time) = (3, 0.0); } }
            _ if self.turbo_time > 3.0 => (self.turbo, self.meter) = (0, 1.0),
            _ => self.meter = (self.turbo_time + 1.0) * 0.25,
        }
        if !input.brake {
            let ramp = remap(0.0, 1.0, 0.0, 1.0, self.since_release) * p.turbo * 49.05;
            match self.turbo {
                1 => accel += z * ramp,
                2 => { let f = remap(0.0, 1.0, 1.0, 0.0, self.turbo_time); accel += z * ramp * (1.0 - (1.0 - f) * (1.0 - f)); }
                _ => {}
            }
        }

        // Ceiling: climbing stalls out and the nose drops near 1372.5 m.
        let above = b.pos.y - 1372.5;
        accel.y += b.vel.y.max(0.0) * remap(-15.25, 0.0, 0.0, -10.0, above);
        b.torque += Vec3::NEG_Y.cross(z) * b.inertia.x * remap(-7.625, 0.0, 0.0, -30.0, above);
        b.accelerate(accel);
    }

    /// The barrel roll under way or ended within the last second.
    pub fn recent_roll(&self) -> f32 {
        if self.barrel != 0.0 { self.barrel } else if self.since_barrel < 1.0 { self.last_barrel } else { 0.0 }
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

/// Pedal glider drive (3DS `PGlider`): taps spin the crank, the crank pushes
/// towards a speed, and stamina limits how hard the rider can keep it up.
#[derive(Clone, Debug)]
pub struct Pedal {
    /// Crank speed, radians per second.
    pub crank: f32,
    pub stamina: f32,
    pub exhausted: bool,
    since_tap: f32,
}

impl Default for Pedal {
    fn default() -> Self { Self { crank: 0.0, stamina: 1.0, exhausted: false, since_tap: 9.0 } }
}

impl Pedal {
    pub fn step(&mut self, b: &mut Body, tap: bool) {
        self.since_tap += DT;
        if tap && !self.exhausted {
            // Pedalling faster than every 0.75 s tires the rider three times as much.
            self.stamina = (self.stamina - if self.since_tap < 0.75 { 0.0625 } else { 0.020625 }).max(0.0);
            if self.stamina < 0.075 { self.exhausted = true; }
            self.since_tap = 0.0;
        }
        if self.since_tap > 1.8 {
            self.stamina = (self.stamina + 0.00417084).min(1.0);
            if self.stamina > 0.325 { self.exhausted = false; }
        }
        let target = if self.since_tap < 1.0 && !self.exhausted { std::f32::consts::TAU } else { 0.0 };
        self.crank += (target - self.crank).clamp(-0.0524123, 0.104825);
        let z = b.rot * Vec3::Z;
        let push = (self.crank.max(0.0) * 160.0 * 0.444444 / std::f32::consts::TAU - b.vel.dot(z)) * 0.05;
        b.accelerate(z * push);
    }
}

/// Rocket belt (3DS `Rocket` flight state): jets along the tilted body axis,
/// a light throttle that eases sink, an air brake that holds position, and
/// attitude control that keeps the pilot upright and turns on the spot.
#[derive(Clone, Debug)]
pub struct Rocket { power: f32 }

/// Rocket controls: `tilt` > 0 leans the jets forward, `steer` > 0 turns right.
#[derive(Default, Clone, Copy)]
pub struct RocketInput { pub thrust: bool, pub light: bool, pub hover: bool, pub tilt: f32, pub steer: f32 }

impl Rocket {
    pub fn step(&self, b: &mut Body, input: RocketInput) {
        let (x, y, z) = b.axes();
        let mut accel = if input.hover {
            Vec3::Y * GRAVITY - b.vel / (b.vel.length().max(1.0) * DT)
        } else {
            -b.vel * 0.5
        };
        let jets = if input.thrust { 29.43 } else if input.light { remap(8.88889, -8.88889, 7.3575, 22.0725, b.vel.y.min(0.0) * (1.0 - input.tilt.abs())) } else { 0.0 };
        if jets > 0.0 {
            let lean = input.tilt.clamp(-1.0, 1.0) * std::f32::consts::FRAC_PI_2;
            let fade = remap(1372.0, 1373.0, 1.0, 0.0, b.pos.y);
            accel += (y * lean.cos() + z * lean.sin()) * jets * self.power * fade;
            if b.vel.y > 0.0 { accel.y -= b.vel.y * 5.0 * remap(1322.5, 1372.5, 0.0, 1.0, b.pos.y); }
        }
        if b.pos.y < 0.0 {
            accel.y += remap(0.0, 5.0, 0.0, 39.24, -b.pos.y);
            accel -= b.vel * remap(0.0, 0.25, 0.0, 0.5, -b.pos.y);
        }
        b.accelerate(accel);
        let roll = (x.y * -5.0 - b.ang.dot(z) * 4.0).clamp(-10.0, 10.0);
        let pitch = (z.y * 20.0 - b.ang.dot(x) * 4.0).clamp(-10.0, 10.0);
        let yaw = ((input.steer * 1.0472 - b.ang.dot(y)) * 8.0).clamp(-10.0, 10.0);
        b.torque += (z * roll * b.inertia.z + x * pitch * b.inertia.x + y * yaw * b.inertia.y) * 10.0;
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
    pedal: Pedal,
    rocket: Rocket,
    pub flight: Flight,
    params: PlaneParams,
    tapped: bool,
    /// Controls and state for animation and sound.
    pub drive: crate::motion::Drive,
    /// Poses after the previous and the latest physics step; the drawn
    /// transform blends between them so the chase camera does not judder.
    pose: [Transform; 2],
}

impl Vehicle {
    /// Pedal glider hearts left (of five).
    pub fn hearts(&self) -> u32 { if self.kind == Kind::PGlider { (self.pedal.stamina * 5.0).ceil() as u32 } else { 0 } }
    pub fn exhausted(&self) -> bool { self.kind == Kind::PGlider && self.pedal.exhausted }
}

pub fn plugin(app: &mut App) {
    if !pwr::available() { return; }
    let thermals = std::fs::read(crate::asset_dir().join("pwr/data/wuhu_free_glider/thermal.val.json")).ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok()).map_or_else(Thermals::default, |t| Thermals::load(&t));
    app.insert_resource(thermals)
        .add_systems(PreUpdate, shoulders.after(crate::input::read))
        .add_systems(Update, board.before(crate::plane::board).run_if(crate::menu::playing))
        .add_systems(FixedUpdate, fly.run_if(crate::menu::playing.and_then(in_state(Mode::Fly))))
        .add_systems(Update, (smooth, camera.run_if(crate::menu::playing.and_then(in_state(Mode::Fly)))).chain())
        .add_systems(OnEnter(AppState::Play), (spawn_thermals, spawn_gauge))
        .add_systems(Update, gauge)
        .add_systems(Update, show_thermals)
        .add_systems(OnExit(Mode::Fly), despawn)
        .add_systems(OnExit(AppState::Play), despawn);
}

/// While a 3DS plane flies, its shoulder buttons lean instead of opening the
/// i-point list or toggling the speedometer (they share LB / RB by default).
fn shoulders(mut c: ResMut<Controls>, vehicles: Query<&Vehicle>) {
    if (c.lean_left || c.lean_right) && vehicles.iter().any(|v| v.kind.plane()) {
        c.record = false;
        c.speedometer = false;
    }
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
            let Some(kind) = selected(&save).filter(|k| !k.wii_physics(&save)) else { return };
            // The Wii plane's boarding code must not also react.
            c.board = false;
            // Every 3DS vehicle body: 454.5 kg, inertia 422.8 on each axis.
            let mut body = Body::new(454.5, Vec3::splat(422.799));
            // Gliders launch above the player, rocket belts lift off where they stand.
            let feet = player.1.translation;
            // Planes start at cruise, 60 m up (as the Wii plane does).
            let (height, speed) = match kind { Kind::Glider => (80.0, 11.1111), Kind::PGlider => (50.0, 11.1111), Kind::Plane => (60.0, 21.0), Kind::Jet => (60.0, 30.0), _ => (1.0, 0.0) };
            body.pos = Vec3::new(feet.x, feet.y, -feet.z) / SCALE + Vec3::Y * height;
            body.rot = Quat::from_rotation_y(-rig.yaw);
            body.vel = body.rot * Vec3::Z * speed;
            let t = body.transform();
            let rocket = Rocket { power: if kind == Kind::SuperRocket { 1.3 } else { 1.0 } };
            commands.spawn((Vehicle { kind, body, glider: Glider::default(), pedal: Pedal::default(), rocket, flight: Flight::default(), params: PlaneParams::load(kind), tapped: false, drive: default(), pose: [t; 2] }, Flying, t, Visibility::default())).with_children(|v| {
                v.spawn((WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(format!("pwr/models/{}_lod1.glb", kind.container())))), Rider(kind), crate::motion::Parts::new(kind)))
                    .observe(attach_pilot).observe(crate::motion::collect);
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
    mut q: Query<&mut Vehicle>,
    mut player: Single<(&mut Player, &mut Transform, &mut Visibility), Without<Vehicle>>,
    mut sounds: MessageWriter<crate::sound::Sound>,
) {
    let Ok(mut v) = q.single_mut() else { return };
    v.pose[0] = v.pose[1];
    let stick = c.stick * save.flight_sensitivity;
    // The Wii controls push the stick up to dive; the 3DS pitch input raises the nose.
    let forward = if save.invert_pitch { -stick.y } else { stick.y };
    let input = Input { pitch: -forward, steer: stick.x, brake: c.slow > 0.5 || c.cut };
    let v = &mut *v;
    let from = pwr::world(v.body.pos.to_array());
    let pedalling = c.boost > 0.5 || c.shoot;
    let tap = pedalling && !v.tapped;
    v.tapped = pedalling;
    let lift = thermals.lift(v.body.pos);
    // Eased controls drive the moving parts.
    let ease = |from: f32, to: f32| from + (to - from) * 0.12;
    let thrust = if c.boost > 0.5 { 1.0 } else if c.shoot { 0.5 } else { 0.0 };
    let rocket = matches!(v.kind, Kind::Rocket | Kind::SuperRocket);
    let thrust = if v.kind.plane() { if v.flight.turbo == 1 { 1.0 } else if input.brake { 0.2 } else { 0.6 } } else { thrust };
    v.drive = crate::motion::Drive {
        steer: if v.kind.plane() { v.flight.roll } else { ease(v.drive.steer, stick.x.clamp(-1.0, 1.0)) },
        pitch: if v.kind.plane() { v.flight.pitch } else { ease(v.drive.pitch, if rocket { forward } else { input.pitch }.clamp(-1.0, 1.0)) },
        thrust: if rocket || v.kind.plane() { ease(v.drive.thrust, thrust) } else { 0.0 },
        crank: v.pedal.crank,
        brake: input.brake,
        lift: if v.kind.gliding() { lift } else { 0.0 },
        speed: v.body.vel.length(),
    };
    match v.kind {
        Kind::Glider | Kind::PGlider => {
            if v.kind == Kind::PGlider { v.pedal.step(&mut v.body, tap); }
            v.glider.step(&mut v.body, input, lift);
        }
        Kind::Rocket | Kind::SuperRocket => {
            let r = RocketInput { thrust: c.boost > 0.5, light: c.shoot, hover: c.slow > 0.5, tilt: forward, steer: stick.x };
            v.rocket.step(&mut v.body, r);
        }
        Kind::Plane | Kind::Jet => {
            let p = PlaneInput { steer: stick.x.clamp(-1.0, 1.0), pitch: input.pitch.clamp(-1.0, 1.0), lean_left: c.lean_left, lean_right: c.lean_right, brake: c.slow > 0.5, turbo: c.boost > 0.5 };
            v.flight.step(&mut v.body, p, v.params);
        }
    }
    v.body.integrate();
    let to = pwr::world(v.body.pos.to_array());
    // Ground contact: land gently, otherwise the pilot falls off (3DS limits:
    // 13.3 m/s into the ground or 26.7 m/s overall).
    let travel = to - from;
    // Gliders land on their wheels/feet (`ground_dy`); rocket belts touch with their boots.
    let reach = if v.kind.gliding() { 0.95 } else if v.kind.plane() { 0.6 } else { 0.05 } * SCALE;
    v.flight.on_water = false;
    let hit = col.raycast(from, travel.normalize_or_zero(), travel.length() + reach);
    let below = col.ground(to + Vec3::Y * reach, 0.0, 2.0 * reach);
    if let Some(h) = hit.or(below) {
        let normal = Vec3::new(h.n.x, h.n.y, -h.n.z) * h.n.y.signum();
        let water = h.flags & 0x1f == 6;
        // Rocket belts touch down at up to 15.6 m/s, gliders at 13.3 m/s.
        let limit = if v.kind.gliding() { 13.3333 } else { 15.5556 };
        let gentle = !water && v.body.vel.dot(normal) > -limit && v.body.vel.length() < 26.6667 && h.n.y.abs() > 0.6;
        // The planes float on water and can take off again.
        if v.kind.plane() && water && v.body.vel.y > -13.3333 && h.n.y.abs() > 0.6 {
            v.flight.on_water = true;
            v.body.pos.y = v.body.pos.y.max(h.pos.y / SCALE);
            v.body.vel.y = v.body.vel.y.max(0.0);
            v.pose[1] = v.body.transform();
            return;
        }
        // A rocket belt stands on the ground and can lift off again.
        if gentle && !v.kind.gliding() && !v.kind.plane() {
            if let Some(k) = h.platform {
                let ride = col.platforms[k].carry(h.pos, DT) / SCALE;
                v.body.pos += Vec3::new(ride.x, ride.y, -ride.z);
            }
            v.body.pos.y = v.body.pos.y.max(h.pos.y / SCALE + 0.05);
            v.body.vel = Vec3::new(v.body.vel.x * 0.8, v.body.vel.y.max(0.0), v.body.vel.z * 0.8);
            v.pose[1] = v.body.transform();
            return;
        }
        step_off(&mut *player, &v.body, !gentle);
        if gentle { player.1.translation = h.pos + Vec3::Y * 2.0; } else { sounds.write(crate::sound::Sound::effect(crate::motion::crash(v.kind), None)); }
        next.set(Mode::Walk);
        return;
    }
    v.pose[1] = v.body.transform();
}

/// Places the vehicle between its last two physics poses.
fn smooth(fixed: Res<Time<Fixed>>, mut q: Query<(&Vehicle, &mut Transform)>) {
    let f = fixed.overstep_fraction();
    for (v, mut t) in &mut q {
        let [a, b] = v.pose;
        t.translation = a.translation.lerp(b.translation, f);
        t.rotation = a.rotation.slerp(b.rotation, f);
    }
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
    // Gliders frame their travel; rocket belts look where the pilot faces.
    let travel = Vec3::new(v.body.vel.x, 0.0, -v.body.vel.z);
    let nose = t.rotation * Vec3::NEG_Z;
    let flat = if travel.length() > 1.0 && (v.kind.gliding() || v.kind.plane()) { travel } else { nose.with_y(0.0) };
    if *heading == Vec3::ZERO { *heading = flat.normalize_or(Vec3::NEG_Z); }
    *heading = heading.lerp(flat.normalize_or(*heading), 1.0 - (-3.0 * dt).exp()).normalize_or(Vec3::NEG_Z);
    let yaw = (-heading.x).atan2(-heading.z);
    let fwd = Quat::from_euler(EulerRot::YXZ, yaw + look.x, -look.y - 0.12, 0.0) * Vec3::NEG_Z;
    let (chase, height) = v.kind.camera();
    let target = t.translation + Vec3::Y * (height + 1.0) * SCALE;
    let dir = (-fwd).normalize();
    let mut dist = chase * SCALE;
    if let Some(h) = col.raycast(target, dir, dist) { dist = (h.t - 10.0).max(20.0); }
    let want = target + dir * dist;
    cam.translation = cam.translation.lerp(want, 1.0 - (-10.0 * dt).exp());
    cam.look_at(target + fwd * 3.0 * SCALE, Vec3::Y);
}

#[derive(Component)]
struct ThermalColumn;

#[derive(Component)]
struct Gauge;

fn spawn_gauge(mut commands: Commands, font: Res<crate::lang::UiFont>) {
    commands.spawn((
        Node { position_type: PositionType::Absolute, top: px(20), left: percent(4), padding: UiRect::axes(px(16), px(6)), border: UiRect::all(px(3)), border_radius: BorderRadius::all(px(16)), ..default() },
        BackgroundColor(crate::menu::PANEL), BorderColor::all(crate::menu::OUTLINE),
        Text::new(""), TextFont::from(font.0.clone()).with_font_size(FontSize::Px(24.0)), TextColor(crate::menu::INK),
        Visibility::Hidden, Gauge, DespawnOnExit(AppState::Play),
    ));
}

/// Pedal glider stamina as five hearts (as the 3DS game shows it) and the
/// Turbo Jet's turbo meter.
fn gauge(vehicles: Query<&Vehicle>, planes: Query<&crate::plane::Plane>, mut q: Single<(&mut Text, &mut Visibility, &mut TextColor), With<Gauge>>) {
    let (text, vis, color) = &mut *q;
    let bar = |filled: usize, full: char, empty: char| (0..5).map(|i| if i < filled { full } else { empty }).collect::<String>();
    let shown = if let Some(v) = vehicles.iter().find(|v| v.kind == Kind::PGlider) {
        **color = TextColor(if v.pedal.exhausted { Color::srgb(0.6, 0.6, 0.65) } else { Color::srgb(0.85, 0.15, 0.3) });
        Some(bar((v.pedal.stamina * 5.0).ceil() as usize, '♥', '♡'))
    } else if let Some(v) = vehicles.iter().find(|v| v.kind.plane()) {
        **color = TextColor(crate::menu::INK);
        Some(format!("TURBO {}", bar((v.flight.meter * 5.0).ceil() as usize, '■', '□')))
    } else if let Some(p) = planes.iter().find(|p| p.jet) {
        **color = TextColor(crate::menu::INK);
        Some(format!("TURBO {}", bar((p.turbo * 5.0).ceil() as usize, '■', '□')))
    } else {
        None
    };
    **vis = if shown.is_some() { Visibility::Inherited } else { Visibility::Hidden };
    if let Some(s) = shown { text.set_if_neq(Text::new(s)); }
}

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
    let gliding = vehicles.iter().any(|v| v.kind.gliding());
    for mut vis in &mut columns {
        vis.set_if_neq(if gliding { Visibility::Inherited } else { Visibility::Hidden });
    }
}

#[derive(Component)]
struct Rider(Kind);

/// The explorer rides along: prone under the hang glider, seated on the pedal
/// glider and standing in the rocket belt's harness. Model space here is the
/// 3DS model's (metres, facing -Z), so the figure is scaled to about 1.3 m.
fn attach_pilot(ready: On<WorldInstanceReady>, riders: Query<&Rider>, children: Query<&Children>, names: Query<&Name>, mut meshes: ResMut<Assets<Mesh>>, mut materials: ResMut<Assets<crate::gx::GxMaterial>>, globals: Res<crate::gx::GxGlobals>, mut commands: Commands) {
    let Ok(rider) = riders.get(ready.entity) else { return };
    use std::f32::consts::{FRAC_PI_2, PI};
    let (joint, pose, seated) = match rider.0 {
        Kind::Glider => ("harness", Transform::from_xyz(0.0, -0.35, 0.6).with_rotation(Quat::from_rotation_x(-FRAC_PI_2) * Quat::from_rotation_y(PI)), false),
        Kind::PGlider => ("butt", Transform::from_xyz(0.0, -0.4, 0.0).with_rotation(Quat::from_rotation_y(PI)), true),
        Kind::Rocket | Kind::SuperRocket => ("harness", Transform::from_xyz(0.0, -0.6, -0.15).with_rotation(Quat::from_rotation_y(PI)), false),
        _ => return,
    };
    let Some(at) = children.iter_descendants(ready.entity).find(|&e| names.get(e).is_ok_and(|n| n.as_str() == joint)) else { return };
    commands.entity(at).with_children(|c| {
        c.spawn((Name::new("Pilot"), pose.with_scale(Vec3::splat(0.025)), Visibility::default(), crate::motion::Pilot(pose)))
            .with_children(|c| crate::player::body(c, &mut meshes, &mut materials, &globals, seated));
    });
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

    fn plane(speed: f32) -> (Flight, Body) {
        let mut b = Body::new(454.5, Vec3::splat(422.799));
        b.pos = Vec3::new(0.0, 300.0, 0.0);
        b.vel = Vec3::Z * speed;
        (Flight::default(), b)
    }

    fn cruise(f: &mut Flight, b: &mut Body, input: PlaneInput, p: PlaneParams, seconds: f32) {
        for _ in 0..(seconds / DT) as usize { f.step(b, input, p); b.integrate(); }
    }

    #[test]
    fn hands_off_plane_cruises_straight_and_level() {
        let (mut f, mut b) = plane(21.0);
        cruise(&mut f, &mut b, PlaneInput::default(), PlaneParams::default(), 20.0);
        let (_, y, z) = b.axes();
        assert!((b.pos.y - 300.0).abs() < 15.0, "altitude {}", b.pos.y);
        assert!(b.vel.length() > 17.0 && b.vel.length() < 26.0, "speed {}", b.vel.length());
        assert!(y.y > 0.95 && z.x.abs() < 0.05, "attitude up {y} nose {z}");
        // The jet's lower drag makes it faster at the same power.
        let (mut f, mut j) = plane(21.0);
        cruise(&mut f, &mut j, PlaneInput::default(), PlaneParams { drag: 0.25, ..default() }, 20.0);
        assert!(j.vel.length() > b.vel.length() + 5.0, "jet {} plane {}", j.vel.length(), b.vel.length());
    }

    #[test]
    fn plane_banks_and_turns_the_way_of_the_stick_and_climbs_with_it() {
        let (mut f, mut b) = plane(21.0);
        cruise(&mut f, &mut b, PlaneInput { steer: 1.0, ..default() }, PlaneParams::default(), 4.0);
        let (x, _, z) = b.axes();
        assert!(z.x > 0.3, "heading {z}");
        assert!(x.y < -0.5, "right wing down {x}");
        let (mut f, mut b) = plane(21.0);
        cruise(&mut f, &mut b, PlaneInput { pitch: 1.0, ..default() }, PlaneParams::default(), 4.0);
        assert!(b.pos.y > 315.0, "climbed to {}", b.pos.y);
    }

    #[test]
    fn double_tapped_shoulder_barrel_rolls_through_upside_down() {
        let (mut f, mut b) = plane(21.0);
        let p = PlaneParams::default();
        let right = PlaneInput { lean_right: true, ..default() };
        for input in [right, PlaneInput::default(), right] { f.step(&mut b, input, p); b.integrate(); }
        assert_eq!(f.barrel, 1.0);
        let mut lowest: f32 = 1.0;
        for _ in 0..90 {
            f.step(&mut b, PlaneInput::default(), p);
            b.integrate();
            lowest = lowest.min(b.axes().1.y);
        }
        assert!(lowest < -0.5, "never upside down ({lowest})");
        assert_eq!((f.barrel, f.last_barrel), (0.0, 1.0));
        cruise(&mut f, &mut b, PlaneInput::default(), p, 2.0);
        assert!(b.axes().1.y > 0.9, "upright again");
    }

    #[test]
    fn both_shoulders_fly_inverted() {
        let (mut f, mut b) = plane(21.0);
        cruise(&mut f, &mut b, PlaneInput { lean_left: true, lean_right: true, ..default() }, PlaneParams::default(), 3.0);
        assert!(b.axes().1.y < -0.7, "up {}", b.axes().1);
        let (mut f, mut b) = plane(21.0);
        cruise(&mut f, &mut b, PlaneInput { lean_left: true, ..default() }, PlaneParams::default(), 2.0);
        assert!(b.axes().0.y > 0.8, "knife edge, right wing up {}", b.axes().0);
    }

    #[test]
    fn turbo_drains_then_refills() {
        let (mut f, mut b) = plane(21.0);
        let p = PlaneParams::default();
        cruise(&mut f, &mut b, PlaneInput { turbo: true, ..default() }, p, 1.0);
        assert_eq!(f.turbo, 1);
        assert!(f.meter < 0.7 && b.vel.length() > 30.0, "meter {} speed {}", f.meter, b.vel.length());
        cruise(&mut f, &mut b, PlaneInput::default(), p, 2.0);
        assert!(f.turbo >= 2 && f.meter < 0.5, "state {} meter {}", f.turbo, f.meter);
        cruise(&mut f, &mut b, PlaneInput::default(), p, 4.5);
        assert_eq!((f.turbo, f.meter), (0, 1.0));
    }

    fn rocket() -> (Rocket, Body) {
        let mut b = Body::new(454.5, Vec3::splat(422.799));
        b.pos = Vec3::new(0.0, 100.0, 0.0);
        (Rocket { power: 1.0 }, b)
    }

    fn burn(r: &Rocket, b: &mut Body, input: RocketInput, seconds: f32) {
        for _ in 0..(seconds / DT) as usize { r.step(b, input); b.integrate(); }
    }

    #[test]
    fn rocket_jets_climb_and_leaning_moves_forward() {
        let (r, mut b) = rocket();
        burn(&r, &mut b, RocketInput { thrust: true, ..default() }, 2.0);
        assert!(b.pos.y > 110.0, "climbed to {}", b.pos.y);
        let (r, mut b) = rocket();
        burn(&r, &mut b, RocketInput { thrust: true, tilt: 0.5, ..default() }, 2.0);
        assert!(b.pos.z > 5.0, "moved forward {}", b.pos.z);
        assert!((b.rot * Vec3::Y).y > 0.95, "pilot stays upright");
    }

    #[test]
    fn rocket_air_brake_holds_position() {
        let (r, mut b) = rocket();
        b.vel = Vec3::new(10.0, -10.0, 0.0);
        burn(&r, &mut b, RocketInput { hover: true, ..default() }, 2.0);
        assert!(b.vel.length() < 0.5, "still moving at {}", b.vel);
    }

    #[test]
    fn light_throttle_slows_a_fall() {
        let (r, mut b) = rocket();
        burn(&r, &mut b, RocketInput { light: true, ..default() }, 3.0);
        let (r2, mut free) = rocket();
        burn(&r2, &mut free, RocketInput::default(), 3.0);
        assert!(b.pos.y > free.pos.y + 10.0, "light {} vs free {}", b.pos.y, free.pos.y);
    }

    #[test]
    fn pedalling_pushes_forward_and_tires_the_rider() {
        let (mut g, mut b) = glider();
        let mut p = Pedal::default();
        let mut slow = b.clone();
        let (mut g2, mut p2) = (g.clone(), Pedal::default());
        for i in 0..(10.0 / DT) as usize {
            let tap = i % 50 == 0;
            p.step(&mut b, tap); g.step(&mut b, Input::default(), 0.0); b.integrate();
            p2.step(&mut slow, false); g2.step(&mut slow, Input::default(), 0.0); slow.integrate();
        }
        assert!(b.pos.y > slow.pos.y + 5.0 || b.vel.length() > slow.vel.length() + 2.0, "pedalled {} vs coasting {}", b.pos, slow.pos);
        assert!(p.stamina < 0.8, "stamina {}", p.stamina);
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
