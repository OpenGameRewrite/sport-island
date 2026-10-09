//! Disc placements, original skeletal animations, air traffic and balloon targets.

use bevy::prelude::*;
use bevy::world_serialization::WorldInstanceReady;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use crate::{ipoint::{IPoints, Marker}, plane::{Plane, Shot}, sound::{Sound, Sounds}, world::TimeOfDay};

#[derive(Deserialize, Serialize, Clone, PartialEq)]
pub struct Placement {
    pub group: u16,
    pub kind: u16,
    pub pos: Vec3,
    pub scale: Vec3,
    pub rot: [f32; 9],
    pub params: [u32; 5],
}

impl Placement {
    pub fn transform(&self) -> Transform {
        // PMP stores forward/up/right, not an X/Y/Z matrix. The old ordering
        // produced a reflection, which cannot be represented by a quaternion.
        let forward = Vec3::from_slice(&self.rot[..3]);
        let up = Vec3::from_slice(&self.rot[3..6]);
        let right = Vec3::from_slice(&self.rot[6..]);
        Transform { translation: self.pos, rotation: Quat::from_mat3(&Mat3::from_cols(right, up, forward)).normalize(), scale: self.scale }
    }
}

#[derive(Deserialize)]
pub struct Route { pub points: Vec<Vec3>, pub flags: u16 }
#[derive(Deserialize)]
struct ModelRoutes { routes: Vec<Route> }
#[derive(Deserialize)]
struct Motion { clip: usize, points: Vec<Vec3> }
#[derive(Deserialize)]
struct CarMotion { motion: Vec<Motion> }
#[derive(Deserialize)]
pub struct Placements { pub objects: Vec<Placement>, pub routes: Vec<Route> }
#[derive(Resource)]
struct Island(Vec<(Placement, u8)>);
#[derive(Component)]
pub struct Times(pub u8);
#[derive(Component)]
struct Animated { graph: Handle<AnimationGraph>, node: AnimationNodeIndex, phase: f32, speed: f32 }
#[derive(Component)]
struct Carried { index: usize, bone: &'static str }
#[derive(Component)]
struct AttachedSound { name: &'static str, bone: &'static str }
#[derive(Component)]
struct Follower { points: Vec<Vec3>, segment: usize, distance: f32, speed: f32, looping: bool }
#[derive(Component)]
struct Guide { paths: [Vec<Vec3>; 3], time: u8 }
#[derive(Component)]
struct Target { radius: f32, respawn: f32 }
#[derive(Component)]
struct Ambient { kind: u16, range: f32, next: f32 }
#[derive(Component)]
struct Firework { next: f32, explode: f32, phase: f32 }
#[derive(Component)]
struct Flock { center: Vec3, radius: f32, phase: f32 }

pub fn plugin(app: &mut App) {
    let mut merged: HashMap<String, (Placement, u8)> = HashMap::new();
    for time in 0..3 {
        let data: Placements = crate::read_json(&format!("data/island_tag{time}.json"));
        for p in data.objects {
            let key = serde_json::to_string(&p).unwrap();
            merged.entry(key).and_modify(|(_, mask)| *mask |= 1 << time).or_insert((p, 1 << time));
        }
    }
    let mut objects: Vec<_> = merged.into_values().collect();
    // Stable ordering keeps animation offsets and carrier selection reproducible.
    objects.sort_by(|a, b| (a.0.group, a.0.kind).cmp(&(b.0.group, b.0.kind)).then_with(|| a.0.pos.x.total_cmp(&b.0.pos.x)).then_with(|| a.0.pos.z.total_cmp(&b.0.pos.z)));
    app.insert_resource(Island(objects))
        .add_systems(Startup, setup)
        .add_systems(Update, visibility)
        .add_systems(Update, (guide_route, follow, birds, targets, ambient, fireworks).chain().run_if(crate::menu::playing));
}

#[allow(clippy::too_many_arguments)]
fn setup(mut commands: Commands, assets: Res<AssetServer>, mut graphs: ResMut<Assets<AnimationGraph>>, island: Res<Island>, points: Res<IPoints>, sounds: Res<Sounds>) {
    let mut clips = HashMap::new();
    let car_motion: CarMotion = crate::read_json("models/car.json");
    for (i, (p, mask)) in island.0.iter().enumerate() {
        if p.group == 5 {
            commands.spawn((p.transform(), Visibility::default(), Times(*mask), Ambient { kind: p.kind, range: p.scale.x.abs().max(1.0), next: 0.0 }));
            continue;
        }
        if p.group != 1 { continue; }
        if p.kind == 7 || p.kind == 32 {
            let model = if p.kind == 7 { "bird" } else { "hawk" };
            let path = format!("models/{model}.glb");
            let (graph, node) = clips.entry((model, 0)).or_insert_with(|| {
                let (g, n) = AnimationGraph::from_clip(assets.load(GltfAssetLabel::Animation(0).from_asset(path.clone())));
                (graphs.add(g), n)
            }).clone();
            for bird in 0..3 {
                commands.spawn((WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(path.clone()))), p.transform().with_scale(Vec3::ONE), Visibility::default(), Times(*mask), Animated { graph: graph.clone(), node, phase: bird as f32 * 0.3, speed: 1.0 }, Flock { center: p.pos, radius: p.scale.x.abs().max(100.0), phase: i as f32 + bird as f32 * 2.1 })).observe(start_animation).observe(crate::player::no_culling);
            }
            continue;
        }
        if p.kind == 36 {
            let root = commands.spawn((p.transform(), Visibility::default(), Times(4))).id();
            let (graph, node) = AnimationGraph::from_clip(assets.load(GltfAssetLabel::Animation(0).from_asset("models/fireworks.glb")));
            commands.entity(root).with_children(|c| {
                c.spawn((WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset("models/fireworks.glb"))), Visibility::Hidden, Animated { graph: graphs.add(graph), node, phase: 0.0, speed: 1.0 }, Firework { next: i as f32 % 10.0, explode: 0.0, phase: 0.0 })).observe(start_animation);
            });
            continue;
        }
        let (model, mut clip, speed, sound) = match p.kind {
            0 => ("tree", None, 1.0, None),
            1 => ("tree2", None, 1.0, None),
            2 => ("tree3", None, 1.0, None),
            3 => ("airship", Some(3), 1.0, Some("SE_CMN_AIR_SHIP")),
            4 | 9 | 27 | 28 | 46 => ("cruiser", None, 1.0, Some("SE_CMN_MOVE_CRUISER")),
            10 => ("pole", None, 1.0, None),
            11 | 12 | 18 | 19 => ("flag_a", None, 1.0, None),
            13 | 15 | 24 | 25 => ("flag_b", None, 1.0, None),
            33 | 35 => ("flag_c", None, 1.0, None),
            14 => ("tee", None, 1.0, None),
            16 => ("car", None, 1.0, None),
            20 | 22 | 23 => ("balloon", Some(0), 1.0, None),
            26 => ("windmill", Some(0), 1.0, None),
            30 => ("fountain", None, 1.0, Some("SE_CMN_FOUNTAIN")),
            31 => ("campfire", None, 1.0, Some("SE_CMN_CAMP_FIRE")),
            34 => ("tee", None, 1.0, None),
            37 => ("car", Some(3), 1.0, Some("SE_CMN_ELEC_CAR_RUNNING_1")),
            43 => ("seaguard", None, 1.0, None),
            45 => ("whale", Some(3), 1.0, None),
            // ponytail: the liner clip encodes a route in 30 frames; exact original traversal timing is not recovered.
            54 => ("liner", Some(0), 0.001, Some("SE_CMN_MOVE_LINER")),
            _ => continue,
        };
        let path = format!("models/{model}.glb");
        if !crate::asset_dir().join(&path).exists() { continue; }
        // Cars with baked world paths and the airship are already in world coordinates.
        let transform = if p.kind == 37 || p.kind == 3 || p.kind == 54 || p.kind == 46 || p.kind == 45 { Transform::default() } else { p.transform() };
        let entity = commands.spawn((WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(path.clone()))), transform, Visibility::default(), Times(*mask))).id();
        let mut phase = i as f32 * 1.7;
        if p.kind == 37 {
            if let Some((_, chosen, frame)) = car_motion.motion.iter().flat_map(|m| m.points.iter().enumerate().map(move |(frame, pos)| (p.pos.distance_squared(*pos), m.clip, frame))).min_by(|a, b| a.0.total_cmp(&b.0)) {
                clip = Some(chosen);
                phase = frame as f32 / 60.0;
            }
        }
        if let Some(clip) = clip {
            let (graph, node) = clips.entry((model, clip)).or_insert_with(|| {
                let (graph, node) = AnimationGraph::from_clip(assets.load(GltfAssetLabel::Animation(clip).from_asset(path.clone())));
                (graphs.add(graph), node)
            }).clone();
            commands.entity(entity).insert(Animated { graph, node, phase, speed }).observe(start_animation).observe(crate::player::no_culling);
        }
        if p.kind == 54 {
            commands.entity(entity).insert(Carried { index: 79, bone: "liner" }).observe(attach_point);
        }
        if p.kind == 3 {
            commands.entity(entity).insert(Carried { index: 77, bone: "WS2_common_airship" }).observe(attach_point);
        }
        if p.kind == 46 || p.kind == 45 {
            let mut routes: ModelRoutes = crate::read_json(&format!("models/{model}.json"));
            if !routes.routes.is_empty() {
                let route = routes.routes.swap_remove(i % routes.routes.len());
                if route.points.len() > 1 {
                    // ponytail: path geometry is original; exact per-instance path/speed selection still needs an oracle.
                    commands.entity(entity).insert(Follower { points: route.points, segment: 0, distance: 0.0, speed: if p.kind == 46 { 65.0 } else { 90.0 }, looping: route.flags & 1 != 0 });
                }
            }
        }
        if let Some(name) = sound {
            let bone = match model { "car" => "WS2_common_car", "airship" => "WS2_common_airship", "liner" => "liner", _ => "" };
            commands.entity(entity).insert(AttachedSound { name, bone }).observe(attach_sound);
        }
        if matches!(p.kind, 4 | 9 | 27 | 28 | 46) && *mask & 4 != 0 {
            commands.entity(entity).with_child((WorldAssetRoot(assets.load(GltfAssetLabel::Scene(1).from_asset("models/cruiser.glb"))), Times(4)));
        }
    }
    // Flight traffic follows the extracted point paths.
    let routes: Placements = crate::read_json("data/seaplane_path.json");
    for (i, route) in routes.routes.into_iter().enumerate() {
        if route.points.len() < 2 { continue; }
        let pos = route.points[0];
        let entity = commands.spawn((WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset("models/seaplane.glb"))), Transform::from_translation(pos), Visibility::default(), Follower { points: route.points, segment: 0, distance: 0.0, speed: 280.0, looping: route.flags & 1 != 0 })).observe(crate::player::no_culling).id();
        crate::sound::source(&mut commands, &assets, &sounds, entity, "SE_CMN_SEA_PLANE", 1200.0);
        if i == 0 { commands.entity(entity).insert(Carried { index: 78, bone: "WS2_common_seaplane" }).observe(attach_point); }
    }
    let paths = std::array::from_fn(|time| {
        let guide: Placements = crate::read_json(&format!("data/guide_path{time}.json"));
        guide.routes.into_iter().find(|r| r.points.len() > 1).expect("original guide route missing").points
    });
    commands.spawn((WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset("models/guideplane.glb"))), Transform::from_translation(paths[0][0]), Visibility::default(), Carried { index: 44, bone: "plane_body" }, Follower { points: paths[0].clone(), segment: 0, distance: 0.0, speed: 420.0, looping: true }, Guide { paths, time: 0 })).observe(attach_point).observe(crate::player::no_culling);
    let balloons: Placements = crate::read_json("data/island_f0.json");
    for p in balloons.objects.into_iter().filter(|p| p.group == 3 && (p.kind == 3 || p.kind == 4)) {
        let giant = p.kind == 4;
        commands.spawn((WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(if giant { "models/giant_balloon.glb" } else { "models/small_balloon.glb" }))), p.transform(), Target { radius: if giant { 100.0 } else { 30.0 }, respawn: 0.0 }));
    }
    let _ = points;
}

