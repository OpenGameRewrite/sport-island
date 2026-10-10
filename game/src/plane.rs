//! Island Flyover plane: flight model, camera, Balloon Popper and flares.
//!
//! The flight model follows the structure of the original: a rigid body pushed
//! by tail forces (steering), engine thrust, speed-dependent lift and per-axis
//! drag, all in game units per 60 Hz frame. Core constants were recovered from
//! the original, including terrain-normal assistance and the evolving up target.
//! Water contact uses the recovered release, damping and alignment curves.
//! The modern input mapping and the remaining reference-body contacts are partial.

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
    up_target: Vec3,
    inverted: f32,
    ground_up: Vec3,
    ground_strength: f32,
    water_strength: f32,
    water_release: f32,
    contact_forward: Vec3,
    /// Settings multiplier for the pull back toward the reference heading.
    pub turn_recovery: f32,
}

#[derive(Resource)]
pub struct FirstFlight { boarded: bool, starts: [Transform; 3] }
#[derive(Resource)]
struct PlaneColors([Vec4; 12]);

impl Plane {
    fn new(pose: Transform, speed: f32) -> Self {
        Self { vel: pose.rotation * Vec3::Z * speed, ang: Vec3::ZERO, ctrl: Vec2::ZERO, flight: 1.0, throttle: 1.0, bank: 0.0, gun_cooldown: 0, gun_side: false, previous: pose, boost_left: 0, boost_held: false, boost_force: 0.0, boost_sign: 1.0, up_target: Vec3::Y, inverted: 0.0, ground_up: Vec3::Y, ground_strength: 0.0, water_strength: 0.0, water_release: 0.0, contact_forward: pose.rotation * Vec3::Z, turn_recovery: 1.0 }
    }
    /// One original-rate physics tick, independent of rendering and collision.
    #[cfg(test)]
    fn step(&mut self, t: &mut Transform, c: &Controls, invert_pitch: bool) {
        self.step_with_surface(t, c, invert_pitch, None);
    }

