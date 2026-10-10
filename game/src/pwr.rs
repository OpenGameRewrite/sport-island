//! Optional Pilotwings Resort add-on, active when `assets/pwr` was extracted
//! from the player's own cartridge dump. Without it the game is unchanged.
//!
//! Pilotwings Resort models Wuhu Island in metres, mirrored on Z against the
//! Wii game: a Wii position is `(x, y, -z) * 10` of the 3DS one.

use crate::ipoint::{IPointDef, IPoints, Marker, MarkerFade};
use crate::AppState;
use bevy::gltf::GltfMaterialExtras;
use bevy::prelude::*;
use bevy::world_serialization::WorldInstanceReady;
use serde_json::Value;
use std::collections::HashMap;

pub const SCALE: f32 = 10.0;

pub fn available() -> bool { crate::asset_dir().join("pwr/VERSION").exists() }

pub fn world(p: [f32; 3]) -> Vec3 { Vec3::new(p[0], p[1], -p[2]) * SCALE }

/// A 3DS object frame (three basis rows, then the origin) in Wii space.
pub fn frame(f: &[f32]) -> Transform {
    let mirror = Mat3::from_diagonal(Vec3::new(1.0, 1.0, -1.0));
    let basis = Mat3::from_cols(Vec3::from_slice(&f[0..3]), Vec3::from_slice(&f[3..6]), Vec3::from_slice(&f[6..9]));
    Transform { translation: world([f[9], f[10], f[11]]), rotation: Quat::from_mat3(&(mirror * basis * mirror)).normalize(), scale: Vec3::splat(SCALE) }
}

fn data(rel: &str) -> Value {
    std::fs::read(crate::asset_dir().join("pwr/data").join(rel)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(Value::Null)
}

fn floats(v: &Value) -> Vec<f32> { v.as_array().into_iter().flatten().filter_map(|x| x.as_f64()).map(|x| x as f32).collect() }

/// `object` entries of a placement table such as `def_objects.val`.
fn objects(table: &Value) -> Vec<Value> { table["objects"][0]["object"].as_array().cloned().unwrap_or_default() }

/// Eyepoint id of the i-point the landing-pad blimp carries.
const BLIMP_POINT: u32 = 100;

/// Links shared i-points to their 3DS counterparts and appends the points
/// Pilotwings Resort added, unless one would sit on top of a Wii point.
pub fn add_ipoints(defs: &mut Vec<IPointDef>) {
    if !available() { return; }
    let wii = defs.len();
    for o in objects(&data("wuhu/eyepoint.val.json")) {
        let Some(id) = o["name"].as_str().and_then(|n| n.parse::<u32>().ok()) else { continue };
        let f = floats(&o["unfloat_frame64"]);
        if f.len() < 12 { continue; }
        if (id as usize) < wii {
            defs[id as usize].pwr = Some(id);
            continue;
        }
        // The Meca Hawk point rides a creature this add-on does not spawn yet.
        if o["counts_toward_total"] == Value::Bool(false) { continue; }
        let pos = world([f[9], f[10], f[11]]);
        if defs[..wii].iter().any(|d| d.kind == 0 && d.pos.distance(pos) < 300.0) { continue; }
        let carried = id == BLIMP_POINT;
        defs.push(IPointDef { radius: 500.0, kind: if carried { 3 } else { 0 }, msg: 0, pos, pwr: Some(id) });
    }
}

pub fn plugin(app: &mut App) {
    if !available() { return; }
    app.init_resource::<Scrolling>()
        .add_systems(OnEnter(AppState::Play), spawn)
        .add_systems(Update, (convert_materials, follow_path, spin, scroll));
}

/// Materials whose texture drifts (`tex_anim_u/v`, texture widths per second).
#[derive(Resource, Default)]
struct Scrolling(HashMap<AssetId<crate::gx::GxMaterial>, Vec2>);

fn scroll(time: Res<Time>, scrolling: Res<Scrolling>, mut gx: ResMut<Assets<crate::gx::GxMaterial>>) {
    let t = time.elapsed_secs();
    for (&id, rate) in &scrolling.0 {
        let offset = (*rate * t).fract_gl();
        if let Some(mut m) = gx.get_mut(id) { m.set_srt(0, [1.0, 1.0, 0.0, offset.x, offset.y]); }
    }
}

#[derive(Component)]
struct PwrObject;
#[derive(Component)]
struct PathFollower { points: Vec<Vec3>, distance: f32, speed: f32 }
#[derive(Component)]
struct Spin(f32);

fn model(assets: &AssetServer, name: &str) -> WorldAssetRoot {
    WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(format!("pwr/models/race_{}.glb", name.trim_end_matches(".mod").to_ascii_lowercase()))))
}

