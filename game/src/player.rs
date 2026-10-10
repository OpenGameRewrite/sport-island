//! On-foot exploration: a simple third-person character controller.

use crate::collide::Collision;
use crate::input::Controls;
use crate::world::MainCamera;
use crate::{AppState, Mode};
use bevy::camera::visibility::NoFrustumCulling;
use bevy::prelude::*;
use bevy::world_serialization::WorldInstanceReady;

const HEIGHT: f32 = 40.0;
const RADIUS: f32 = 10.0;
const GRAVITY: f32 = 1400.0;
/// Below this the player is in the sea; respawn at the last safe spot.
const SEA_LEVEL: f32 = -20.0;

#[derive(Component)]
pub struct Player {
    pub vel: Vec3,
    grounded: bool,
    safe: Vec3,
    /// Descending under a parachute after leaving the plane.
    pub parachute: bool,
}

#[derive(Component)]
struct Parachute;

/// Orbit camera angles shared by walking and flying.
#[derive(Resource)]
pub struct CamRig {
    pub yaw: f32,
    pub pitch: f32,
}

pub fn plugin(app: &mut App) {
    app.insert_resource(CamRig { yaw: 0.0, pitch: -0.25 })
        .add_systems(OnEnter(AppState::Play), spawn)
        .add_systems(Update, (walk, camera).chain().run_if(crate::menu::playing.and_then(in_state(Mode::Walk))))
        .add_systems(Update, parachute_visual.run_if(in_state(AppState::Play)));
}

/// Start next to the Wishing Fountain in town.
pub const START: Vec3 = Vec3::new(-1880.0, 400.0, 2180.0);

fn spawn(mut commands: Commands, assets: Res<AssetServer>, col: Res<Collision>, existing: Query<(), With<Player>>, mut meshes: ResMut<Assets<Mesh>>, mut materials: ResMut<Assets<crate::gx::GxMaterial>>, globals: Res<crate::gx::GxGlobals>) {
    if !existing.is_empty() {
        return;
    }
    let y = col.ground(START, 2000.0, 4000.0).map_or(START.y, |h| h.pos.y);
    let p = Vec3::new(START.x, y, START.z);
    commands
        .spawn((Player { vel: Vec3::ZERO, grounded: true, safe: p, parachute: false }, Transform::from_translation(p), Visibility::default()))
        .with_children(|c| {
            body(c, &mut meshes, &mut materials, &globals, false);
            c.spawn((
                WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset("models/parachute.glb"))),
                Transform::from_xyz(0.0, HEIGHT, 0.0),
                Visibility::Hidden,
                Parachute,
            ));
        });
}

/// Skinned meshes keep their bind-pose bounds; never cull them.
pub fn no_culling(ev: On<WorldInstanceReady>, children: Query<&Children>, meshes: Query<(), With<Mesh3d>>, mut commands: Commands) {
    for e in children.iter_descendants(ev.entity) {
        if meshes.contains(e) {
            commands.entity(e).insert(NoFrustumCulling);
        }
    }
}

fn walk(c: Res<Controls>, rig: Res<CamRig>, col: Res<Collision>, time: Res<Time>, mut q: Query<(&mut Player, &mut Transform)>) {
    let dt = time.delta_secs().min(0.05);
    for (mut p, mut t) in &mut q {
        let fwd = Vec3::new(-rig.yaw.sin(), 0.0, -rig.yaw.cos());
        let right = Vec3::new(fwd.z * -1.0, 0.0, fwd.x);
        let wish = (fwd * c.stick.y + right * c.stick.x).clamp_length_max(1.0);
        let speed = if p.parachute { 150.0 } else if c.run { 320.0 } else { 140.0 };
        let accel = if p.grounded { 12.0 } else { 3.0 };
        let target = wish * speed;
        let h = Vec3::new(p.vel.x, 0.0, p.vel.z).lerp(target, (accel * dt).min(1.0));
        p.vel.x = h.x;
        p.vel.z = h.z;
        if p.grounded && c.jump {
            p.vel.y = 520.0;
            p.grounded = false;
        }
        p.vel.y -= GRAVITY * dt * if p.parachute { 0.15 } else { 1.0 };
        if p.parachute {
            p.vel.y = p.vel.y.max(-160.0);
        }
        let mut pos = t.translation + p.vel * dt;
        // Walls: push a sphere at waist height out of steep triangles.
        let waist = col.push_sphere(pos + Vec3::Y * (HEIGHT * 0.5), RADIUS, 0.7);
        pos = waist - Vec3::Y * (HEIGHT * 0.5);
        // Floor: snap to walkable ground when close.
        p.grounded = false;
        if let Some(hit) = col.ground(pos, HEIGHT * 0.5, if p.vel.y <= 0.0 { 12.0 } else { 0.0 })
            && hit.n.y > 0.5
            && p.vel.y <= 0.0
        {
            pos.y = hit.pos.y;
            p.vel.y = 0.0;
            p.grounded = true;
            p.parachute = false;
            p.safe = pos;
        }
        if pos.y < SEA_LEVEL {
            pos = p.safe;
            p.vel = Vec3::ZERO;
        }
        t.translation = pos;
        if wish.length_squared() > 0.01 {
            let yaw = (-wish.x).atan2(-wish.z) + std::f32::consts::PI;
            t.rotation = t.rotation.slerp(Quat::from_rotation_y(yaw), (10.0 * dt).min(1.0));
        }
    }
}