    fn step_with_surface(&mut self, t: &mut Transform, c: &Controls, invert_pitch: bool, surface: Option<(f32, Vec3)>) {
        self.previous = *t;
        let rot = t.rotation;
        let (x, y, z) = (rot * Vec3::X, rot * Vec3::Y, rot * Vec3::Z);
        let fwd = self.vel.dot(z);

        // Stick: x = turn right, y = push forward (nose down) unless inverted.
        let mut input = Vec2::new(c.stick.x, if invert_pitch { -c.stick.y } else { c.stick.y });
        input = input.clamp(Vec2::NEG_ONE, Vec2::ONE);
        // Wiimote tilt was an attitude input. A stick/key maps directly to it;
        // simulation already supplies angular momentum, so another low-pass lags it.
        self.ctrl = input * (1.0 - 0.95 * self.water_strength);

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

        // Original low-pass helper: 30..120 clearance and 0.6..0.9 surface/up
        // alignment. Push along the surface normal projected off the nose,
        // scaled by tangential speed; do not add a fixed world-Y impulse.
        let mut ground = 0.0;
        if let Some((clearance, normal)) = surface {
            ground = ((normal.dot(y) - 0.6) / 0.3).clamp(0.0, 1.0)
                * (1.0 - ((clearance - 30.0) / 90.0).clamp(0.0, 1.0));
            if ground > 0.0 {
                self.ground_up = (normal - z * normal.dot(z)).normalize_or_zero();
            }
        }
        self.ground_strength = self.flight * (self.ground_strength + 0.01 * (ground - self.ground_strength));
        let tangent_speed = (self.vel - self.ground_up * self.vel.dot(self.ground_up)).length();
        let assistance = self.ground_strength * tangent_speed;
        force += self.ground_up * (assistance * 0.1);
        torque += y.cross(self.ground_up) * (assistance * 0.004);

        let level = 1.0 - self.ctrl.y.abs();
        torque += y.cross(self.up_target) * LEVEL_TORQUE * level;
        // Reference direction is disturbed by contacts and settles back to the
        // nose with the original 0.01..0.05 water-dependent blend.
        // The 0.007 gain is the original's; the lagging reference it pulls toward
        // is reconstructed, so its strength is a player setting (Turn recovery).
        torque += z.cross(self.contact_forward) * 0.007 * self.turn_recovery;
        self.contact_forward = self.contact_forward.lerp(z, 0.01 + 0.04*self.water_strength).normalize_or_zero();

        self.vel = self.vel * 0.997 + force;
        let local_t = rot.inverse() * torque;
        self.ang = self.ang * ANGULAR_DAMPING + rot * (local_t / INERTIA);
        // The body's original integrator multiplies by (omega/2, 1), then
        // normalizes; its effective angle is 2*atan(|omega|/2).
        let half = self.ang * 0.5;
        t.rotation = (Quat::from_xyzw(half.x, half.y, half.z, 1.0) * rot).normalize();

        // Constrain the right axis against the evolving up target, then rebuild
        // an orthonormal frame. Pitch remains free through vertical maneuvers.
        let body = Mat3::from_quat(t.rotation);
        let right = (body.x_axis - self.up_target * body.x_axis.dot(self.up_target)).normalize_or_zero();
        let nose = right.cross(body.y_axis).normalize_or_zero();
        if right != Vec3::ZERO && nose != Vec3::ZERO {
            t.rotation = Quat::from_mat3(&Mat3::from_cols(right, nose.cross(right), nose)).normalize();
        }
        let (up, nose) = (t.rotation * Vec3::Y, t.rotation * Vec3::Z);
        let upright = ((up.y + 1.0) / 1.3).clamp(0.0, 1.0);
        self.inverted = ((self.inverted + (1.0 - upright) * 0.01) * 0.999).min(1.0);
        let weight = level * (0.01 - 0.009 * self.inverted);
        // Rotate a fraction of the angle to world up, as in the vector helper.
        let axis = up.cross(Vec3::Y);
        let target = if axis.length_squared() > 3.8147e-6 {
            Quat::from_axis_angle(axis.normalize(), axis.length().atan2(up.dot(Vec3::Y)) * weight) * up
        } else { up };
        let target = (target - nose * target.dot(nose)).normalize_or_zero();
        if target != Vec3::ZERO { self.up_target = target; }
        // Keep the plane's roll for visuals separate from physics.
        self.bank += (self.ctrl.x * 0.6 - self.bank) * 0.08;
    }

    fn pose(&self, t: &Transform, fraction: f32) -> Transform {
        Transform::from_translation(self.previous.translation.lerp(t.translation, fraction))
            .with_rotation(self.previous.rotation.slerp(t.rotation, fraction))
    }

    fn water_contact(&mut self, t: &mut Transform, contact: Option<(f32, Vec3, f32)>, pitch: f32) {
        let speed = (t.rotation.inverse()*self.vel).z;
        let near = contact.map_or(0.0, |(clearance, _, _)| 1.0 - ((clearance-6.5)/1.0).clamp(0.0,1.0));
        let speed_release = ((speed-1.0)/4.0).clamp(0.0,1.0);
        let pitch_release = 1.0 - ((pitch+0.1)/0.1).clamp(0.0,1.0);
        self.water_strength = (self.water_strength + near*0.1 - 0.02*(speed_release+pitch_release)).clamp(0.0,1.0);
        self.water_release = (self.water_release + if self.water_strength > 0.0 {0.05} else {-0.01}).clamp(0.0,1.0);
        let w = self.water_strength;
        let mut local = t.rotation.inverse()*self.vel;
        local.x *= 1.0-0.5*w;
        if local.y < 0.0 {local.y *= 1.0-0.1*w;}
        local.z *= if local.z < 0.0 {1.0-w} else {1.0-0.001*w};
        self.vel = t.rotation*local;
        if let Some((_, normal, height)) = contact {
            let up = (t.rotation*Vec3::Y).lerp(normal,0.005*w).normalize_or_zero();
            let nose = t.rotation*Vec3::Z;
            let right = up.cross(nose).normalize_or_zero();
            if right != Vec3::ZERO {
                t.rotation = Quat::from_mat3(&Mat3::from_cols(right, nose.cross(right).normalize(), nose)).normalize();
            }
            // Contact frame correction from the original surface callback.
            if self.vel.length_squared()>0.01 {
                self.contact_forward = self.vel.normalize();
                let correction = Quat::IDENTITY.slerp(Quat::from_rotation_arc(nose, self.contact_forward),0.01*(1.0-w));
                t.rotation = (correction*t.rotation).normalize();
            }
            if t.translation.y < height+5.0 { t.translation.y = height+5.0; self.vel.y = self.vel.y.max(0.0); }
        }
    }
}