fn spawn(mut commands: Commands, assets: Res<AssetServer>, points: Res<IPoints>, existing: Query<(), With<PwrObject>>) {
    if !existing.is_empty() { return; }
    let dynos: HashMap<String, String> = data("race_/obstacle.val.json")["dyno"].as_array().into_iter().flatten()
        .filter_map(|d| Some((d["name"].as_str()?.to_string(), d["model"].as_str()?.to_string()))).collect();
    // The free-flight course layout is the same in all three free-flight modes.
    for o in objects(&data("wuhu_free_plane/def_objects.val.json")) {
        let Some(kind) = o["dyno"].as_str() else { continue };
        if !matches!(kind, "glider_landing_pad" | "water_landing_pad" | "turbo_ring" | "blimp_landing_pad") { continue; }
        let (Some(name), f) = (dynos.get(kind), floats(&o["unfloat_frame64"])) else { continue };
        if f.len() < 12 { continue; }
        let t = frame(&f);
        if kind == "blimp_landing_pad" {
            spawn_blimp(&mut commands, &assets, &points, t, name);
            continue;
        }
        commands.spawn((model(&assets, name), t, Visibility::default(), PwrObject, DespawnOnExit(AppState::Play)));
    }
}

/// The landing-pad blimp loops its route carrying i-point 100.
fn spawn_blimp(commands: &mut Commands, assets: &AssetServer, points: &IPoints, start: Transform, model_name: &str) {
    let mut path: Vec<(i64, Vec3)> = objects(&data("wuhu_free_plane/paths.val.json")).iter()
        .filter(|o| o["name"].as_str() == Some("blimp_landing_pad"))
        .filter_map(|o| { let f = floats(&o["unfloat_frame64"]); (f.len() >= 12).then(|| (o["idx"].as_i64().unwrap_or(0), world([f[9], f[10], f[11]]))) })
        .collect();
    path.sort_by_key(|p| p.0);
    let speed = 7.5 * SCALE;
    let root = commands.spawn((Transform::from_translation(start.translation).with_rotation(start.rotation), Visibility::default(), PwrObject, DespawnOnExit(AppState::Play))).id();
    if path.len() >= 2 {
        commands.entity(root).insert(PathFollower { points: path.into_iter().map(|p| p.1).collect(), distance: 0.0, speed });
    }
    commands.entity(root).with_children(|c| {
        c.spawn((model(assets, model_name), Transform::from_scale(Vec3::splat(SCALE)), Visibility::default())).observe(attach_props);
    });
    if let Some(i) = points.0.iter().position(|p| p.pwr == Some(BLIMP_POINT) && p.kind == 3) {
        let offset = start.rotation.inverse() * (points.0[i].pos - start.translation);
        commands.entity(root).with_children(|c| {
            c.spawn((WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset("models/ipoint.glb"))), Transform::from_translation(offset), Marker(i), MarkerFade::default())).observe(crate::ipoint::attach_visuals);
        });
    }
}

fn attach_props(ready: On<WorldInstanceReady>, children: Query<&Children>, names: Query<&Name>, assets: Res<AssetServer>, mut commands: Commands) {
    for e in children.iter_descendants(ready.entity) {
        let Ok(name) = names.get(e) else { continue };
        let dir = match name.as_str() { "left_prop" => 1.0, "right_prop" => -1.0, _ => continue };
        commands.entity(e).with_children(|c| { c.spawn((model(&assets, "BlmpProp"), Transform::default(), Visibility::default(), Spin(dir * 12.0))); });
    }
}

