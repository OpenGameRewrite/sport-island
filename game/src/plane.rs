//! Island Flyover plane: flight model, camera, Balloon Popper and flares.
//!
//! The flight model follows the structure of the original: a rigid body pushed
//! by tail forces (steering), engine thrust, speed-dependent lift and per-axis
//! drag, all in game units per 60 Hz frame. Constants are the original values.

use crate::collide::Collision;
use crate::input::Controls;
use crate::player::{CamRig, Player};
use crate::save::Save;
use crate::world::MainCamera;
use crate::{AppState, Mode};
use bevy::prelude::*;
use bevy::world_serialization::WorldInstanceReady;

// Steering forces applied 8 units behind the centre (rudder / elevator).
const TAIL_ARM: f32 = 8.0;
const RUDDER: f32 = 0.0012;
const ELEVATOR: f32 = 0.0024;
// Engine thrust per unit throttle, and the boost / slow-down adjustments.
const THRUST: f32 = 0.035;
const BOOST: f32 = 0.03;
const SLOW: f32 = -0.02;
// Local-axis drag (x, y, z) while stalled and while flying.
const DRAG_STALL: Vec3 = Vec3::new(0.5, 0.5, 0.01);
const DRAG_FLY: Vec3 = Vec3::new(0.015, 0.015, 0.005);
const DRAG_GAIN: f32 = 0.995;
const STALL_GRAVITY: f32 = 0.1;
const DIVE_GRAVITY: f32 = 0.03;
const LEVEL_TORQUE: f32 = 0.001;
const ANGULAR_DAMPING: f32 = 0.99;
/// ponytail: inertia is not recovered from the original rigid body yet; tuned so
/// turn rates match footage (~45 deg/s yaw). Replace once the body setup is decoded.
const INERTIA: Vec3 = Vec3::new(90.0, 75.0, 90.0);

#[derive(Component)]
pub struct Plane {
    /// Units per frame.
    pub vel: Vec3,
    /// Radians per frame, world space.
    pub ang: Vec3,
    pub ctrl: Vec2,
    /// 0 stalled .. 1 fully flying.
    pub flight: f32,
    pub throttle: f32,
    bank: f32,
    gun_cooldown: u32,
    gun_side: bool,
}

#[derive(Component)]
struct PlaneModel;

#[derive(Component)]
struct Propeller;

#[derive(Component)]
struct Bullet {
    vel: Vec3,
    life: u32,
}

#[derive(Component)]
struct Flare {
    vel: Vec3,
    life: u32,
}

/// Fired when something should pop (bullets, plane body).
#[derive(Message)]
pub struct Shot {
    pub pos: Vec3,
    pub radius: f32,
}

pub fn plugin(app: &mut App) {
    app.add_message::<Shot>()
        .add_systems(Update, board.run_if(crate::menu::playing))
        .add_systems(FixedUpdate, (fly, guns, projectiles).chain().run_if(in_state(AppState::Play).and_then(in_state(Mode::Fly))))
        .add_systems(Update, (visuals, chase_camera).run_if(crate::menu::playing.and_then(in_state(Mode::Fly))))
        .add_systems(OnExit(Mode::Fly), despawn_plane);
}

fn model_path(kind: u8) -> &'static str { if kind == 1 { "models/plane02.glb" } else { "models/plane.glb" } }

/// Board (spawn the plane above the player) or eject with the parachute.
pub fn board(
    c: Res<Controls>,
    mode: Res<State<Mode>>,
    mut next: ResMut<NextState<Mode>>,
    mut commands: Commands,
    assets: Res<AssetServer>,
    save: Res<Save>,
    rig: Res<CamRig>,
    mut player: Single<(&mut Player, &mut Transform, &mut Visibility), Without<Plane>>,
    planes: Query<(&Plane, &Transform)>,
) {
    if !c.board {
        return;
    }
    match mode.get() {
        Mode::Walk => {
            let fwd = Vec3::new(-rig.yaw.sin(), 0.0, -rig.yaw.cos());
            let pos = player.1.translation + Vec3::Y * 600.0;
            commands
                .spawn((
                    Plane { vel: fwd * 7.0, ang: Vec3::ZERO, ctrl: Vec2::ZERO, flight: 1.0, throttle: 1.0, bank: 0.0, gun_cooldown: 0, gun_side: false },
                    Transform::from_translation(pos).looking_to(-fwd, Vec3::Y),
                    Visibility::default(),
                ))
                .with_children(|p| {
                    p.spawn((WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(model_path(save.plane)))), PlaneModel)).observe(attach_propeller);
                });
            *player.2 = Visibility::Hidden;
            next.set(Mode::Fly);
        }
        Mode::Fly => {
            if let Ok((plane, t)) = planes.single() {
                let (pl, pt, pv) = &mut *player;
                eject(pl, pt, t.translation, plane.vel);
                **pv = Visibility::Inherited;
            }
            next.set(Mode::Walk);
        }
    }
}