#[derive(Component)]
struct PlaneModel;
#[derive(Component)] struct PlaneAnimation { nodes: Vec<AnimationNodeIndex>, water: bool }

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
#[derive(Component)] struct Speedometer;
#[derive(Component)] struct ProjectileSprite;
#[derive(Component)] struct FlareCore;
#[derive(Component)] struct FlareTrail { remaining: f32, lifetime: f32, alpha: f32, vel: Vec3 }
#[derive(serde::Deserialize)] struct FlareDef { texture: String, color: [u8; 4], size: [f32; 2], life: f32, wrap: [u8; 2] }
#[derive(Resource)] struct FlareEffects { quad: Handle<Mesh>, parts: std::collections::BTreeMap<String, (FlareDef, Handle<StandardMaterial>)> }

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
        .add_systems(Startup, (speedometer_spawn, load_flare_effects))
        .add_systems(Update, (speedometer, flare_visuals))
        .add_systems(PostUpdate, projectile_billboards.before(TransformSystems::Propagate))
        .add_systems(Update, paint)
        .add_systems(Update, board.run_if(crate::menu::playing))
        .add_systems(FixedUpdate, (fly, guns).chain().run_if(crate::menu::playing.and_then(in_state(Mode::Fly))))
        .add_systems(FixedUpdate, projectiles.after(guns).run_if(crate::menu::playing))
        .add_systems(Update, (visuals, animate, chase_camera, trails).run_if(crate::menu::playing.and_then(in_state(Mode::Fly))))
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
            let mut plane = Plane::new(pose, speed);
            if !first.boarded { plane.flight = 0.0; plane.water_strength = 1.0; plane.water_release = 1.0; }
            first.boarded = true;
            commands
                .spawn((
                    plane,
                    pose,
                    Visibility::default(),
                ))
                .with_children(|p| {
                    p.spawn((WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(model_path(save.plane)))), PlaneModel)).observe(attach_propeller).observe(mark_paint).observe(attach_animation);
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

