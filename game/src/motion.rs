//! Moving parts and sounds of the Pilotwings Resort vehicles. The 3DS game
//! turns these bones from code every frame (its `.can` clips only hold the
//! landing and menu scenes), so here they follow the controls: control
//! surfaces, the pedal glider's crank and propellers, the rocket nozzles and
//! flames, and the pilot's lean and pedalling.

use crate::plane::Plane;
use crate::save::Save;
use crate::sound::{self, Sound, Sounds};
use crate::vehicles::{self, Kind, Vehicle};
use bevy::audio::{AudioSinkPlayback, Volume};
use bevy::prelude::*;
use bevy::world_serialization::WorldInstanceReady;
use std::collections::HashMap;
use std::f32::consts::{PI, TAU};

/// Named nodes of an add-on vehicle model (lower case), plus the flames and
/// propellers mounted on them.
#[derive(Component)]
pub struct Parts { kind: Kind, nodes: HashMap<String, Entity> }

impl Parts {
    pub fn new(kind: Kind) -> Self { Self { kind, nodes: HashMap::new() } }
}

/// The figure riding a vehicle, and its pose at rest.
#[derive(Component)]
pub struct Pilot(pub Transform);

/// A seated leg's hip pivot: -1 left, 1 right.
#[derive(Component)]
pub struct Leg(pub f32);

/// What the vehicle is doing, for its animation and sound.
#[derive(Default, Clone, Copy)]
pub struct Drive {
    /// -1..1, right positive.
    pub steer: f32,
    /// -1..1, nose up positive.
    pub pitch: f32,
    /// Jet output 0..1.
    pub thrust: f32,
    /// Pedal glider crank speed, radians per second.
    pub crank: f32,
    pub brake: bool,
    /// Thermal lift felt, 0..1.
    pub lift: f32,
    /// Metres per second.
    pub speed: f32,
}

pub fn plugin(app: &mut App) {
    if !crate::pwr::available() { return; }
    app.add_systems(Update, (animate, loops, cues).run_if(in_state(crate::AppState::Play)));
}

/// Remembers the model's nodes and mounts flames and propellers.
pub fn collect(ready: On<WorldInstanceReady>, mut parts: Query<&mut Parts>, children: Query<&Children>, names: Query<&Name>, assets: Res<AssetServer>, mut commands: Commands) {
    let Ok(mut p) = parts.get_mut(ready.entity) else { return };
    for e in children.iter_descendants(ready.entity) {
        if let Ok(n) = names.get(e) { p.nodes.insert(n.as_str().to_ascii_lowercase(), e); }
    }
    let mounts: &[(&str, &str, &str)] = match p.kind {
        Kind::Rocket | Kind::SuperRocket => &[("jet_1", "rocket_jet_fx", "flame_1"), ("jet_2", "rocket_jet_fx", "flame_2")],
        Kind::Jet => &[("jet_1", "jet_jet_fx", "flame_1"), ("jet_2", "jet_jet_fx", "flame_2")],
        Kind::PGlider => &[("left_prop", "pglider_prop_slow", "prop_l"), ("right_prop", "pglider_prop_slow", "prop_r")],
        Kind::Plane => &[("prop", "plane_prop_fast", "prop_main")],
        _ => &[],
    };
    for (joint, model, name) in mounts {
        let Some(&at) = p.nodes.get(*joint) else { continue };
        let scene = assets.load(GltfAssetLabel::Scene(0).from_asset(format!("pwr/models/{model}.glb")));
        let id = commands.spawn((WorldAssetRoot(scene), Transform::default(), Visibility::default(), ChildOf(at))).id();
        p.nodes.insert(name.to_string(), id);
    }
}

/// The flying add-on vehicle's drive state, if one is flying.
fn drive(vehicles: &Query<&Vehicle>, planes: &Query<&Plane>) -> Option<Drive> {
    if let Ok(v) = vehicles.single() { return Some(v.drive); }
    planes.single().ok().map(|p| Drive {
        steer: p.ctrl.x,
        pitch: -p.ctrl.y,
        thrust: if p.jet { (p.throttle * 0.6 + if p.burning { 0.6 } else { 0.0 }).min(1.0) } else { p.throttle },
        speed: p.vel.length() * 60.0 / crate::pwr::SCALE,
        ..default()
    })
}

