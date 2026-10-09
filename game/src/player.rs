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

fn spawn(mut commands: Commands, assets: Res<AssetServer>, col: Res<Collision>, existing: Query<(), With<Player>>) {
    if !existing.is_empty() {
        return;
    }
    let y = col.ground(START, 2000.0, 4000.0).map_or(START.y, |h| h.pos.y);
    let p = Vec3::new(START.x, y, START.z);
    commands
        .spawn((Player { vel: Vec3::ZERO, grounded: true, safe: p, parachute: false }, Transform::from_translation(p), Visibility::default()))
        .with_children(|c| {
            c.spawn((
                WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset("models/walker.glb"))),
                Transform::from_scale(Vec3::splat(3.5)),
            ))
            .observe(no_culling);
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