fn attach_animation(ev: On<WorldInstanceReady>, children: Query<&Children>, mut players: Query<&mut AnimationPlayer>, plane: Single<&Plane>, save: Res<Save>, assets: Res<AssetServer>, mut graphs: ResMut<Assets<AnimationGraph>>, mut commands: Commands) {
    let (graph,nodes) = AnimationGraph::from_clips([0,3].map(|i| assets.load(GltfAssetLabel::Animation(i).from_asset(model_path(save.plane)))));
    let graph = graphs.add(graph);
    let water = plane.water_strength>0.1;
    for child in children.iter_descendants(ev.entity) {
        if let Ok(mut player) = players.get_mut(child) {
            player.play(nodes[if water {0} else {1}]);
            commands.entity(child).insert((AnimationGraphHandle(graph.clone()), PlaneAnimation {nodes:nodes.clone(),water}));
        }
    }
}
fn animate(plane: Single<&Plane>, mut players: Query<(&mut AnimationPlayer, &mut PlaneAnimation)>) {
    let water = plane.water_strength>0.1 &&plane.water_release>0.1;
    for (mut player,mut animation) in &mut players {
        if animation.water != water { animation.water = water; player.start(animation.nodes[if water {0} else {1}]); }
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
pub(crate) fn attach_propeller(ev: On<WorldInstanceReady>, children: Query<&Children>, names: Query<&Name>, mut commands: Commands, assets: Res<AssetServer>, mut meshes: ResMut<Assets<Mesh>>, mut materials: ResMut<Assets<StandardMaterial>>) {
    for e in children.iter_descendants(ev.entity) {
        if names.get(e).is_ok_and(|n| n.as_str() == "propeller_point") {
            commands.entity(e).with_child((WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset("models/propeller.glb"))), Transform::default(), Propeller));
        }
        if names.get(e).is_ok_and(|n| n.as_str() == "cockpit") {
            commands.entity(e).with_children(|c| {
                // Cockpit bones mark the seat, not the standing body's feet.
                // Anchor its hips (y=15) here and fit the legs inside the hull.
                c.spawn((Name::new("Seated pilot"), Transform::from_xyz(0.0, -3.75, 0.0).with_scale(Vec3::splat(0.25)), Visibility::default()))
                    .with_children(|c| crate::player::body(c, &mut meshes, &mut materials, true));
            });
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
    let from = t.translation;
    let ground = col.ground(from, 0.0, 120.0);
    // The original contact callback maps KCL's low five bits to surface flags;
    // bit 0x40 (type 6) selects the water controller, not land assistance.
    let water = ground.as_ref().filter(|h| h.flags & 0x1f == 6);
    if water.is_some() { p.ground_strength = 0.0; }
    let surface = ground.as_ref().filter(|h| h.flags & 0x1f != 6).map(|h| (h.t, if h.n.y < 0.0 { -h.n } else { h.n }));
    let mut controls = c.clone();
    controls.stick *= save.flight_sensitivity;
    let pitch = if save.invert_pitch {-controls.stick.y} else {controls.stick.y};
    p.turn_recovery = save.turn_recovery;
    p.water_contact(&mut t, water.map(|h| (h.t, if h.n.y < 0.0 {-h.n} else {h.n}, h.pos.y)), pitch);
    p.step_with_surface(&mut t, &controls, save.invert_pitch, surface);

    // Terrain / sea: crash and bail out with the parachute.
    let step = p.vel;
    let crashed = col.raycast(from, step.normalize_or_zero(), step.length() + 10.0).is_some_and(|h| h.flags & 0x1f != 6);
    if crashed {
        let (pl, pt, pv) = &mut *player;
        eject(pl, pt, from - step * 10.0 + Vec3::Y * 80.0, p.vel * 0.2);
        **pv = Visibility::Inherited;
        next.set(Mode::Walk);
        return;
    }
    t.translation += step;
    // Collision floor is still a plane; animated wave/float contacts remain partial.
    let water_height = water.map_or(5.0, |h| h.pos.y + 5.0);
    if t.translation.y < water_height { t.translation.y = water_height; p.vel.y = p.vel.y.max(0.0); }
}

fn guns(mut c: ResMut<Controls>, mut commands: Commands, assets: Res<AssetServer>, effects: Res<FlareEffects>, mut sounds: MessageWriter<crate::sound::Sound>, mut q: Query<(&mut Plane, &Transform)>) {
    let Ok((mut p, t)) = q.single_mut() else { return };
    p.gun_cooldown = p.gun_cooldown.saturating_sub(1);
    if c.shoot && p.gun_cooldown == 0 {
        p.gun_cooldown = 6;
        p.gun_side = !p.gun_side;
        let side = if p.gun_side { 14.0 } else { -14.0 };
        let pos = t.transform_point(Vec3::new(side, -2.0, 20.0));
        let vel = p.vel + t.rotation * Vec3::Z * 22.0;
        commands.spawn((
            // The original selects WS2_pln_billbord_bullet (scene 0), with its
            // bullet_image texture; scene 1 is an untextured solid sphere.
            WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset("models/bullet.glb"))),
            Transform::from_translation(pos), ProjectileSprite,
            Bullet { vel, life: 75 },
        ));
        sounds.write(crate::sound::Sound::effect("SE_PLN_SHOOT_BULLET", None));
    }
    if c.flare {
        c.flare = false;
        sounds.write(crate::sound::Sound::effect("SE_PLN_ILLUM_FLAME_PLAYER_LAUNCH", None));
        let (def, material) = &effects.parts["WS2_pln_sign_st"];
        commands.spawn((
            Name::new("Signal flare"),
            Transform::from_translation(t.translation + Vec3::Y * 10.0), Visibility::default(),
            PointLight { color: Color::srgba_u8(def.color[0], def.color[1], def.color[2], 255), intensity: 5.0e8, range: 3000.0, ..default() },
            Flare { vel: p.vel * 0.5 + Vec3::Y * 9.0, life: 360 },
        )).with_child((Mesh3d(effects.quad.clone()), MeshMaterial3d(material.clone()), Transform::from_scale(Vec3::new(def.size[0]*2.0, def.size[1]*2.0, 1.0)), FlareCore, ProjectileSprite));
    }
}

