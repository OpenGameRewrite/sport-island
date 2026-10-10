//! Pilotwings Resort free-flight collectibles: gold rings for the gliders,
//! stunt rings for the plane and jet (flown through while doing their stunt),
//! Mii trophies for the rocket belts and 40 balloons for each. Each vehicle
//! class sees and collects only its own, as in the 3DS free-flight modes, and
//! found ones stay found.

use crate::plane::{Flying, Plane};
use crate::pwr::{self, SCALE};
use crate::save::Save;
use crate::sound::Sound;
use crate::vehicles::{self, Kind, Vehicle};
use crate::world::MainCamera;
use crate::AppState;
use bevy::prelude::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Class { Glider, Plane, Rocket }

fn class(kind: Kind) -> Class {
    match kind { Kind::Glider | Kind::PGlider => Class::Glider, Kind::Plane | Kind::Jet => Class::Plane, Kind::Rocket | Kind::SuperRocket => Class::Rocket }
}

/// The attitude a stunt ring asks for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Stunt { LeanLeft, LeanRight, Inverted, RollLeft, RollRight }

/// One collectible set: save key, placement table, class, model, pickup
/// radius (metres), whether it is a ring to fly through, sound and name.
struct Set { key: &'static str, table: &'static str, class: Class, model: &'static str, radius: f32, ring: bool, sound: &'static str, name: &'static str }

const SETS: &[Set] = &[
    Set { key: "gold_rings", table: "wuhu_free_glider/ffgoldring.val.json", class: Class::Glider, model: "gold_ring", radius: 8.0, ring: true, sound: "pwr/freeflight_goldringhit", name: "Freeflight:GOLD_RINGS_PLAIN" },
    Set { key: "stunt_rings", table: "wuhu_free_plane/ffstuntring.val.json", class: Class::Plane, model: "miiring", radius: 10.0, ring: true, sound: "pwr/freeflight_stuntringhit", name: "Freeflight:MII_STUNT_RINGS_PLAIN" },
    Set { key: "mii_trophies", table: "wuhu_free_rocket/miidoll.val.json", class: Class::Rocket, model: "miidoll", radius: 2.5, ring: false, sound: "pwr/race_miidollspin", name: "Freeflight:MII_DOLLS_PLAIN" },
    Set { key: "balloons_glider", table: "wuhu/ffballoon.val.json", class: Class::Glider, model: "balloonc", radius: 4.8, ring: false, sound: "pwr/race_balloonpop", name: "Freeflight:BALLOONS_PLAIN" },
    Set { key: "balloons_plane", table: "wuhu/ffballoon.val.json", class: Class::Plane, model: "balloonc", radius: 4.8, ring: false, sound: "pwr/race_balloonpop", name: "Freeflight:BALLOONS_PLAIN" },
    Set { key: "balloons_rocket", table: "wuhu/ffballoon.val.json", class: Class::Rocket, model: "balloonc", radius: 4.8, ring: false, sound: "pwr/race_balloonpop", name: "Freeflight:BALLOONS_PLAIN" },
];

/// A placed collectible: its set, index within the set and pose.
struct Item { set: usize, index: u16, pose: Transform, stunt: Option<Stunt> }

#[derive(Resource, Default)]
struct Items(Vec<Item>);

#[derive(Component)]
struct Pickup(usize);
#[derive(Component)]
struct Billboard;
#[derive(Component)]
struct Turn(Vec3);
#[derive(Component)]
struct Tally;

pub fn plugin(app: &mut App) {
    if !pwr::available() { return; }
    app.insert_resource(load())
        .add_systems(OnEnter(AppState::Play), spawn)
        .add_systems(Update, (show, face_camera, turn, tally).run_if(in_state(AppState::Play)))
        .add_systems(FixedPostUpdate, pick.run_if(crate::menu::playing));
}

fn load() -> Items {
    let mut items = vec![];
    for (s, set) in SETS.iter().enumerate() {
        let class = match set.class { Class::Glider => "glider", Class::Plane => "plane", Class::Rocket => "rocket" };
        let objects = pwr::objects(&pwr::data(set.table));
        // Balloon tables list every class; keep this set's.
        let mine = objects.iter().filter(|o| o["vehicle"].as_str().is_none_or(|v| v == class));
        for (i, o) in mine.enumerate() {
            let f = pwr::floats(&o["unfloat_frame64"]);
            if f.len() < 12 { continue; }
            let stunt = match o["stunt"].as_str() {
                Some("lean_left") => Some(Stunt::LeanLeft),
                Some("lean_right") => Some(Stunt::LeanRight),
                Some("inverted") => Some(Stunt::Inverted),
                Some("barrel_roll_left") => Some(Stunt::RollLeft),
                Some("barrel_roll_right") => Some(Stunt::RollRight),
                _ => None,
            };
            items.push(Item { set: s, index: i as u16, pose: pwr::frame(&f), stunt });
        }
    }
    Items(items)
}

fn spawn(mut commands: Commands, assets: Res<AssetServer>, items: Res<Items>, save: Res<Save>, existing: Query<(), With<Pickup>>) {
    if !existing.is_empty() { return; }
    for (i, item) in items.0.iter().enumerate() {
        let set = &SETS[item.set];
        if save.found.get(set.key).is_some_and(|f| f.contains(&item.index)) { continue; }
        let scene = assets.load(GltfAssetLabel::Scene(0).from_asset(format!("pwr/models/race_{}.glb", set.model)));
        let mut e = commands.spawn((WorldAssetRoot(scene), item.pose, Visibility::Hidden, Pickup(i), DespawnOnExit(AppState::Play)));
        match (set.model, item.stunt) {
            ("balloonc", _) => { e.insert(Billboard); }
            // Trophies turn on their stands; barrel-roll rings spin the way to roll.
            ("miidoll", _) => { e.insert(Turn(Vec3::Y * 1.5)); }
            (_, Some(Stunt::RollLeft)) => { e.insert(Turn(Vec3::Z * 2.0)); }
            (_, Some(Stunt::RollRight)) => { e.insert(Turn(Vec3::Z * -2.0)); }
            _ => {}
        }
    }
}

/// The class of the add-on vehicle being flown, if any.
fn flying_class(save: &Save, vehicles: &Query<&Vehicle>, planes: &Query<&Plane>) -> Option<Class> {
    if let Ok(v) = vehicles.single() { return Some(class(v.kind)); }
    planes.single().ok().and(vehicles::selected(save)).map(class)
}

fn show(save: Res<Save>, items: Res<Items>, vehicles: Query<&Vehicle>, planes: Query<&Plane>, mut q: Query<(&Pickup, &mut Visibility)>) {
    let now = flying_class(&save, &vehicles, &planes);
    for (p, mut vis) in &mut q {
        let on = now == Some(SETS[items.0[p.0].set].class);
        vis.set_if_neq(if on { Visibility::Inherited } else { Visibility::Hidden });
    }
}

fn face_camera(cam: Single<&GlobalTransform, With<MainCamera>>, mut q: Query<&mut Transform, With<Billboard>>) {
    let eye = cam.translation();
    for mut t in &mut q {
        let to = (eye - t.translation).with_y(0.0);
        if to.length_squared() > 1.0 { t.rotation = Quat::from_rotation_y(to.x.atan2(to.z)); }
    }
}

fn turn(time: Res<Time>, mut q: Query<(&Turn, &mut Transform)>) {
    for (r, mut t) in &mut q {
        let local = t.rotation * Quat::from_scaled_axis(r.0 * time.delta_secs());
        t.rotation = local.normalize();
    }
}

/// Whether the plane shows a stunt: `up` is its displayed up axis, `right`
/// its right wing, `roll` the barrel roll under way or just finished.
fn stunt_done(stunt: Stunt, up: Vec3, right: Vec3, roll: f32) -> bool {
    let lean = up.dot(right);
    match stunt {
        Stunt::LeanLeft => lean < -0.42,
        Stunt::LeanRight => lean > 0.42,
        Stunt::Inverted => up.y < -0.3,
        Stunt::RollLeft => roll < 0.0,
        Stunt::RollRight => roll > 0.0,
    }
}

/// Closest approach of the segment `a..b` to `c`.
fn segment_distance(a: Vec3, b: Vec3, c: Vec3) -> f32 {
    let d = b - a;
    let t = if d.length_squared() > 0.0 { ((c - a).dot(d) / d.length_squared()).clamp(0.0, 1.0) } else { 0.0 };
    (a + d * t).distance(c)
}

#[allow(clippy::too_many_arguments)]
fn pick(
    mut commands: Commands, items: Res<Items>, mut save: ResMut<Save>, mut last: Local<Option<Vec3>>,
    flying: Query<(&Transform, Option<&Plane>), With<Flying>>, vehicles: Query<&Vehicle>, planes: Query<&Plane>,
    pickups: Query<(Entity, &Pickup)>, mut sounds: MessageWriter<Sound>,
) {
    let (Ok((t, plane)), Some(now_class)) = (flying.single(), flying_class(&save, &vehicles, &planes)) else { *last = None; return };
    let now = t.translation;
    let Some(before) = last.replace(now) else { return };
    // The plane's shown attitude: its bank and any barrel roll on top of the flight model.
    let (up, right, roll) = plane.map_or((t.rotation * Vec3::Y, t.rotation * Vec3::X, 0.0), |p| {
        let shown = t.rotation * Quat::from_rotation_z(p.bank() + p.twist);
        let roll = if p.roll != 0.0 { p.roll } else if p.since_roll < 60 { p.last_roll } else { 0.0 };
        // The Wii plane frame's +X is its left wing.
        (shown * Vec3::Y, -(t.rotation * Vec3::X), roll)
    });
    for (e, p) in &pickups {
        let item = &items.0[p.0];
        let set = &SETS[item.set];
        if set.class != now_class { continue; }
        let (center, radius) = (item.pose.translation, set.radius * SCALE);
        let hit = if set.ring {
            let normal = item.pose.rotation * Vec3::Z;
            let (a, b) = ((before - center).dot(normal), (now - center).dot(normal));
            a.signum() != b.signum() && before.lerp(now, a / (a - b)).distance(center) < radius
                && item.stunt.is_none_or(|s| stunt_done(s, up, right, roll))
        } else {
            // The vehicle itself is a couple of metres across.
            segment_distance(before, now, center) < radius + 1.5 * SCALE
        };
        if !hit { continue; }
        save.found.entry(set.key.to_string()).or_default().insert(item.index);
        save.store();
        sounds.write(Sound::effect(set.sound, None));
        commands.entity(e).despawn();
    }
}

/// Found / total for the flown class's sets, under the i-point counter.
fn tally(
    mut commands: Commands, font: Res<crate::lang::UiFont>, texts: Res<crate::lang::Texts>, save: Res<Save>, items: Res<Items>,
    vehicles: Query<&Vehicle>, planes: Query<&Plane>, mut q: Query<(&mut Text, &mut Visibility), With<Tally>>,
) {
    let Ok((mut text, mut vis)) = q.single_mut() else {
        commands.spawn((
            Node { position_type: PositionType::Absolute, right: px(24), top: px(76), padding: UiRect::axes(px(16), px(5)), border: UiRect::all(px(3)), border_radius: BorderRadius::all(px(999)), ..default() },
            BackgroundColor(crate::menu::PANEL), BorderColor::all(crate::menu::OUTLINE),
            Text::new(""), TextFont::from(font.0.clone()).with_font_size(FontSize::Px(22.0)), TextColor(crate::menu::INK),
            Visibility::Hidden, Tally, DespawnOnExit(AppState::Play),
        ));
        return;
    };
    let Some(now) = flying_class(&save, &vehicles, &planes) else { vis.set_if_neq(Visibility::Hidden); return };
    let parts: Vec<String> = SETS.iter().enumerate().filter(|(_, s)| s.class == now).map(|(i, s)| {
        let total = items.0.iter().filter(|it| it.set == i).count();
        let found = save.found.get(s.key).map_or(0, |f| f.len());
        format!("{} {found}/{total}", texts.pwr(s.name).unwrap_or_else(|| s.key.replace('_', " ")))
    }).collect();
    text.set_if_neq(Text::new(parts.join("   ")));
    vis.set_if_neq(Visibility::Inherited);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stunts_need_the_right_attitude() {
        let (right, level) = (Vec3::X, Vec3::Y);
        assert!(!stunt_done(Stunt::LeanRight, level, right, 0.0));
        assert!(stunt_done(Stunt::LeanRight, Vec3::new(0.7, 0.7, 0.0), right, 0.0));
        assert!(stunt_done(Stunt::LeanLeft, Vec3::new(-0.7, 0.7, 0.0), right, 0.0));
        assert!(stunt_done(Stunt::Inverted, Vec3::NEG_Y, right, 0.0));
        assert!(stunt_done(Stunt::RollRight, level, right, 1.0) && !stunt_done(Stunt::RollRight, level, right, -1.0));
    }

    #[test]
    fn passing_close_counts_along_the_whole_step() {
        assert!(segment_distance(Vec3::new(-10.0, 0.0, 0.0), Vec3::new(10.0, 0.0, 0.0), Vec3::new(0.0, 1.0, 0.0)) < 1.01);
        assert!(segment_distance(Vec3::ZERO, Vec3::X, Vec3::new(5.0, 0.0, 0.0)) > 3.9);
    }
}