fn parachute_visual(players: Query<(&Player, &Children)>, mut chutes: Query<&mut Visibility, With<Parachute>>) {
    for (p, children) in &players {
        for c in children.iter() {
            if let Ok(mut v) = chutes.get_mut(c) {
                *v = if p.parachute { Visibility::Inherited } else { Visibility::Hidden };
            }
        }
    }
}

fn camera(
    c: Res<Controls>,
    mut rig: ResMut<CamRig>,
    col: Res<Collision>,
    time: Res<Time>,
    player: Single<&Transform, With<Player>>,
    mut cam: Single<&mut Transform, (With<MainCamera>, Without<Player>)>,
) {
    rig.yaw -= c.look.x;
    rig.pitch = (rig.pitch - c.look.y).clamp(-1.2, 0.6);
    let target = player.translation + Vec3::Y * HEIGHT * 1.1;
    // Camera sits behind and above the player (pitch < 0 looks down).
    let dir = Vec3::new(rig.yaw.sin() * rig.pitch.cos(), -rig.pitch.sin(), rig.yaw.cos() * rig.pitch.cos());
    let mut dist = 180.0;
    if let Some(h) = col.raycast(target, dir, dist) {
        dist = (h.t - 10.0).max(20.0);
    }
    let want = target + dir * dist;
    let k = (12.0 * time.delta_secs()).min(1.0);
    cam.translation = cam.translation.lerp(want, k);
    cam.look_at(target, Vec3::Y);
}