fn speedometer_spawn(mut commands: Commands, font: Res<crate::lang::UiFont>) {
    commands.spawn((Node { position_type: PositionType::Absolute, bottom: px(24), right: px(24), padding: UiRect::all(px(10)), ..default() }, BackgroundColor(Color::srgba(0.0, 0.1, 0.25, 0.7)), Text::new(""), TextFont::from(font.0.clone()).with_font_size(FontSize::Px(19.0)), TextColor(Color::WHITE), Speedometer));
}
fn speedometer(save: Res<Save>, texts: Res<crate::lang::Texts>, planes: Query<&Plane>, mut text: Single<(&mut Text, &mut Visibility), With<Speedometer>>) {
    let Ok(p) = planes.single() else { *text.1 = Visibility::Hidden; return; };
    *text.1 = if save.speedometer {Visibility::Inherited} else {Visibility::Hidden};
    use crate::lang::Ui;
    text.0.set_if_neq(Text::new(format!("{}: {:.0} {}\n{}: {:.0}%\n{}: {:.0}%", texts.ui(Ui::Speed), p.vel.length()*60.0, texts.ui(Ui::Units), texts.ui(Ui::Engine), p.throttle*100.0, texts.ui(if p.boost_sign >= 0.0 {Ui::Boost} else {Ui::Slow}), p.boost_force*100.0)));
}
fn load_flare_effects(mut commands: Commands, assets: Res<AssetServer>, mut meshes: ResMut<Assets<Mesh>>, mut materials: ResMut<Assets<StandardMaterial>>) {
    let definitions: std::collections::BTreeMap<String, FlareDef> = crate::read_json("effects/flare.json");
    let parts = definitions.into_iter().map(|(name, def)| {
        use bevy::image::{ImageSampler, ImageSamplerDescriptor, ImageAddressMode};
        let mirror = def.wrap == [2,2];
        let mode = |v| match v { 1 => ImageAddressMode::Repeat, 2 => ImageAddressMode::MirrorRepeat, _ => ImageAddressMode::ClampToEdge };
        let sampler = ImageSamplerDescriptor { address_mode_u: mode(def.wrap[0]), address_mode_v: mode(def.wrap[1]), ..ImageSamplerDescriptor::linear() };
        let texture = assets.load_builder().with_settings(move |s: &mut bevy::image::ImageLoaderSettings| s.sampler = ImageSampler::Descriptor(sampler.clone())).load(format!("effects/{}.png", def.texture));
        let material = materials.add(StandardMaterial {
            base_color: Color::srgba_u8(def.color[0], def.color[1], def.color[2], 255),
            base_color_texture: Some(texture),
            // REFF glow/ring assets store the quadrant centered at bottom left.
            uv_transform: if mirror {bevy::math::Affine2::from_scale_angle_translation(Vec2::splat(2.0), 0.0, Vec2::new(-1.0,0.0))} else {bevy::math::Affine2::IDENTITY},
            unlit: true, alpha_mode: if mirror {AlphaMode::Add} else {AlphaMode::Blend}, cull_mode: None, ..default()
        });
        (name, (def, material))
    }).collect();
    commands.insert_resource(FlareEffects { quad: meshes.add(Rectangle::new(1.0, 1.0)), parts });
}
fn projectile_billboards(cam: Single<&GlobalTransform, With<MainCamera>>, parents: Query<&GlobalTransform>, mut sprites: Query<(Option<&ChildOf>, &mut Transform), With<ProjectileSprite>>) {
    for (parent, mut t) in &mut sprites {
        t.rotation = parent.and_then(|p| parents.get(p.parent()).ok()).map_or(cam.rotation(), |p| p.rotation().inverse()*cam.rotation());
    }
}
fn flare_visuals(mut commands: Commands, time: Res<Time>, effects: Res<FlareEffects>, mut materials: ResMut<Assets<StandardMaterial>>, mut flares: Query<(&Flare, &Transform, &Children, &mut PointLight)>, mut cores: Query<(&mut Transform, &mut MeshMaterial3d<StandardMaterial>), (With<FlareCore>, Without<Flare>, Without<FlareTrail>)>, mut smoke: Query<(Entity, &mut FlareTrail, &mut Transform, &MeshMaterial3d<StandardMaterial>), (Without<Flare>, Without<FlareCore>)>, mut next: Local<f32>) {
    *next -= time.delta_secs();
    let emit = *next <= 0.0;
    if emit { *next = 0.08; }
    for (f, t, children, mut light) in &mut flares {
        let fade = (f.life as f32 / 60.0).min(1.0);
        light.intensity = 5.0e8 * fade;
        let phase = if f.life >= 359 { "WS2_pln_sign_st" } else if f.life > 300 { "WS2_pln_sign_tail" } else if f.life > 16 { "WS2_pln_sign_loop" } else { "WS2_pln_sign_end" };
        let (def, mat) = &effects.parts[phase];
        for child in children {
            if let Ok((mut transform, mut material)) = cores.get_mut(*child) {
                transform.scale = Vec3::new(def.size[0]*2.0, def.size[1]*2.0, 1.0)*fade;
                material.0 = mat.clone();
            }
        }
        if emit {
            let (def, mat) = &effects.parts[if f.life > 300 {"WS2_pln_sign_tail_smk"} else {"WS2_pln_sign_smk_bl_loop"}];
            let alpha = def.color[3] as f32/255.0*0.18;
            let mut material = materials.get(mat).unwrap().clone(); material.base_color.set_alpha(alpha);
            let lifetime = def.life.max(0.1);
            // ponytail: original textures/base parameters; the full REFF child,
            // TEV and keyframe curves still need a particle-engine rewrite.
            commands.spawn((Mesh3d(effects.quad.clone()), MeshMaterial3d(materials.add(material)), Transform::from_translation(t.translation).with_scale(Vec3::new(def.size[0]*2.0, def.size[1]*2.0, 1.0)), ProjectileSprite, FlareTrail { remaining: lifetime, lifetime, alpha, vel: Vec3::Y*6.0 }));
        }
    }
    for (e, mut smoke, mut t, mat) in &mut smoke {
        smoke.remaining -= time.delta_secs(); t.scale.x += time.delta_secs()*2.0; t.scale.y += time.delta_secs()*2.0; t.translation += smoke.vel*time.delta_secs();
        if let Some(mut material) = materials.get_mut(&mat.0) { material.base_color.set_alpha(smoke.alpha*(smoke.remaining/smoke.lifetime).max(0.0)); }
        if smoke.remaining <= 0.0 { commands.entity(e).despawn(); }
    }
}

