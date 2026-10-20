//! Wuhu Island, sky domes and the three original times of day.

use crate::save::Save;
use crate::gx::GxEnv;
use bevy::core_pipeline::tonemapping::{Tonemapping, DebandDither};
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
    /// GX fog type; 0 is off, 2 perspective linear.
    kind: u8,
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

/// The game's per-mode table of island sections (bones) to show, by name.
/// The flyover uses mode 13 by day and dusk and mode 14 at night; the
/// sections it hides overlap the ones it shows (e.g. the volcano tunnel).
#[derive(Resource, Default)]
struct SectionTable(std::collections::HashMap<String, Vec<bool>>);

/// 0 day, 1 dusk, 2 night (the three Island Flyover settings).
#[derive(Resource, Clone, Copy, PartialEq)]
pub struct TimeOfDay(pub u8);

#[derive(Component)]
pub struct MainCamera;

/// Base vertical field of view (55°) used across camera setups and flight.
pub const BASE_FOV_DEGREES: f32 = 55.0;

/// Sky dome for one time of day.
#[derive(Component)]
pub(crate) struct Sky(u8);

/// Lit windows and lamps shown at night.
#[derive(Component)]
struct NightLights;

pub fn rgb(c: [u8; 4]) -> Color { Color::srgb_u8(c[0], c[1], c[2]) }

pub fn plugin(app: &mut App) {
    let envs = (0..3).map(|i| crate::read_json(&format!("data/env{i}.json"))).collect();
    let time = app.world().resource::<Save>().time.min(2);
    let table = std::fs::read(crate::asset_dir().join("data/island_visibility.json")).ok().and_then(|b| serde_json::from_slice(&b).ok());
    if table.is_none() { warn!("data/island_visibility.json missing: run the extractor again to hide overlapping island sections"); }
    app.insert_resource(Envs(envs))
        .insert_resource(SectionTable(table.unwrap_or_default()))
        .add_systems(Update, sections)
        .insert_resource(TimeOfDay(time))
        .insert_resource(ClearColor(Color::srgb(0.4, 0.8, 1.0)))
        .add_systems(Startup, setup)
        .add_systems(Update, (apply_time.run_if(resource_changed::<TimeOfDay>), cycle_time.run_if(crate::menu::playing), apply_msaa.run_if(resource_changed::<Save>)))
        // After every camera system has moved the camera this frame.
        .add_systems(PostUpdate, sky_follow.before(TransformSystems::Propagate));
}

fn island_scene(assets: &AssetServer, i: usize) -> WorldAssetRoot {
    WorldAssetRoot(assets.load(GltfAssetLabel::Scene(i).from_asset("models/island.glb")))
}

fn setup(mut commands: Commands, assets: Res<AssetServer>, save: Res<Save>) {
    // island.glb scenes: island, island_L, island_R, island_V, sea, under, vr, vr_E, vr_N.
    // island_R duplicates the island for the water reflection pass; not needed here.
    commands.spawn(island_scene(&assets, 0)).observe(hide_crater_lid);
    for i in [3, 4, 5] {
        commands.spawn(island_scene(&assets, i));
    }
    commands.spawn((island_scene(&assets, 1), NightLights, Visibility::Hidden));
    for (t, scene) in [(0u8, 6usize), (1, 7), (2, 8)] {
        commands.spawn((island_scene(&assets, scene), Sky(t), Visibility::Hidden));
    }
    commands.spawn((
        Camera3d::default(),
        crate::save::msaa_sample(save.msaa),
        Projection::Perspective(PerspectiveProjection { far: 300_000.0, near: 2.0, fov: BASE_FOV_DEGREES.to_radians(), ..default() }),
        Transform::from_xyz(0.0, 3000.0, 8000.0).looking_at(Vec3::ZERO, Vec3::Y),
        Tonemapping::None,
        DebandDither::Disabled,
        MainCamera,
        SpatialListener::new(20.0),
    ));
}

fn apply_msaa(save: Res<Save>, mut cam: Query<&mut Msaa, With<MainCamera>>) {
    for mut msaa in &mut cam {
        let target = crate::save::msaa_sample(save.msaa);
        if *msaa != target {
            *msaa = target;
        }
    }
}

/// A flat, black-vertex-coloured disc caps the volcano's chimney halfway down,
/// hiding the open shaft to the lava chamber. Flying through it showed a black
/// box, so it is removed and the shaft stays open.
fn hide_crater_lid(ev: On<bevy::world_serialization::WorldInstanceReady>, children: Query<&Children>, q: Query<(&Mesh3d, &bevy::gltf::GltfMaterialName)>, meshes: Res<Assets<Mesh>>, mut commands: Commands) {
    for e in children.iter_descendants(ev.entity) {
        let Ok((mesh, name)) = q.get(e) else { continue };
        if name.0 != "mt_gake" { continue; }
        let Some(mesh) = meshes.get(&mesh.0) else { continue };
        let (Some(bevy::mesh::VertexAttributeValues::Float32x3(pos)), Some(bevy::mesh::VertexAttributeValues::Float32x4(col))) =
            (mesh.attribute(Mesh::ATTRIBUTE_POSITION), mesh.attribute(Mesh::ATTRIBUTE_COLOR)) else { continue };
        let flat = pos.iter().all(|p| (p[1] - pos[0][1]).abs() < 0.5);
        if flat && col.iter().all(|c| c[0] + c[1] + c[2] == 0.0) {
            commands.entity(e).insert(Visibility::Hidden);
        }
    }
}

fn sections(time: Res<TimeOfDay>, table: Res<SectionTable>, mut nodes: Query<(Ref<Name>, &mut Visibility)>) {
    let mode = if time.0 == 2 { 14 } else { 13 };
    for (name, mut vis) in &mut nodes {
        if !(name.is_added() || time.is_changed()) { continue; }
        if let Some(show) = table.0.get(name.as_str()).and_then(|row| row.get(mode)) {
            vis.set_if_neq(if *show { Visibility::Inherited } else { Visibility::Hidden });
        }
    }
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
    for (i, f) in env.fog.iter().take(2).enumerate() {
        gx.fogs[i] = (v(f.color), Vec3::new(f.start, f.end, f.kind as f32));
    }
    if let Some(f) = env.fog.iter().find(|f| f.end > f.start) {
        clear.0 = rgb(f.color);
    }
    for (s, mut vis) in &mut skies {
        *vis = if s.0 == time.0 { Visibility::Inherited } else { Visibility::Hidden };
    }
    for mut vis in &mut night {
        *vis = if time.0 == 2 { Visibility::Inherited } else { Visibility::Hidden };
    }
}

pub(crate) fn sky_follow(cam: Single<&Transform, With<MainCamera>>, mut skies: Query<&mut Transform, (With<Sky>, Without<MainCamera>)>) {
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
