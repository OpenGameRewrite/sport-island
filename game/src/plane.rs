//! Island Flyover plane: flight model, camera, Balloon Popper and flares.
//!
//! The flight model follows the structure of the original: a rigid body pushed
//! by tail forces (steering), engine thrust, speed-dependent lift and per-axis
//! drag, all in game units per 60 Hz frame. Core constants were recovered from
//! the original. Ground assistance and modern input adaptation remain partial.

use crate::collide::Collision;
use crate::input::Controls;
use crate::player::{CamRig, Player};
use crate::save::Save;
use crate::world::MainCamera;
use crate::Mode;
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
// The original plane's body setup uses unit mass and diagonal inertia 10/10/10.
const INERTIA: Vec3 = Vec3::splat(10.0);

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
    previous: Transform,
    boost_left: u8,
    boost_held: bool,
    boost_force: f32,
    boost_sign: f32,
}

#[derive(Resource)]
pub struct FirstFlight { boarded: bool, starts: [Transform; 3] }
#[derive(Resource)]
struct PlaneColors([Vec4; 12]);

impl Plane {
    fn new(pose: Transform, speed: f32) -> Self {
        Self { vel: pose.rotation * Vec3::Z * speed, ang: Vec3::ZERO, ctrl: Vec2::ZERO, flight: 1.0, throttle: 1.0, bank: 0.0, gun_cooldown: 0, gun_side: false, previous: pose, boost_left: 0, boost_held: false, boost_force: 0.0, boost_sign: 1.0 }
    }
    /// One original-rate physics tick, independent of rendering and collision.
    fn step(&mut self, t: &mut Transform, c: &Controls, invert_pitch: bool) {
        self.previous = *t;
        let rot = t.rotation;
        let (x, y, z) = (rot * Vec3::X, rot * Vec3::Y, rot * Vec3::Z);
        let fwd = self.vel.dot(z);

        // Stick: x = turn right, y = push forward (nose down) unless inverted.
        let mut input = Vec2::new(c.stick.x, if invert_pitch { -c.stick.y } else { c.stick.y });
        input = input.clamp(Vec2::NEG_ONE, Vec2::ONE);
        // Wiimote tilt was an attitude input. A stick/key maps directly to it;
        // simulation already supplies angular momentum, so another low-pass lags it.
        self.ctrl = input;

        let mut force = Vec3::ZERO;
        let mut torque = Vec3::ZERO;
        // Tail forces: +X is the plane's left, so a force along +X at the tail yaws right.
        let tail = x * (RUDDER * self.ctrl.x) + y * (ELEVATOR * self.ctrl.y);
        torque += (-z * TAIL_ARM).cross(tail);

        // Engine: boost (thrust forward) / slow down (draw back) / cut the engine.
        self.throttle = if c.cut { (self.throttle - 0.02).max(0.0) } else { (self.throttle + 0.02).min(1.0) };
        // A shake was an impulse, with a 0.04/frame re-arm timer and a decaying
        // envelope. Adapt it to trigger/key presses, never continuous held thrust.
        let accel = c.boost - c.slow;
        let held = accel.abs() > 0.15;
        if held && !self.boost_held && self.boost_left == 0 {
            self.boost_left = 25;
            self.boost_sign = accel.signum();
        }
        self.boost_held = held;
        self.boost_force = ((self.boost_force + if self.boost_left > 0 { 1.0 } else { 0.0 }) * 0.993).min(1.0);
        self.boost_left = self.boost_left.saturating_sub(1);
        force += z * (THRUST * self.throttle + self.boost_force * if self.boost_sign >= 0.0 { BOOST } else { SLOW });

        // Flying factor: rises slowly with airspeed, drops immediately.
        let target = (self.throttle * ((fwd - 0.5) / 0.5).clamp(0.0, 1.0)).powi(4);
        self.flight = if target > self.flight { self.flight + (target - self.flight) * 0.01 } else { target };
        let stall_lift = (0.1 - 0.1 * self.flight) * ((fwd - 0.5) / 2.5).clamp(0.0, 1.0);
        force += y * stall_lift;

        // Per-axis drag in plane space.
        let local = rot.inverse() * self.vel;
        let k = DRAG_STALL.lerp(DRAG_FLY, self.flight);
        force -= rot * (k * local) * DRAG_GAIN;

        // Gravity is cancelled by lift in level flight; diving adds a little speed.
        let g = self.flight * (z.y.abs() * if z.y < 0.0 { DIVE_GRAVITY } else { 0.0 } - STALL_GRAVITY) + STALL_GRAVITY;
        force.y -= g;

        // Self-levelling towards world up, weakened while pitching.
        torque += y.cross(Vec3::Y) * LEVEL_TORQUE * (1.0 - self.ctrl.y.abs());

        self.vel = self.vel * 0.997 + force;
        let local_t = rot.inverse() * torque;
        self.ang = self.ang * ANGULAR_DAMPING + rot * (local_t / INERTIA);
        let ang = self.ang;
        if ang.length_squared() > 1e-12 {
            t.rotation = (Quat::from_scaled_axis(ang) * t.rotation).normalize();
        }
        // Keep the plane's roll for visuals separate from physics.
        self.bank += (self.ctrl.x * 0.6 - self.bank) * 0.08;
    }