fn eject(p: &mut Player, t: &mut Transform, pos: Vec3, vel: Vec3) {
    t.translation = pos;
    p.vel = vel * 60.0 * 0.3 + Vec3::Y * 300.0;
    p.parachute = true;
}

fn despawn_plane(mut commands: Commands, planes: Query<Entity, With<Plane>>) {
    for e in &planes {
        commands.entity(e).despawn();
    }
}

/// The propeller is a separate model mounted on the `propeller_point` bone.
fn attach_propeller(ev: On<WorldInstanceReady>, children: Query<&Children>, names: Query<&Name>, mut commands: Commands, assets: Res<AssetServer>) {
    for e in children.iter_descendants(ev.entity) {
        if names.get(e).is_ok_and(|n| n.as_str() == "propeller_point") {
            commands.entity(e).with_child((WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset("models/propeller.glb"))), Transform::default(), Propeller));
        }
    }
}

fn fly(
    c: Res<Controls>,
    save: Res<Save>,
    col: Res<Collision>,
    mut next: ResMut<NextState<Mode>>,
    mut q: Query<(&mut Plane, &mut Transform)>,
    mut player: Single<(&mut Player, &mut Transform, &mut Visibility), Without<Plane>>,
) {
    let Ok((mut p, mut t)) = q.single_mut() else { return };
    let rot = t.rotation;
    let (x, y, z) = (rot * Vec3::X, rot * Vec3::Y, rot * Vec3::Z);
    let fwd = p.vel.dot(z);

    // Stick: x = turn right, y = push forward (nose down) unless inverted.
    let mut input = Vec2::new(c.stick.x, if save.invert_pitch { -c.stick.y } else { c.stick.y });
    input = input.clamp(Vec2::NEG_ONE, Vec2::ONE);
    let ctrl = p.ctrl;
    p.ctrl = ctrl + (input - ctrl) * 0.2;

    let mut force = Vec3::ZERO;
    let mut torque = Vec3::ZERO;
    // Tail forces: +X is the plane's left, so a force along +X at the tail yaws right.
    let tail = x * (RUDDER * p.ctrl.x) + y * (ELEVATOR * p.ctrl.y);
    torque += (-z * TAIL_ARM).cross(tail);

    // Engine: boost (thrust forward) / slow down (draw back) / cut the engine.
    p.throttle = if c.cut { (p.throttle - 0.02).max(0.0) } else { (p.throttle + 0.02).min(1.0) };
    let accel = c.boost - c.slow;
    force += z * (THRUST * p.throttle + accel.abs() * if accel >= 0.0 { BOOST } else { SLOW });

    // Flying factor: rises slowly with airspeed, drops immediately.
    let target = (p.throttle.max(0.4) * ((fwd - 0.5) / 0.5).clamp(0.0, 1.0)).powi(4);
    p.flight = if target > p.flight { p.flight + (target - p.flight) * 0.01 } else { target };
    let stall_lift = (0.1 - 0.1 * p.flight) * ((fwd - 0.5) / 2.5).clamp(0.0, 1.0);
    force += y * stall_lift;

    // Per-axis drag in plane space.
    let local = rot.inverse() * p.vel;
    let k = DRAG_STALL.lerp(DRAG_FLY, p.flight);
    force -= rot * (k * local) * DRAG_GAIN;

    // Gravity is cancelled by lift in level flight; diving adds a little speed.
    let g = p.flight * (z.y.abs() * if z.y < 0.0 { DIVE_GRAVITY } else { 0.0 } - STALL_GRAVITY) + STALL_GRAVITY;
    force.y -= g;

    // Self-levelling towards world up, weakened while pitching.
    torque += y.cross(Vec3::Y) * LEVEL_TORQUE * (1.0 - p.ctrl.y.abs());

    p.vel += force;
    let local_t = rot.inverse() * torque;
    p.ang += rot * (local_t / INERTIA);
    p.ang *= ANGULAR_DAMPING;
    let ang = p.ang;
    if ang.length_squared() > 1e-12 {
        t.rotation = (Quat::from_scaled_axis(ang) * t.rotation).normalize();
    }
    // Keep the plane's roll for visuals separate from physics.
    p.bank += (-p.ctrl.x * 0.6 - p.bank) * 0.08;

    // Terrain / sea: crash and bail out with the parachute.
    let step = p.vel;
    let from = t.translation;
    let crashed = col.raycast(from, step.normalize_or_zero(), step.length() + 25.0).is_some() || from.y + step.y < 5.0;
    if crashed {
        let (pl, pt, pv) = &mut *player;
        eject(pl, pt, from - step * 10.0 + Vec3::Y * 80.0, p.vel * 0.2);
        **pv = Visibility::Inherited;
        next.set(Mode::Walk);
        return;
    }
    t.translation += step;
}