/// Part rotations for one frame. Model space: right +X, up +Y, forward -Z;
/// hinged surfaces trail behind their pivots.
fn pose(kind: Kind, d: &Drive, phase: f32) -> Vec<(&'static str, Quat)> {
    let x = Quat::from_rotation_x;
    let y = Quat::from_rotation_y;
    match kind {
        // Ailerons (the jet's are elevons), elevator and rudder; a positive
        // X rotation lowers a trailing edge.
        Kind::Plane | Kind::Jet => vec![
            ("left_flap", x(0.35 * d.steer - if kind == Kind::Jet { 0.25 * d.pitch } else { 0.0 })),
            ("right_flap", x(-0.35 * d.steer - if kind == Kind::Jet { 0.25 * d.pitch } else { 0.0 })),
            ("tail", x(-0.35 * d.pitch)),
            ("rudder", y(0.3 * d.steer)),
        ],
        // The pilot hangs from the keel and shifts weight to steer.
        Kind::Glider => vec![("harness_pivot", Quat::from_rotation_z(0.3 * d.steer) * x(-0.2 * d.pitch))],
        // Pedals stay level on the turning crank; the bars turn the nose wheels.
        Kind::PGlider => vec![
            ("crank", x(-phase)),
            ("left_pedal", x(phase)),
            ("right_pedal", x(phase)),
            ("rudder", y(0.4 * d.steer)),
            ("fork_pivot", y(-0.35 * d.steer)),
        ],
        // Nozzles swing back as the pilot leans and split to turn on the spot.
        Kind::Rocket | Kind::SuperRocket => vec![
            ("left_jet", x(-0.5 * d.pitch - 0.3 * d.steer)),
            ("right_jet", x(-0.5 * d.pitch + 0.3 * d.steer)),
        ],
    }
}

#[allow(clippy::too_many_arguments)]
fn animate(
    time: Res<Time>,
    mut phase: Local<f32>,
    mut spin: Local<f32>,
    mut prop: Local<f32>,
    vehicles: Query<&Vehicle>,
    planes: Query<&Plane>,
    models: Query<&Parts>,
    mut nodes: Query<&mut Transform, (Without<Pilot>, Without<Leg>)>,
    mut pilots: Query<(&Pilot, &mut Transform), Without<Leg>>,
    mut legs: Query<(&Leg, &mut Transform), Without<Pilot>>,
) {
    let Some(d) = drive(&vehicles, &planes) else { return };
    let dt = time.delta_secs();
    *phase = (*phase + d.crank * dt) % TAU;
    *spin = (*spin + (4.0 + d.crank * 3.0) * dt) % TAU;
    *prop = (*prop + (20.0 + 40.0 * d.thrust) * dt) % TAU;
    for p in &models {
        let mut set = |name: &str, f: &dyn Fn(&mut Transform)| {
            if let Some(mut t) = p.nodes.get(name).and_then(|&e| nodes.get_mut(e).ok()) { f(&mut t); }
        };
        for (name, q) in pose(p.kind, &d, *phase) { set(name, &|t| t.rotation = q); }
        // Flames grow with the jets; propellers turn with the crank.
        let flame = if d.thrust > 0.01 { 0.3 + 0.7 * d.thrust } else { 0.0 };
        let stretch = if p.kind == Kind::Jet { Vec3::new(1.0, 1.0, flame) } else { Vec3::new(1.0, flame, 1.0) };
        for f in ["flame_1", "flame_2"] { set(f, &|t| t.scale = stretch.max(Vec3::splat(1e-3))); }
        set("prop_l", &|t| t.rotation = Quat::from_rotation_z(*spin));
        set("prop_r", &|t| t.rotation = Quat::from_rotation_z(-*spin));
        set("prop_main", &|t| t.rotation = Quat::from_rotation_z(*prop));
        // The pilot leans into turns (the figure faces the vehicle's -Z).
        if matches!(p.kind, Kind::PGlider | Kind::Rocket | Kind::SuperRocket) {
            for (rest, mut t) in &mut pilots { t.rotation = rest.0.rotation * Quat::from_rotation_z(0.25 * d.steer); }
        }
        if p.kind == Kind::PGlider {
            for (leg, mut t) in &mut legs { t.rotation = Quat::from_rotation_x(0.45 * (*phase + if leg.0 > 0.0 { PI } else { 0.0 }).sin()); }
        }
    }
}

/// Looping vehicle sounds, as children of the vehicle.
#[derive(Component)]
struct Loop(&'static str);

fn loop_names(kind: Kind) -> &'static [&'static str] {
    match kind {
        Kind::Glider => &["pwr/glider_flutter", "pwr/race_thermalwind"],
        Kind::PGlider => &["pwr/glider_flutter", "pwr/pglider_bicyclefreerun", "pwr/race_thermalwind"],
        Kind::Rocket => &["pwr/rocket_jet", "pwr/rocket_wind"],
        Kind::SuperRocket => &["pwr/superrocket_jet", "pwr/rocket_wind"],
        Kind::Plane => &["pwr/plane_engine"],
        Kind::Jet => &["pwr/jet_engine"],
    }
}