fn birds(time: Res<Time>, mut q: Query<(&Flock, &mut Transform)>) {
    for (flock, mut t) in &mut q {
        // ponytail: original flock regions/wing clips; orbit replaces unrecovered flock steering.
        let a = time.elapsed_secs() * 100.0 / flock.radius + flock.phase;
        t.translation = flock.center + Vec3::new(a.cos() * flock.radius, a.sin() * 30.0, a.sin() * flock.radius);
        t.rotation = Quat::from_rotation_y(-a);
    }
}

fn start_animation(ready: On<WorldInstanceReady>, children: Query<&Children>, animated: Query<&Animated>, mut players: Query<&mut AnimationPlayer>, mut commands: Commands) {
    let Ok(a) = animated.get(ready.entity) else { return };
    for child in children.iter_descendants(ready.entity) {
        if let Ok(mut player) = players.get_mut(child) {
            player.play(a.node).repeat().set_speed(a.speed).set_seek_time(a.phase);
            commands.entity(child).insert(AnimationGraphHandle(a.graph.clone()));
        }
    }
}

fn attach_point(ready: On<WorldInstanceReady>, children: Query<&Children>, names: Query<&Name>, carried: Query<&Carried>, points: Res<IPoints>, assets: Res<AssetServer>, mut commands: Commands) {
    let Ok(carried) = carried.get(ready.entity) else { return };
    let bone = children.iter_descendants(ready.entity).find(|&e| names.get(e).is_ok_and(|n| n.as_str() == carried.bone)).unwrap_or(ready.entity);
    commands.entity(bone).with_children(|c| {
        c.spawn((WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset("models/ipoint.glb"))), Transform::from_translation(points.0[carried.index].pos), Marker(carried.index), crate::ipoint::MarkerFade::default())).observe(crate::ipoint::attach_visuals);
    });
}