fn follow_path(time: Res<Time>, mut q: Query<(&mut PathFollower, &mut Transform)>) {
    for (mut p, mut t) in &mut q {
        let n = p.points.len();
        let lengths: Vec<f32> = (0..n).map(|i| p.points[i].distance(p.points[(i + 1) % n])).collect();
        let total: f32 = lengths.iter().sum();
        if total <= 0.0 { continue; }
        p.distance = (p.distance + p.speed * time.delta_secs()) % total;
        let (mut d, mut i) = (p.distance, 0);
        while d > lengths[i] { d -= lengths[i]; i += 1; }
        let (a, b) = (p.points[i], p.points[(i + 1) % n]);
        t.translation = a.lerp(b, d / lengths[i].max(1e-3));
        let dir = (b - a).with_y(0.0).normalize_or_zero();
        if dir != Vec3::ZERO {
            let target = Transform::IDENTITY.looking_to(-dir, Vec3::Y).rotation;
            t.rotation = t.rotation.slerp(target, (time.delta_secs() * 0.8).min(1.0));
        }
    }
}

fn spin(time: Res<Time>, mut q: Query<(&Spin, &mut Transform)>) {
    for (s, mut t) in &mut q { t.rotate_local_z(s.0 * time.delta_secs()); }
}

/// The 12 Mii favourite colours, in `colortex.val` order, that liveries are painted in.
pub fn favorites() -> Vec<String> {
    data("liveries.json")["favorites"].as_array().into_iter().flatten().filter_map(|f| f["name"].as_str().map(str::to_owned)).collect()
}

/// The add-on's models carry plain glTF materials; give them the island's
/// GX lighting and fog. Wii materials carry `extras.gx` and are skipped.
/// Livery materials (`extras.livery`) show the chosen favourite colour.
fn convert_materials(
    mut commands: Commands,
    q: Query<(Entity, &MeshMaterial3d<StandardMaterial>, Option<&GltfMaterialExtras>)>,
    standard: Res<Assets<StandardMaterial>>,
    globals: Res<crate::gx::GxGlobals>,
    save: Res<crate::save::Save>,
    assets: Res<AssetServer>,
    mut gx: ResMut<Assets<crate::gx::GxMaterial>>,
    mut favorites: Local<Option<Vec<String>>>,
    mut scrolling: ResMut<Scrolling>,
    mut cache: Local<HashMap<(AssetId<StandardMaterial>, usize), Handle<crate::gx::GxMaterial>>>,
) {
    for (e, handle, extras) in &q {
        let extras: Value = extras.and_then(|x| serde_json::from_str(&x.value).ok()).unwrap_or(Value::Null);
        if !extras["gx"].is_null() { continue; }
        let Some(m) = standard.get(&handle.0) else { continue };
        let livery = extras["livery"].as_str();
        let favorite = if livery.is_some() { save.favorite as usize } else { usize::MAX };
        let h = cache.entry((handle.0.id(), favorite)).or_insert_with(|| {
            let names = favorites.get_or_insert_with(self::favorites);
            let texture = match (livery, names.get(favorite)) {
                (Some(v), Some(name)) => Some(assets.load(format!("pwr/textures/{v}_{name}.png"))),
                _ => m.base_color_texture.clone(),
            };
            let alpha = if extras["blend"] == "add" { AlphaMode::Add } else { m.alpha_mode };
            gx.add(crate::gx::textured(texture, alpha, globals.0.clone()))
        }).clone();
        if let Some([u, v]) = extras["scroll"].as_array().map(|a| [0, 1].map(|i| a.get(i).and_then(Value::as_f64).unwrap_or(0.0) as f32)) {
            scrolling.0.insert(h.id(), Vec2::new(u, v));
        }
        commands.entity(e).remove::<MeshMaterial3d<StandardMaterial>>().insert(MeshMaterial3d(h));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_scale_and_mirror_into_wii_space() {
        // Red Gate Bridge: Wii (738.3, 450, 1629.4) is 3DS (73.83, 45, -162.94).
        assert!(world([73.83, 45.0, -162.94]).distance(Vec3::new(738.3, 450.0, 1629.4)) < 0.01);
    }

    #[test]
    fn mirrored_frames_stay_rotations() {
        let (s, c) = 0.6f32.sin_cos();
        let t = frame(&[c, 0.0, -s, 0.0, 1.0, 0.0, s, 0.0, c, 1.0, 2.0, 3.0]);
        assert!((t.rotation.length() - 1.0).abs() < 1e-4);
        assert!(t.translation.distance(Vec3::new(10.0, 20.0, -30.0)) < 1e-4);
    }
}