/// Volume and playback rate of a loop.
fn mix(name: &str, d: &Drive) -> (f32, f32) {
    let wind = ((d.speed - 4.0) / 30.0).clamp(0.0, 1.0);
    let pedal = (d.crank / TAU).clamp(0.0, 1.0);
    match name {
        "pwr/glider_flutter" | "pwr/rocket_wind" => (0.05 + 0.45 * wind, 0.8 + 0.5 * wind),
        "pwr/race_thermalwind" => (0.4 * d.lift.clamp(0.0, 1.0), 1.0),
        "pwr/pglider_bicyclefreerun" => (0.5 * pedal, 0.6 + 0.6 * pedal),
        _ => (0.15 + 0.5 * d.thrust, 0.85 + 0.3 * d.thrust),
    }
}

fn loops(
    mut commands: Commands, assets: Res<AssetServer>, sounds: Res<Sounds>, menu: Res<crate::menu::Menu>,
    new: Query<(Entity, &Vehicle), Added<Vehicle>>, vehicles: Query<&Vehicle>,
    mut sinks: Query<(&Loop, &mut AudioSink)>,
) {
    for (e, v) in &new {
        for &name in loop_names(v.kind).iter().filter(|n| sounds.has(n)) {
            commands.entity(e).with_child((AudioPlayer::new(assets.load(sound::path(name))), PlaybackSettings::LOOP.with_volume(Volume::SILENT), Loop(name)));
        }
    }
    let Ok(v) = vehicles.single() else { return };
    for (l, mut sink) in &mut sinks {
        let (gain, rate) = mix(l.0, &v.drive);
        sink.set_volume(Volume::Linear(gain));
        sink.set_speed(rate);
        if menu.open { sink.pause(); } else { sink.play(); }
    }
}

/// One-shot cues on state changes: braking, hearts, exhaustion and turbo.
fn cues(save: Res<Save>, vehicles: Query<&Vehicle>, planes: Query<&Plane>, mut last: Local<Option<(bool, u32, bool, bool, f32, bool)>>, mut out: MessageWriter<Sound>) {
    let selected = vehicles::selected(&save);
    let now = if let Ok(v) = vehicles.single() {
        let f = &v.flight;
        if v.kind.plane() { (false, 0, false, f.turbo == 1, f.meter, f.barrel != 0.0) } else { (v.drive.brake, v.hearts(), v.exhausted(), false, 1.0, false) }
    } else if let (Ok(p), Some(_)) = (planes.single(), selected) {
        (false, 0, false, p.burning, if p.jet { p.turbo } else { 1.0 }, false)
    } else {
        *last = None;
        return;
    };
    let Some(before) = last.replace(now) else { return };
    let kind = vehicles.single().map(|v| v.kind).ok();
    if now.0 && !before.0 && kind == Some(Kind::Glider) { out.write(Sound::effect("pwr/glider_brake", None)); }
    if now.1 < before.1 { out.write(Sound::effect("pwr/race_loseheart", None)); }
    if now.1 > before.1 { out.write(Sound::effect("pwr/race_gainheart", None)); }
    if now.2 && !before.2 { out.write(Sound::effect("pwr/pglider_overheated", None)); }
    if now.3 && !before.3 { out.write(Sound::effect("pwr/plane_turbo", None)); }
    if now.4 >= 1.0 && before.4 < 1.0 { out.write(Sound::effect(if selected == Some(Kind::Jet) { "pwr/jet_turbo_recharge" } else { "pwr/plane_turbo_recharge" }, None)); }
    if now.5 && !before.5 { out.write(Sound::effect(if selected == Some(Kind::Jet) { "pwr/jet_pln_twiststart" } else { "pwr/plane_pln_twiststart" }, None)); }
}

/// The crash sound for a vehicle thrown off by a hard landing.
pub fn crash(kind: Kind) -> &'static str {
    match kind {
        Kind::Glider => "pwr/glider_crash",
        Kind::PGlider => "pwr/pglider_crash",
        Kind::Rocket => "pwr/rocket_crash",
        Kind::SuperRocket => "pwr/superrocket_crash",
        Kind::Plane => "pwr/plane_crash",
        Kind::Jet => "pwr/jet_crash",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn surfaces_follow_the_stick_and_pedals_stay_level() {
        let right = Drive { steer: 1.0, ..default() };
        let p = pose(Kind::Plane, &right, 0.0);
        let get = |p: &[(&str, Quat)], n: &str| p.iter().find(|x| x.0 == n).unwrap().1;
        // Rolling right lowers the left aileron's trailing edge (+Z behind the hinge).
        assert!((get(&p, "left_flap") * Vec3::Z).y < -0.1 && (get(&p, "right_flap") * Vec3::Z).y > 0.1);
        let g = pose(Kind::PGlider, &Drive::default(), 1.0);
        let level = get(&g, "crank") * get(&g, "left_pedal");
        assert!(level.angle_between(Quat::IDENTITY) < 1e-5);
        // The glider pilot swings to the inside of a right turn.
        let h = pose(Kind::Glider, &right, 0.0);
        assert!((get(&h, "harness_pivot") * Vec3::NEG_Y).x > 0.1);
    }
}