fn guns(c: Res<Controls>, mut commands: Commands, assets: Res<AssetServer>, mut q: Query<(&mut Plane, &Transform)>) {
    let Ok((mut p, t)) = q.single_mut() else { return };
    p.gun_cooldown = p.gun_cooldown.saturating_sub(1);
    if c.shoot && p.gun_cooldown == 0 {
        p.gun_cooldown = 6;
        p.gun_side = !p.gun_side;
        let side = if p.gun_side { 14.0 } else { -14.0 };
        let pos = t.transform_point(Vec3::new(side, -2.0, 20.0));
        let vel = p.vel + t.rotation * Vec3::Z * 22.0;
        commands.spawn((
            WorldAssetRoot(assets.load(GltfAssetLabel::Scene(1).from_asset("models/bullet.glb"))),
            Transform::from_translation(pos).with_rotation(t.rotation).with_scale(Vec3::splat(1.5)),
            Bullet { vel, life: 75 },
        ));
    }
    if c.flare {
        commands.spawn((
            WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset("models/signalgun.glb"))),
            Transform::from_translation(t.translation + Vec3::Y * 10.0).with_scale(Vec3::splat(3.0)),
            PointLight { color: Color::srgb(1.0, 0.4, 0.3), intensity: 5.0e8, range: 3000.0, ..default() },
            Flare { vel: p.vel * 0.5 + Vec3::Y * 9.0, life: 360 },
        ));
    }
}

fn projectiles(
    mut commands: Commands,
    col: Res<Collision>,
    mut shots: MessageWriter<Shot>,
    mut bullets: Query<(Entity, &mut Bullet, &mut Transform), Without<Flare>>,
    mut flares: Query<(Entity, &mut Flare, &mut Transform), Without<Bullet>>,
) {
    for (e, mut b, mut t) in &mut bullets {
        let len = b.vel.length();
        if b.life == 0 || col.raycast(t.translation, b.vel / len, len).is_some() {
            commands.entity(e).despawn();
            continue;
        }
        b.life -= 1;
        t.translation += b.vel;
        shots.write(Shot { pos: t.translation, radius: 30.0 });
    }
    for (e, mut f, mut t) in &mut flares {
        if f.life == 0 {
            commands.entity(e).despawn();
            continue;
        }
        f.life -= 1;
        f.vel *= 0.985;
        f.vel.y -= 0.02;
        t.translation += f.vel;
    }
}

fn visuals(time: Res<Time>, planes: Query<&Plane>, mut models: Query<&mut Transform, (With<PlaneModel>, Without<Propeller>)>, mut props: Query<&mut Transform, (With<Propeller>, Without<PlaneModel>)>) {
    let Ok(p) = planes.single() else { return };
    for mut t in &mut models {
        t.rotation = Quat::from_rotation_z(p.bank);
    }
    for mut t in &mut props {
        t.rotate_local_z(time.delta_secs() * (20.0 + 40.0 * p.throttle));
    }
}

fn chase_camera(
    c: Res<Controls>,
    time: Res<Time>,
    col: Res<Collision>,
    mut look: Local<Vec2>,
    plane: Single<(&Plane, &Transform)>,
    mut cam: Single<&mut Transform, (With<MainCamera>, Without<Plane>)>,
) {
    let (p, t) = *plane;
    // Look around with the right stick / mouse; springs back when released.
    *look = if c.look.length() > 0.0005 { (*look + c.look * Vec2::new(-1.0, 1.0)).clamp(Vec2::new(-2.5, -0.8), Vec2::new(2.5, 0.8)) } else { *look * 0.92 };
    let nose = t.rotation * Vec3::Z;
    let heading = Quat::from_rotation_y(nose.x.atan2(nose.z));
    let fwd = (heading * Quat::from_euler(EulerRot::YXZ, look.x, -look.y, 0.0)) * Vec3::Z;
    let speed_pull = (p.vel.length() / 7.0).clamp(0.5, 2.0);
    let target = t.translation + Vec3::Y * 20.0;
    let mut dist = 130.0 * speed_pull.sqrt();
    let dir = (-fwd + Vec3::Y * 0.28).normalize();
    if let Some(h) = col.raycast(target, dir, dist) {
        dist = (h.t - 10.0).max(30.0);
    }
    let want = target + dir * dist;
    let k = (8.0 * time.delta_secs()).min(1.0);
    cam.translation = cam.translation.lerp(want, k);
    cam.look_at(target + fwd * 60.0, Vec3::Y);
}