/// The same original character, seated in a cockpit or standing on foot.
/// The explorer: an original, Mii-inspired figure (big round head, ball hands)
/// with its own face, collar and shoes. Feet at y=0, facing +Z, about 52 tall.
pub fn body(c: &mut ChildSpawnerCommands, meshes: &mut Assets<Mesh>, materials: &mut Assets<crate::gx::GxMaterial>, globals: &crate::gx::GxGlobals, seated: bool) {
    let mut mat = |r: u8, g: u8, b: u8| materials.add(crate::gx::shaded(Color::srgb_u8(r, g, b), globals.0.clone()));
    let shirt = mat(30, 130, 215);
    let collar = mat(240, 240, 235);
    let pants = mat(52, 58, 82);
    let shoes = mat(120, 72, 40);
    let skin = mat(250, 205, 165);
    let hair = mat(70, 45, 30);
    let ink = mat(30, 28, 32);
    let white = mat(255, 255, 255);
    let mouth = mat(170, 60, 60);
    let blush = mat(250, 170, 160);
    let mut part = |mesh: Mesh, material: &Handle<crate::gx::GxMaterial>, t: Transform| {
        c.spawn((Mesh3d(meshes.add(mesh)), MeshMaterial3d(material.clone()), t));
    };
    use std::f32::consts::FRAC_PI_2;
    let at = |x: f32, y: f32, z: f32| Transform::from_xyz(x, y, z);

    // Head: a slightly tall ellipsoid; face features sit on its surface.
    let head = Vec3::new(0.0, 40.5, 0.0);
    let radii = Vec3::new(9.6, 10.2, 9.0);
    let surface = |x: f32, y: f32| head + Vec3::new(x, y, radii.z * (1.0 - (x / radii.x).powi(2) - (y / radii.y).powi(2)).max(0.0).sqrt());
    part(Sphere::new(1.0).mesh().uv(32, 24), &skin, Transform::from_translation(head).with_scale(radii));
    // Hair: a cap set back and up so the forehead shows, with side tufts.
    part(Sphere::new(1.0).mesh().uv(32, 24), &hair, Transform::from_translation(head + Vec3::new(0.0, 3.6, -1.6)).with_scale(Vec3::new(10.1, 8.0, 9.6)));
    for x in [-1.0, 1.0] {
        part(Sphere::new(1.0).mesh().uv(16, 12), &hair, Transform::from_translation(head + Vec3::new(x * 8.0, 1.5, -1.0)).with_scale(Vec3::new(2.6, 5.0, 5.5)));
        part(Sphere::new(1.0).mesh().uv(12, 8), &skin, Transform::from_translation(head + Vec3::new(x * 9.4, -1.0, 0.5)).with_scale(Vec3::new(1.2, 2.2, 1.6)));
        // Round eyes with a highlight, brows, cheeks.
        let eye = surface(x * 3.2, 0.6);
        part(Sphere::new(1.0).mesh().uv(16, 12), &ink, Transform::from_translation(eye).with_scale(Vec3::new(1.25, 1.55, 0.7)));
        part(Sphere::new(0.45).mesh().uv(8, 6), &white, at(eye.x + 0.35, eye.y + 0.55, eye.z + 0.55));
        let brow = surface(x * 3.4, 3.6);
        part(Capsule3d::new(0.42, 2.2).into(), &hair, Transform::from_translation(brow + Vec3::Z * 0.2).with_rotation(Quat::from_rotation_z(FRAC_PI_2 - x * 0.12)));
        let cheek = surface(x * 5.4, -2.6);
        part(Sphere::new(1.0).mesh().uv(12, 8), &blush, Transform::from_translation(cheek).with_scale(Vec3::new(1.5, 0.9, 0.35)));
    }
    part(Sphere::new(1.0).mesh().uv(12, 8), &skin, Transform::from_translation(surface(0.0, -1.6)).with_scale(Vec3::new(1.2, 1.0, 1.1)));
    // Smile: a thin ring whose upper half sinks into the head.
    let smile = surface(0.0, -5.4);
    part(Torus::new(2.0, 2.6).into(), &mouth, at(0.0, smile.y + 2.2, smile.z + 0.15).with_rotation(Quat::from_rotation_x(FRAC_PI_2)));
    part(Cylinder::new(2.6, 4.0).into(), &skin, at(0.0, 30.5, 0.0));

    // Torso with a white collar.
    part(Capsule3d::new(6.2, 7.0).into(), &shirt, at(0.0, 22.5, 0.0).with_scale(Vec3::new(1.05, 1.0, 0.82)));
    part(Torus::new(2.4, 4.2).into(), &collar, at(0.0, 28.6, 0.2).with_scale(Vec3::new(1.0, 1.0, 0.9)));
    part(Sphere::new(1.0).mesh().uv(24, 16), &pants, at(0.0, 15.5, 0.0).with_scale(Vec3::new(6.6, 4.2, 5.4)));

    for x in [-1.0, 1.0] {
        // Arms: sleeve, forearm, ball hand. Seated, they reach for the stick.
        let (shoulder, tilt) = if seated { (Vec3::new(x * 7.2, 26.0, 0.5), Quat::from_rotation_x(-1.0)) } else { (Vec3::new(x * 7.4, 26.0, 0.0), Quat::from_rotation_z(x * 0.22)) };
        let down = tilt * Vec3::NEG_Y;
        part(Capsule3d::new(2.3, 4.5).into(), &shirt, Transform::from_translation(shoulder + down * 3.5).with_rotation(tilt));
        part(Capsule3d::new(1.9, 4.5).into(), &skin, Transform::from_translation(shoulder + down * 9.0).with_rotation(tilt));
        part(Sphere::new(2.5).mesh().uv(12, 8), &skin, Transform::from_translation(shoulder + down * 13.0));
        // Legs and rounded shoes.
        if seated {
            part(Capsule3d::new(2.7, 7.0).into(), &pants, at(x * 3.4, 14.5, 5.0).with_rotation(Quat::from_rotation_x(FRAC_PI_2)));
            part(Capsule3d::new(2.5, 5.0).into(), &pants, at(x * 3.4, 9.5, 10.5));
            part(Sphere::new(1.0).mesh().uv(16, 12), &shoes, at(x * 3.4, 4.2, 12.0).with_scale(Vec3::new(3.0, 2.2, 4.4)));
        } else {
            part(Capsule3d::new(2.7, 8.0).into(), &pants, at(x * 3.4, 8.8, 0.0));
            part(Sphere::new(1.0).mesh().uv(16, 12), &shoes, at(x * 3.4, 2.2, 1.2).with_scale(Vec3::new(3.0, 2.2, 4.4)));
        }
    }
}