fn projectiles(
    mut commands: Commands,
    col: Res<Collision>,
    mut shots: MessageWriter<Shot>,
    mut sounds: MessageWriter<crate::sound::Sound>,
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
        if f.life == 300 { sounds.write(crate::sound::Sound::effect("SE_PLN_ILLUM_FLAME_PLAYER_EXPLODE", Some(t.translation))); }
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
    fn water_departure_can_leave_the_surface_under_normal_controls() {
        let mut pose=Transform::from_xyz(0.0,5.0,0.0);
        let mut p=Plane::new(pose,2.5); p.flight=0.0; p.water_strength=1.0;
        let mut highest=pose.translation.y;
        for frame in 0..600 {
            let c=Controls {stick:if frame<180 {Vec2::new(0.0,-0.4)} else {Vec2::ZERO},boost:if frame<1 {1.0} else {0.0},..default()};
            let contact=(pose.translation.y<120.0).then_some((pose.translation.y,Vec3::Y,0.0));
            p.water_contact(&mut pose,contact,c.stick.y);
            p.step_with_surface(&mut pose,&c,false,None);
            pose.translation+=p.vel;
            if pose.translation.y<5.0 {pose.translation.y=5.0;p.vel.y=p.vel.y.max(0.0);}
            highest=highest.max(pose.translation.y);
        }
        assert!(highest>30.0,"boost and pull-back must take off; highest={highest}");
    }
    #[test]
    fn water_release_and_damping_match_the_recovered_bands() {
        let mut pose = Transform::from_xyz(0.0,5.0,0.0);
        let mut p = Plane::new(pose,3.0); p.water_strength=1.0;
        p.vel = Vec3::new(2.0,-1.0,3.0);
        p.water_contact(&mut pose,Some((5.0,Vec3::Y,0.0)),0.0);
        assert!((p.vel.x-1.0).abs()<1e-5 && (p.vel.y+0.9).abs()<1e-5);
        let contact=p.water_strength;
        for _ in 0..60 {p.water_contact(&mut pose,None,-1.0);}
        assert!(p.water_strength<contact*0.1,"pulling up away from contact releases the floats");
        assert!(pose.rotation.is_finite() && (pose.rotation.length()-1.0).abs()<1e-5);
    }
    #[test]
    fn ground_assistance_uses_surface_alignment_and_clearance() {
        let controls = Controls::default();
        for surface in [(120.0, Vec3::Y), (10.0, Vec3::X)] {
            let mut pose = Transform::IDENTITY;
            let mut plane = Plane::new(pose, 7.0);
            plane.step_with_surface(&mut pose, &controls, false, Some(surface));
            assert_eq!(plane.ground_strength, 0.0, "distant ground and walls must not push upward");
            assert_eq!(plane.vel.y, 0.0);
        }
        let normal = Vec3::new(0.5, 0.8660254, 0.0);
        let mut pose = Transform::IDENTITY;
        let mut plane = Plane::new(pose, 7.0);
        plane.step_with_surface(&mut pose, &controls, false, Some((30.0, normal)));
        assert!(plane.vel.x > 0.0 && plane.vel.y > 0.0, "banked surface must push along its normal");
        assert!((plane.vel.x / plane.vel.y - normal.x / normal.y).abs() < 1e-5);
        let strength = plane.ground_strength;
        plane.step(&mut pose, &controls, false);
        assert!(plane.ground_strength > 0.0 && plane.ground_strength < strength, "assistance fades after leaving the surface");
    }

    #[test]
    fn sustained_maneuvers_keep_an_orthonormal_attitude() {
        let mut pose = Transform::IDENTITY;
        let mut plane = Plane::new(pose, 7.0);
        for frame in 0..216_000 {
            let phase = frame as f32 * 0.013;
            plane.step(&mut pose, &Controls { stick: Vec2::new(phase.sin(), phase.cos()), ..default() }, false);
            assert!(plane.vel.is_finite() && plane.up_target.is_finite());
            assert!((pose.rotation.length() - 1.0).abs() < 1e-5);
            let nose = pose.rotation * Vec3::Z;
            assert!(nose.dot(plane.up_target).abs() < 1e-5);
        }
    }

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
    fn turn_recovery_setting_scales_the_swing_back_after_a_turn() {
        // Yaw reversal after releasing a full turn, for a given recovery strength.
        let reversal = |strength: f32| {
            let mut pose = Transform::IDENTITY;
            let mut plane = Plane::new(pose, 7.0);
            plane.turn_recovery = strength;
            let mut c = Controls { stick: Vec2::X, ..default() };
            let yaw = |p: &Transform| { let n = p.rotation * Vec3::Z; n.x.atan2(n.z) };
            for _ in 0..60 { plane.step(&mut pose, &c, false); }
            c.stick = Vec2::ZERO;
            // Accumulate wrapped deltas: without recovery the turn coasts past ±180°.
            let (mut total, mut lowest, mut prev) = (0.0f32, 0.0f32, yaw(&pose));
            for _ in 0..120 {
                plane.step(&mut pose, &c, false);
                let d = yaw(&pose) - prev;
                total += (d + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
                prev = yaw(&pose);
                lowest = lowest.min(total);
            }
            total - lowest
        };
        assert!(reversal(0.0) < 0.01, "no recovery must not swing back");
        assert!(reversal(1.0) > reversal(0.5) && reversal(0.5) > 0.01);
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
