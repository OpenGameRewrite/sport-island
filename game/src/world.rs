//! Wuhu Island, sky domes and the three original times of day.

use crate::save::Save;
use crate::gx::GxEnv;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::prelude::*;
use serde::Deserialize;

#[derive(Deserialize, Clone)]
struct LightDef {
    pos: Vec3,
    aim: Vec3,
    color: [u8; 4],
}

#[derive(Deserialize, Clone)]
struct FogDef {
    start: f32,
    end: f32,
    color: [u8; 4],
}

#[derive(Deserialize, Clone)]
struct EnvFile {
    lights: Vec<LightDef>,
    ambient: Vec<[u8; 4]>,
    fog: Vec<FogDef>,
}

#[derive(Resource)]
struct Envs(Vec<EnvFile>);

/// 0 day, 1 dusk, 2 night (the three Island Flyover settings).
#[derive(Resource, Clone, Copy, PartialEq)]
pub struct TimeOfDay(pub u8);

#[derive(Component)]
pub struct MainCamera;

/// Sky dome for one time of day.
#[derive(Component)]
struct Sky(u8);

/// Lit windows and lamps shown at night.
#[derive(Component)]
struct NightLights;

pub fn rgb(c: [u8; 4]) -> Color { Color::srgb_u8(c[0], c[1], c[2]) }

pub fn plugin(app: &mut App) {
    let envs = (0..3).map(|i| crate::read_json(&format!("data/env{i}.json"))).collect();
    let time = app.world().resource::<Save>().time.min(2);
    app.insert_resource(Envs(envs))
        .insert_resource(TimeOfDay(time))
        .insert_resource(ClearColor(Color::srgb(0.4, 0.8, 1.0)))
        .add_systems(Startup, setup)
        .add_systems(Update, (apply_time.run_if(resource_changed::<TimeOfDay>), sky_follow, cycle_time.run_if(crate::menu::playing)));
}

fn island_scene(assets: &AssetServer, i: usize) -> WorldAssetRoot {
    WorldAssetRoot(assets.load(GltfAssetLabel::Scene(i).from_asset("models/island.glb")))
}

fn setup(mut commands: Commands, assets: Res<AssetServer>) {
    // island.glb scenes: island, island_L, island_R, island_V, sea, under, vr, vr_E, vr_N.
    // island_R duplicates the island for the water reflection pass; not needed here.
    for i in [0, 3, 4, 5] {
        commands.spawn(island_scene(&assets, i));
    }
    commands.spawn((island_scene(&assets, 1), NightLights, Visibility::Hidden));
    for (t, scene) in [(0u8, 6usize), (1, 7), (2, 8)] {
        commands.spawn((island_scene(&assets, scene), Sky(t), Visibility::Hidden));
    }
    commands.spawn((
        Camera3d::default(),
        Projection::Perspective(PerspectiveProjection { far: 300_000.0, near: 2.0, fov: 55f32.to_radians(), ..default() }),
        Transform::from_xyz(0.0, 3000.0, 8000.0).looking_at(Vec3::ZERO, Vec3::Y),
        Tonemapping::None,
        MainCamera,
    ));
}

fn apply_time(
    time: Res<TimeOfDay>,
    envs: Res<Envs>,
    mut gx: ResMut<GxEnv>,
    mut clear: ResMut<ClearColor>,
    mut skies: Query<(&Sky, &mut Visibility), Without<NightLights>>,
    mut night: Query<&mut Visibility, With<NightLights>>,
) {
    let env = &envs.0[time.0 as usize];
    let l = &env.lights[0];
    let v = |c: [u8; 4]| Vec3::new(c[0] as f32, c[1] as f32, c[2] as f32) / 255.0;
    gx.sun_dir = (l.aim - l.pos).normalize();
    gx.sun_color = v(l.color);
    gx.ambient = v(env.ambient[0]);
    if let Some(f) = env.fog.iter().find(|f| f.end > f.start) {
        gx.fog_color = v(f.color);
        gx.fog = Vec2::new(f.start, f.end) * 3.0;
        clear.0 = rgb(f.color);
    }
    for (s, mut vis) in &mut skies {
        *vis = if s.0 == time.0 { Visibility::Inherited } else { Visibility::Hidden };
    }
    for mut vis in &mut night {
        *vis = if time.0 == 2 { Visibility::Inherited } else { Visibility::Hidden };
    }
}

fn sky_follow(cam: Single<&Transform, With<MainCamera>>, mut skies: Query<&mut Transform, (With<Sky>, Without<MainCamera>)>) {
    for mut t in &mut skies {
        t.translation = Vec3::new(cam.translation.x, 0.0, cam.translation.z);
    }
}

fn cycle_time(c: Res<crate::input::Controls>, mut time: ResMut<TimeOfDay>, mut save: ResMut<Save>) {
    if c.time {
        time.0 = (time.0 + 1) % 3;
        save.time = time.0;
        save.store();
    }
}