fn visibility(tod: Res<TimeOfDay>, mut q: Query<(&Times, &mut Visibility)>) {
    for (times, mut v) in &mut q { *v = if times.0 & (1 << tod.0) != 0 { Visibility::Inherited } else { Visibility::Hidden }; }
}

fn guide_route(tod: Res<TimeOfDay>, mut q: Query<(&mut Guide, &mut Follower, &mut Transform)>) {
    for (mut guide, mut path, mut transform) in &mut q {
        if guide.time == tod.0 { continue; }
        guide.time = tod.0;
        path.points.clone_from(&guide.paths[tod.0 as usize]);
        path.segment = 0;
        path.distance = 0.0;
        transform.translation = path.points[0];
    }
}

fn follow(time: Res<Time>, mut q: Query<(&mut Follower, &mut Transform)>) {
    for (mut path, mut transform) in &mut q {
        path.distance += path.speed * time.delta_secs();
        // Consume all segments, including repeated endpoints, without losing frame time.
        for _ in 0..path.points.len() * 2 {
            if path.segment + 1 == path.points.len() && !path.looping {
                path.points.reverse();
                path.segment = 0;
            }
            let next = (path.segment + 1) % path.points.len();
            let from = path.points[path.segment];
            let to = path.points[next];
            let length = from.distance(to);
            if length > 0.01 && path.distance < length {
                transform.translation = from.lerp(to, path.distance / length);
                transform.rotation = Transform::IDENTITY.looking_to(-(to - from), Vec3::Y).rotation;
                break;
            }
            path.distance = (path.distance - length).max(0.0);
            path.segment = next;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;
    #[test]
    fn placement_keeps_original_forward_and_up_without_reflecting() {
        let p = Placement { group: 0, kind: 0, pos: Vec3::ONE, scale: Vec3::splat(2.0), rot: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, -1.0], params: [0; 5] };
        let t = p.transform();
        assert!((t.rotation * Vec3::Z).abs_diff_eq(Vec3::X, 0.00001));
        assert!((t.rotation * Vec3::Y).abs_diff_eq(Vec3::Y, 0.00001));
        assert!((t.rotation * Vec3::X).abs_diff_eq(Vec3::NEG_Z, 0.00001));
        assert_eq!(t.translation, p.pos);
        assert_eq!(t.scale, p.scale);
    }
    #[test]
    fn open_path_bounces_without_taking_a_closing_shortcut() {
        let mut world = World::new();
        let mut time = Time::<()>::default();
        time.advance_by(std::time::Duration::from_secs_f32(3.25));
        world.insert_resource(time);
        let e = world.spawn((Follower { points: vec![Vec3::ZERO, Vec3::X * 10.0, Vec3::X * 10.0, Vec3::X * 20.0], segment: 0, distance: 0.0, speed: 10.0, looping: false }, Transform::default())).id();
        world.run_system_once(follow).unwrap();
        assert!((world.get::<Transform>(e).unwrap().translation.x - 7.5).abs() < 0.001);
    }
}

fn attach_sound(ready: On<WorldInstanceReady>, children: Query<&Children>, names: Query<&Name>, attached: Query<&AttachedSound>, assets: Res<AssetServer>, sounds: Res<Sounds>, mut commands: Commands) {
    let Ok(sound) = attached.get(ready.entity) else { return };
    let bone = children.iter_descendants(ready.entity).find(|&e| names.get(e).is_ok_and(|n| n.as_str() == sound.bone)).unwrap_or(ready.entity);
    crate::sound::source(&mut commands, &assets, &sounds, bone, sound.name, 1200.0);
}

fn targets(time: Res<Time>, mut shots: MessageReader<Shot>, planes: Query<&Transform, With<Plane>>, mut q: Query<(&mut Target, &Transform, &mut Visibility)>, mut sounds: MessageWriter<Sound>) {
    let shots: Vec<_> = shots.read().map(|s| (s.pos, s.radius)).collect();
    for (mut target, t, mut visible) in &mut q {
        if target.respawn > 0.0 {
            target.respawn -= time.delta_secs();
            if target.respawn <= 0.0 { *visible = Visibility::Inherited; }
            continue;
        }
        if shots.iter().any(|(pos, radius)| t.translation.distance(*pos) < target.radius + radius) || planes.iter().any(|p| t.translation.distance(p.translation) < target.radius + 25.0) {
            *visible = Visibility::Hidden;
            target.respawn = 30.0;
            sounds.write(Sound::effect("SE_PLN_BALLOON_BREAK_S", Some(t.translation)));
        }
    }
}

fn ambient(mut commands: Commands, time: Res<Time>, tod: Res<TimeOfDay>, assets: Res<AssetServer>, sounds: Res<Sounds>, mut q: Query<(Entity, &mut Ambient, &Times, Option<&Children>)>) {
    for (entity, mut ambient, times, children) in &mut q {
        if times.0 & (1 << tod.0) == 0 { continue; }
        if tod.is_changed() {
            if let Some(children) = children { for child in children { commands.entity(*child).despawn(); } }
            ambient.next = 0.0;
        }
        if time.elapsed_secs() >= ambient.next && (tod.is_changed() || children.is_none_or(|c| c.is_empty())) {
            if let Some(name) = crate::sound::ambience(ambient.kind, tod.0) { crate::sound::source(&mut commands, &assets, &sounds, entity, name, ambient.range); }
            ambient.next = time.elapsed_secs() + 8.0 + entity.index_u32() as f32 % 5.0;
        }
    }
}

fn fireworks(time: Res<Time>, tod: Res<TimeOfDay>, children: Query<&Children>, mut players: Query<&mut AnimationPlayer>, mut q: Query<(Entity, &mut Firework, &Animated, &GlobalTransform, &mut Visibility)>, mut sounds: MessageWriter<Sound>) {
    for (entity, mut firework, animated, transform, mut visibility) in &mut q {
        if tod.0 != 2 { *visibility = Visibility::Hidden; continue; }
        let now = time.elapsed_secs();
        if now >= firework.next {
            firework.next = now + 8.0 + entity.index_u32() as f32 % 7.0;
            firework.explode = now + 0.6;
            firework.phase = now;
            *visibility = Visibility::Inherited;
            for e in children.iter_descendants(entity) {
                if let Ok(mut player) = players.get_mut(e) { player.start(animated.node); }
            }
            sounds.write(Sound::effect("SE_CMN_FIREWORKS_LAUNCH", Some(transform.translation())));
        }
        if firework.explode > 0.0 && now >= firework.explode {
            sounds.write(Sound::effect("SE_CMN_FIREWORKS_EXPLODE", Some(transform.translation())));
            firework.explode = 0.0;
        }
        if now - firework.phase > 1.4 { *visibility = Visibility::Hidden; }
    }
}