    fn pose(&self, t: &Transform, fraction: f32) -> Transform {
        Transform::from_translation(self.previous.translation.lerp(t.translation, fraction))
            .with_rotation(self.previous.rotation.slerp(t.rotation, fraction))
    }
}

#[derive(Component)]
struct PlaneModel;

#[derive(Component)]
struct PlanePaint(u8);
#[derive(Component)]
struct PaintSource(Handle<crate::gx::GxMaterial>);

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
    let colors: [[u8; 4]; 12] = crate::read_json("data/plane_colors.json");
    app.insert_resource(PlaneColors(colors.map(|c| Vec4::from_array(c.map(|v| v as f32 / 255.0)))));
    let placements: crate::scenery::Placements = crate::read_json("data/island_f0.json");
    let starts = std::array::from_fn(|i| placements.objects.iter().find(|p| p.group == 3 && p.kind == 2 && (p.params[1] >> 16) as usize == i).expect("original water departure missing").transform());
    app.insert_resource(FirstFlight { boarded: false, starts }).add_message::<Shot>()
        .add_systems(Update, paint)
        .add_systems(Update, board.run_if(crate::menu::playing))
        .add_systems(FixedUpdate, (fly, guns, projectiles).chain().run_if(crate::menu::playing.and_then(in_state(Mode::Fly))))
        .add_systems(Update, (visuals, chase_camera, trails).run_if(crate::menu::playing.and_then(in_state(Mode::Fly))))
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
    tod: Res<crate::world::TimeOfDay>,
    mut first: ResMut<FirstFlight>,
    mut player: Single<(&mut Player, &mut Transform, &mut Visibility), Without<Plane>>,
    planes: Query<(&Plane, &Transform)>,
) {
    if !c.board {
        return;
    }
    match mode.get() {
        Mode::Walk => {
            // Original first departure, facing the town from the water. Later
            // boarding stays convenient for unrestricted on-foot exploration.
            let pose = if first.boarded {
                let fwd = Vec3::new(-rig.yaw.sin(), 0.0, -rig.yaw.cos());
                Transform::from_translation(player.1.translation + Vec3::Y * 600.0).looking_to(-fwd, Vec3::Y)
            } else { first.starts[tod.0 as usize] };
            let speed = if first.boarded { 7.0 } else { 2.5 };
            first.boarded = true;
            commands
                .spawn((
                    Plane::new(pose, speed),
                    pose,
                    Visibility::default(),
                ))
                .with_children(|p| {
                    p.spawn((WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(model_path(save.plane)))), PlaneModel)).observe(attach_propeller).observe(mark_paint);
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

fn mark_paint(ev: On<WorldInstanceReady>, children: Query<&Children>, materials: Query<&bevy::gltf::GltfMaterialName>, mut commands: Commands) {
    for e in children.iter_descendants(ev.entity) {
        if let Ok(name) = materials.get(e) {
            // Primitive entity names are synthesized; use the original material name.
            let kind = match name.0.as_str() { "mt_WS2_pln_plane" => 0, "mt_plane_02" => 1, _ => continue };
            commands.entity(e).insert(PlanePaint(kind));
        }
    }
}

fn paint(mut commands: Commands, save: Res<Save>, colors: Res<PlaneColors>, assets: Res<AssetServer>, mut mats: ResMut<Assets<crate::gx::GxMaterial>>, mut q: Query<(Entity, &PlanePaint, Option<&PaintSource>, &mut MeshMaterial3d<crate::gx::GxMaterial>)>, mut cache: Local<std::collections::HashMap<(AssetId<crate::gx::GxMaterial>, u8, u8), Handle<crate::gx::GxMaterial>>>) {
    for (e, model, original, mut handle) in &mut q {
        let source = original.map_or_else(|| handle.0.clone(), |s| s.0.clone());
        let color = save.livery.min(if model.0 == 0 {2} else {1});
        let key = (source.id(), model.0, color);
        let replacement = if let Some(h) = cache.get(&key) { h.clone() } else {
            let Some(mut material) = mats.get(&source).cloned() else { continue };
            let texture = if model.0 == 0 { ["WS2_pln_plane_tex_normal", "WS2_pln_plane_tex_white", "WS2_pln_plane_tex_yellow"][color as usize] } else { "WS2_pln_plane02" };
            // Paint areas use C0, blended with the atlas's alpha. The original
            // overrides it with the pilot's favorite-color palette at runtime.
            let palette = if model.0 == 0 { [0, 10, 2][color as usize] } else { [1, 5][color as usize] };
            material.tev.regs[1] = colors.0[palette];
            material.t0 = Some(assets.load_builder().with_settings(|s: &mut bevy::image::ImageLoaderSettings| s.is_srgb = false).load(format!("textures/{texture}.png")));
            if std::env::var_os("SI_LOG").is_some() { info!("Plane livery: {texture}"); }
            let h = mats.add(material); cache.insert(key, h.clone()); h
        };
        if original.is_none() { commands.entity(e).insert(PaintSource(source)); }
        handle.0 = replacement;
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
    p.step(&mut t, &c, save.invert_pitch);

    // Terrain / sea: crash and bail out with the parachute.
    let step = p.vel;
    let from = t.translation;
    // The original ground proximity helper keeps low passes afloat.
    if let Some(h) = col.ground(from, 0.0, 120.0) && h.n.y.abs() > 0.6 {
        p.vel.y += (1.0 - ((h.t - 30.0) / 90.0).clamp(0.0, 1.0)) * 0.1;
    }
    if from.y < 35.0 { p.vel.y += ((35.0 - from.y) * 0.01).min(0.1); }
    let crashed = col.raycast(from, step.normalize_or_zero(), step.length() + 10.0).is_some() || from.y + step.y < 5.0;
    if crashed {
        let (pl, pt, pv) = &mut *player;
        eject(pl, pt, from - step * 10.0 + Vec3::Y * 80.0, p.vel * 0.2);
        **pv = Visibility::Inherited;
        next.set(Mode::Walk);
        return;
    }
    t.translation += step;
}

fn guns(mut c: ResMut<Controls>, mut commands: Commands, assets: Res<AssetServer>, mut sounds: MessageWriter<crate::sound::Sound>, mut q: Query<(&mut Plane, &Transform)>) {
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
        sounds.write(crate::sound::Sound::effect("SE_PLN_SHOOT_BULLET", None));
    }
    if c.flare {
        c.flare = false;
        sounds.write(crate::sound::Sound::effect("SE_PLN_ILLUM_FLAME_PLAYER_LAUNCH", None));
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

fn visuals(time: Res<Time>, fixed: Res<Time<Fixed>>, planes: Query<(&Plane, &Transform), (Without<PlaneModel>, Without<Propeller>)>, mut models: Query<&mut Transform, (With<PlaneModel>, Without<Propeller>)>, mut props: Query<&mut Transform, (With<Propeller>, Without<PlaneModel>)>) {
    let Ok((p, root)) = planes.single() else { return };
    let pose = p.pose(root, fixed.overstep_fraction());
    for mut t in &mut models {
        t.translation = root.rotation.inverse() * (pose.translation - root.translation);
        t.rotation = root.rotation.inverse() * pose.rotation * Quat::from_rotation_z(p.bank);
    }
    for mut t in &mut props {
        t.rotate_local_z(time.delta_secs() * (20.0 + 40.0 * p.throttle));
    }
}

fn chase_camera(
    c: Res<Controls>,
    time: Res<Time>,
    fixed: Res<Time<Fixed>>,
    col: Res<Collision>,
    mut look: Local<Vec2>,
    mut idle: Local<f32>,
    plane: Single<(&Plane, &Transform)>,
    mut cam: Single<&mut Transform, (With<MainCamera>, Without<Plane>)>,
) {
    let (p, root) = *plane;
    let t = p.pose(root, fixed.overstep_fraction());
    let dt = time.delta_secs();
    if c.look.length() > 0.0005 {
        *look = (*look + c.look * Vec2::new(-1.0, 1.0)).clamp(Vec2::new(-2.5, -1.2), Vec2::new(2.5, 1.2));
        *idle = 0.0;
    } else {
        *idle += dt;
        if *idle > 4.0 { *look *= (-0.7 * dt).exp(); }
    }
    let nose = t.rotation * Vec3::Z;
    let pitch = -nose.y.clamp(-1.0, 1.0).asin();
    let heading = Quat::from_euler(EulerRot::YXZ, nose.x.atan2(nose.z), pitch, 0.0);
    let fwd = (heading * Quat::from_euler(EulerRot::YXZ, look.x, -look.y, 0.0)) * Vec3::Z;
    let speed_pull = (p.vel.length() / 7.0).clamp(0.5, 2.0);
    let target = t.translation + Vec3::Y * 20.0;
    let mut dist = 130.0 * speed_pull.sqrt();
    let dir = (-fwd + Vec3::Y * 0.28).normalize();
    if let Some(h) = col.raycast(target, dir, dist) {
        dist = (h.t - 10.0).max(30.0);
    }
    let want = target + dir * dist;
    let k = 1.0 - (-12.0 * dt).exp();
    cam.translation = cam.translation.lerp(want, k);
    cam.look_at(target + fwd * 60.0, Vec3::Y);
}

/// Three short, fading ribbons sampled from the interpolated model pose.
fn trails(time: Res<Time>, fixed: Res<Time<Fixed>>, plane: Single<(&Plane, &Transform)>, mut history: Local<std::collections::VecDeque<(f32, [Vec3; 3])>>, mut gizmos: Gizmos) {
    let now = time.elapsed_secs();
    let (p, root) = *plane;
    let pose = p.pose(root, fixed.overstep_fraction());
    if history.back().is_none_or(|(t, _)| now - t > 1.0 / 60.0) {
        let bank = Quat::from_rotation_z(p.bank);
        history.push_back((now, [Vec3::new(-32.0, 0.0, -1.0), Vec3::new(32.0, 0.0, -1.0), Vec3::new(0.0, -2.0, -29.0)].map(|v| pose.transform_point(bank * v))));
    }
    while history.front().is_some_and(|(t, _)| now - t > 1.8) { history.pop_front(); }
    for i in 1..history.len() {
        let (born, a) = history[i - 1];
        let (_, b) = history[i];
        if a[0].distance(b[0]) > 100.0 { continue; }
        let alpha = (1.0 - (now - born) / 1.8) * 0.45;
        for j in 0..3 { gizmos.line(a[j], b[j], Color::srgba(0.85, 0.95, 1.0, alpha)); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn livery_tags_the_primitive_using_its_material_not_its_entity_name() {
        let mut world = World::new();
        let root = world.spawn_empty().observe(mark_paint).id();
        let primitive = world.spawn((ChildOf(root), Name::new("generated primitive name"), bevy::gltf::GltfMaterialName("mt_plane_02".into()))).id();
        let other = world.spawn((ChildOf(root), bevy::gltf::GltfMaterialName("propeller".into()))).id();
        let id = bevy::world_serialization::WorldInstanceSpawner::default().spawn(Handle::<bevy::world_serialization::WorldAsset>::default());
        world.trigger(WorldInstanceReady { entity: root, instance_id: id });
        world.flush();
        assert_eq!(world.get::<PlanePaint>(primitive).unwrap().0, 1);
        assert!(world.get::<PlanePaint>(other).is_none());
    }

    #[test]
    fn right_input_turns_and_banks_right_then_stops_adding_steering() {
        let mut pose = Transform::IDENTITY;
        let mut plane = Plane::new(pose, 7.0);
        let mut controls = Controls { stick: Vec2::X, ..default() };
        for _ in 0..30 { plane.step(&mut pose, &controls, false); }
        assert!((pose.rotation * Vec3::Z).x < -0.3, "right turn must move toward the model's -X side");
        assert!((Quat::from_rotation_z(plane.bank) * Vec3::NEG_X).y < 0.0, "right wing must bank down");
        let rate = plane.ang.y.abs();
        controls.stick = Vec2::ZERO;
        for _ in 0..30 { plane.step(&mut pose, &controls, false); }
        assert_eq!(plane.ctrl, Vec2::ZERO);
        assert!(plane.ang.y.abs() < rate * 0.8, "released steering must decay");
    }

    #[test]
    fn holding_boost_cannot_retrigger_but_a_new_press_can() {
        let mut pose = Transform::IDENTITY;
        let mut plane = Plane::new(pose, 7.0);
        let mut controls = Controls { boost: 1.0, ..default() };
        for _ in 0..300 { plane.step(&mut pose, &controls, false); }
        assert_eq!(plane.boost_left, 0);
        assert!(plane.boost_force < 0.2, "held key must let the impulse decay");
        controls.boost = 0.0;
        plane.step(&mut pose, &controls, false);
        controls.boost = 1.0;
        plane.step(&mut pose, &controls, false);
        assert!(plane.boost_force > 0.9, "fresh press must boost again");
        assert!(plane.boost_left > 0);
    }

    #[test]
    fn pitch_input_and_inversion_are_opposites() {
        for invert in [false, true] {
            let mut pose = Transform::IDENTITY;
            let mut plane = Plane::new(pose, 7.0);
            let controls = Controls { stick: Vec2::Y, ..default() };
            for _ in 0..15 { plane.step(&mut pose, &controls, invert); }
            let height = (pose.rotation * Vec3::Z).y;
            assert!(if invert { height > 0.1 } else { height < -0.1 });
        }
    }
}
