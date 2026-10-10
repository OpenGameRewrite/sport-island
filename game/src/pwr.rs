//! Optional Pilotwings Resort add-on, active when `assets/pwr` was extracted
//! from the player's own cartridge dump. Without it the game is unchanged.
//!
//! Pilotwings Resort models Wuhu Island in metres, mirrored on Z against the
//! Wii game: a Wii position is `(x, y, -z) * 10` of the 3DS one.

use crate::collide::{Collision, Platform};
use crate::ipoint::{IPointDef, IPoints, Marker, MarkerFade};
use bevy::math::Affine3A;
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

pub(crate) fn data(rel: &str) -> Value {
    std::fs::read(crate::asset_dir().join("pwr/data").join(rel)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(Value::Null)
}

pub(crate) fn floats(v: &Value) -> Vec<f32> { v.as_array().into_iter().flatten().filter_map(|x| x.as_f64()).map(|x| x as f32).collect() }

/// `object` entries of a placement table such as `def_objects.val`.
pub(crate) fn objects(table: &Value) -> Vec<Value> { table["objects"][0]["object"].as_array().cloned().unwrap_or_default() }

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
        .add_systems(Update, (convert_materials, spin, scroll))
        // Platforms move before anyone stands on them this frame.
        .add_systems(PreUpdate, (follow_path, move_platforms).chain())
        .add_systems(FixedPostUpdate, boost_gates);
}

#[derive(Component)]
struct BoostGate { center: Vec3, normal: Vec3, radius: f32 }

/// Speed added by flying through the Boost Gate, metres per second.
const GATE_BOOST: f32 = 25.0;

/// Flying through a Boost Gate's opening throws the aircraft forward.
fn boost_gates(
    gates: Query<&BoostGate>,
    mut last: Local<Option<Vec3>>,
    mut flying: Query<(&Transform, Option<&mut crate::plane::Plane>, Option<&mut crate::vehicles::Vehicle>), With<crate::plane::Flying>>,
    mut sounds: MessageWriter<crate::sound::Sound>,
) {
    let Ok((t, plane, vehicle)) = flying.single_mut() else { *last = None; return };
    let now = t.translation;
    let Some(before) = last.replace(now) else { return };
    let through = gates.iter().any(|g| {
        let (a, b) = ((before - g.center).dot(g.normal), (now - g.center).dot(g.normal));
        if a.signum() == b.signum() { return false; }
        let cross = before.lerp(now, a / (a - b));
        cross.distance(g.center) < g.radius
    });
    if !through { return; }
    sounds.write(crate::sound::Sound::effect("pwr/race_turbo_deploy", None));
    if let Some(mut p) = plane {
        // Wii units per frame along the nose.
        let nose = t.rotation * Vec3::Z;
        p.vel += nose * GATE_BOOST * SCALE / 60.0;
    } else if let Some(mut v) = vehicle {
        let forward = v.body.rot * Vec3::Z;
        v.body.vel += forward * GATE_BOOST;
    }
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

/// A moving collision platform (index into `Collision::platforms`).
#[derive(Component)]
struct Carrier(usize);

fn stem(name: &str) -> String { name.trim_end_matches(".mod").to_ascii_lowercase() }

fn model(assets: &AssetServer, name: &str) -> WorldAssetRoot {
    WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(format!("pwr/models/race_{}.glb", stem(name)))))
}

/// A model's collision triangles (model space), when it is solid. The 3DS
/// strips wind both ways, so floors are turned to face up.
fn solid(name: &str) -> Vec<[Vec3; 3]> {
    let bytes = std::fs::read(crate::asset_dir().join(format!("pwr/models/race_{}.col", stem(name)))).unwrap_or_default();
    crate::collide::read_tris(&bytes).into_iter().map(|([a, b, c], _)| if (b - a).cross(c - a).y < 0.0 { [a, c, b] } else { [a, b, c] }).collect()
}

fn move_platforms(time: Res<Time>, mut col: ResMut<Collision>, q: Query<(&Carrier, &Transform)>) {
    for (c, t) in &q {
        let to_world = t.compute_affine() * Affine3A::from_scale(Vec3::splat(SCALE));
        if let Some(p) = col.platforms.get_mut(c.0) { p.set(to_world, time.delta_secs()); }
    }
}

fn spawn(mut commands: Commands, assets: Res<AssetServer>, sounds: Res<crate::sound::Sounds>, points: Res<IPoints>, mut col: ResMut<Collision>, mut solid_once: Local<bool>, existing: Query<(), With<PwrObject>>) {
    if !existing.is_empty() { return; }
    // Static pads join the island collision once; platforms are rebuilt.
    let add_static = !std::mem::replace(&mut *solid_once, true);
    col.platforms.clear();
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
            spawn_blimp(&mut commands, &assets, &sounds, &points, &mut col, t, name);
            continue;
        }
        if add_static {
            info!("pwr: {kind} is solid ({} triangles)", solid(name).len());
            for [a, b, c] in solid(name) { col.add(t.transform_point(a), t.transform_point(b), t.transform_point(c), 0); }
        }
        let mut e = commands.spawn((model(&assets, name), t, Visibility::default(), PwrObject, DespawnOnExit(AppState::Play)));
        // The ring spans the model's XY plane; its opening is about 6 m across the middle.
        if kind == "turbo_ring" { e.insert(BoostGate { center: t.translation, normal: t.rotation * Vec3::Z, radius: 6.0 * SCALE }); }
    }
}

/// The landing-pad blimp loops its route carrying i-point 100.
fn spawn_blimp(commands: &mut Commands, assets: &AssetServer, sounds: &crate::sound::Sounds, points: &IPoints, col: &mut Collision, start: Transform, model_name: &str) {
    let mut path: Vec<(i64, Vec3)> = objects(&data("wuhu_free_plane/paths.val.json")).iter()
        .filter(|o| o["name"].as_str() == Some("blimp_landing_pad"))
        .filter_map(|o| { let f = floats(&o["unfloat_frame64"]); (f.len() >= 12).then(|| (o["idx"].as_i64().unwrap_or(0), world([f[9], f[10], f[11]]))) })
        .collect();
    path.sort_by_key(|p| p.0);
    let speed = 7.5 * SCALE;
    let pose = Transform::from_translation(start.translation).with_rotation(start.rotation);
    col.platforms.push(Platform::new(solid(model_name), pose.compute_affine() * Affine3A::from_scale(Vec3::splat(SCALE))));
    let root = commands.spawn((pose, Visibility::default(), PwrObject, Carrier(col.platforms.len() - 1), DespawnOnExit(AppState::Play))).id();
    // Its engines hum within about 300 m.
    crate::sound::source(commands, assets, sounds, root, "pwr/race_blimp", 3000.0);
    if path.len() >= 2 {
        // Start the loop at the route point nearest the placed blimp.
        let points: Vec<Vec3> = path.into_iter().map(|p| p.1).collect();
        let nearest = (0..points.len()).min_by(|&a, &b| points[a].distance(start.translation).total_cmp(&points[b].distance(start.translation))).unwrap_or(0);
        let distance = (0..nearest).map(|i| points[i].distance(points[i + 1])).sum();
        commands.entity(root).insert(PathFollower { points, distance, speed });
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
