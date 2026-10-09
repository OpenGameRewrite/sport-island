//! i-points: the 80 sightseeing spots of Island Flyover, each with a message
//! for daytime, evening and night (240 readings in total).

use crate::input::Controls;
use crate::lang::Texts;
use crate::plane::Plane;
use crate::player::Player;
use crate::save::Save;
use crate::world::{MainCamera, TimeOfDay};
use crate::AppState;
use bevy::prelude::*;
use serde::Deserialize;

pub const READINGS: usize = 240;

#[derive(Deserialize, Clone)]
pub struct IPointDef {
    pub radius: f32,
    /// 0 fixed, 3 carried by a moving vehicle, 6 unused.
    pub kind: u32,
    /// BMG message group: name is `msg << 8`, readings are `+1..+3`.
    pub msg: u32,
    pub pos: Vec3,
}

#[derive(Resource)]
pub struct IPoints(pub Vec<IPointDef>);

#[derive(Component)]
pub struct Marker(pub usize);

/// Currently shown message and when it expires.
#[derive(Resource, Default)]
struct Shown {
    index: Option<usize>,
    until: f32,
    entered: f32,
    source: Vec3,
    radio: bool,
}

#[derive(Component)]
struct MessageBox;
#[derive(Component)]
struct MessageTitle;
#[derive(Component)]
struct MessageBody;
#[derive(Component)]
struct Counter;
#[derive(Component)]
struct BoardHint;
#[derive(Component)]
struct MessageImage;
#[derive(Component)]
struct Spark;

pub fn plugin(app: &mut App) {
    let defs: Vec<IPointDef> = crate::read_json("data/ipoints.json");
    app.insert_resource(IPoints(defs))
        .init_resource::<Shown>()
        .add_systems(OnEnter(AppState::Play), spawn)
        .add_systems(Update, (billboard, collect.run_if(crate::menu::playing), show_message, spark, counter, hint).chain().run_if(in_state(AppState::Play)));
}

pub fn name_id(def: &IPointDef) -> u32 { def.msg << 8 }

fn spawn(mut commands: Commands, assets: Res<AssetServer>, points: Res<IPoints>, existing: Query<(), With<MessageBox>>) {
    if !existing.is_empty() {
        return;
    }
    let scene = assets.load(GltfAssetLabel::Scene(0).from_asset("models/ipoint.glb"));
    for (i, p) in points.0.iter().enumerate() {
        // Points carried by vehicles appear once the vehicles do.
        if p.kind != 0 {
            continue;
        }
        commands.spawn((WorldAssetRoot(scene.clone()), Transform::from_translation(p.pos).with_scale(Vec3::splat(2.5)), Marker(i)));
    }
    let font = TextFont::from(FontSource::sans_serif());
    commands.spawn((Node { position_type: PositionType::Absolute, left: percent(4), bottom: px(8), ..default() }, Text::new(""), font.clone().with_font_size(FontSize::Px(18.0)), TextColor(Color::WHITE), BoardHint, DespawnOnExit(AppState::Play)));
    commands.spawn((
        Node { position_type: PositionType::Absolute, right: px(24), top: px(20), padding: UiRect::axes(px(14), px(6)), border_radius: BorderRadius::all(px(12)), ..default() },
        BackgroundColor(Color::srgba(0.0, 0.1, 0.25, 0.55)),
        DespawnOnExit(AppState::Play),
        children![(Text::new(""), font.clone().with_font_size(FontSize::Px(26.0)), TextColor(Color::WHITE), Counter)],
    ));
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            left: percent(4),
            right: percent(26),
            bottom: px(40),
            flex_direction: FlexDirection::Column,
            row_gap: px(4),
            border_radius: BorderRadius::all(px(16)),
            ..default()
        },
        Visibility::Hidden,
        MessageBox,
        DespawnOnExit(AppState::Play),
        children![
            (Node { padding: UiRect::axes(px(14), px(5)), ..default() }, BackgroundColor(Color::srgba(0.05, 0.5, 0.22, 0.85)), children![(Text::new(""), font.clone().with_font_size(FontSize::Px(30.0)), TextColor(Color::WHITE), MessageTitle)]),
            (Node { padding: UiRect::all(px(12)), column_gap: px(16), align_items: AlignItems::Center, border: UiRect::all(px(2)), border_radius: BorderRadius::all(px(10)), ..default() }, BorderColor::all(Color::WHITE), BackgroundColor(Color::srgba(0.92, 0.97, 1.0, 0.86)), children![
                (Node { width: px(180), height: px(90), flex_shrink: 0.0, ..default() }, ImageNode::default(), MessageImage),
                (Node { flex_grow: 1.0, min_width: px(0), ..default() }, Text::new(""), font.with_font_size(FontSize::Px(26.0)), TextColor(Color::srgb(0.1, 0.1, 0.1)), MessageBody),
            ]),
        ],
    ));
    commands.spawn((Node { position_type: PositionType::Absolute, width: px(24), height: px(24), border_radius: BorderRadius::all(px(12)), ..default() }, BackgroundColor(Color::srgb(0.6, 0.95, 1.0)), GlobalZIndex(5), Visibility::Hidden, Spark, DespawnOnExit(AppState::Play)));
}

fn billboard(cam: Single<&GlobalTransform, With<MainCamera>>, time: Res<Time>, save: Res<Save>, tod: Res<TimeOfDay>, points: Res<IPoints>, player: Query<&Transform, (With<Player>, Without<Marker>)>, plane: Query<&Transform, (With<Plane>, Without<Marker>)>, parents: Query<&GlobalTransform>, mut q: Query<(&Marker, &GlobalTransform, Option<&ChildOf>, &mut Transform, &mut Visibility)>) {
    let c = cam.translation();
    let viewer = plane.single().or(player.single()).map_or(c, |t| t.translation);
    for (m, global, parent, mut t, mut vis) in &mut q {
        // Original range: per-landmark approach radius (minimum 200) and a
        // 600-unit outer fade band. Previously every point was visible globally.
        let range = points.0[m.0].radius.max(200.0);
        let distance = viewer.distance(global.translation());
        *vis = if distance <= range + 600.0 { Visibility::Inherited } else { Visibility::Hidden };
        let d = c - global.translation();
        let rotation = Quat::from_rotation_y(d.x.atan2(d.z));
        t.rotation = parent.and_then(|p| parents.get(p.parent()).ok()).map_or(rotation, |p| p.rotation().inverse() * rotation);
        // Already-read points shrink, like the greyed check marks in the original.
        let read = save.collected.contains(&((m.0 * 3) as u16 + tod.0 as u16));
        let target = if read { 1.4 } else { 2.5 + 0.15 * (time.elapsed_secs() * 3.0 + m.0 as f32).sin() };
        t.scale = Vec3::splat(target * (1.0 - (distance - range).max(0.0) / 600.0).clamp(0.0, 1.0));
    }
}

fn collect(
    points: Res<IPoints>,
    tod: Res<TimeOfDay>,
    time: Res<Time>,
    mut save: ResMut<Save>,
    mut shown: ResMut<Shown>,
    mut inside: Local<Option<(usize, u8)>>,
    player: Query<&Transform, With<Player>>,
    plane: Query<&Transform, With<Plane>>,
    markers: Query<(&Marker, &GlobalTransform, &InheritedVisibility)>,
    mut sounds: MessageWriter<crate::sound::Sound>,
) {
    let pos = plane.single().or(player.single()).map(|t| t.translation);
    let Ok(pos) = pos else { return };
    let hit = markers.iter().filter(|(_, _, visible)| visible.get()).map(|(m, t, _)| (m.0, t.translation(), t.translation().distance(pos))).filter(|(i, _, d)| *d <= points.0[*i].radius.max(200.0)).min_by(|a, b| a.2.total_cmp(&b.2));
    let key = hit.map(|(i, _, _)| (i, tod.0));
    if key.is_some() && key != *inside {
        let (i, source, _) = hit.unwrap();
        shown.index = Some(i);
        shown.until = time.elapsed_secs() + 10.0;
        shown.entered = time.elapsed_secs();
        shown.source = source;
        shown.radio = false;
        sounds.write(crate::sound::Sound::effect("SE_PLN_LANDMARK_GUIDE_DISP", None));
    }
    if let Some((i, source, distance)) = hit {
        // Keep the reading available while exploring the landmark itself.
        shown.until = time.elapsed_secs() + 7.0;
        if shown.index == Some(i) && !shown.radio && time.elapsed_secs() > shown.entered + 0.15 {
            shown.radio = true;
            shown.source = source;
            sounds.write(crate::sound::Sound::effect("SE_PLN_LANDMARK_RADIO", None));
        }
        if distance <= 50.0 && save.collected.insert((i * 3) as u16 + tod.0 as u16) {
            save.store();
            sounds.write(crate::sound::Sound::effect("SE_PLN_LANDMARK_RING_GET", None));
            sounds.write(crate::sound::Sound::effect("SE_PLN_LANDMARK_INFO_GET", None));
            shown.until = time.elapsed_secs() + 7.0;
        }
    }
    *inside = key;
}

fn show_message(
    shown: Res<Shown>,
    time: Res<Time>,
    texts: Res<Texts>,
    c: Res<Controls>,
    tod: Res<TimeOfDay>,
    points: Res<IPoints>,
    assets: Res<AssetServer>,
    mut image: Single<&mut ImageNode, With<MessageImage>>,
    mut boxv: Single<&mut Visibility, (With<MessageBox>, Without<Spark>)>,
    mut title: Single<&mut Text, (With<MessageTitle>, Without<MessageBody>)>,
    mut body: Single<&mut Text, (With<MessageBody>, Without<MessageTitle>)>,
) {
    let visible = shown.index.is_some() && time.elapsed_secs() < shown.until;
    **boxv = if visible { Visibility::Inherited } else { Visibility::Hidden };
    if let (true, Some(i)) = (visible, shown.index) {
        let def = &points.0[i];
        title.0 = format!("ⓘ  {}", texts.msg(name_id(def), c.pad));
        body.0 = texts.msg(name_id(def) + 1 + tod.0 as u32, c.pad);
        image.image = assets.load(format!("ui/WS2_pln_picture_{i:02}.png"));
    }
}

fn spark(shown: Res<Shown>, time: Res<Time>, camera: Single<(&Camera, &GlobalTransform), With<MainCamera>>, window: Single<&Window>, mut spark: Single<(&mut Node, &mut Visibility), (With<Spark>, Without<MessageBox>)>, mut gizmos: Gizmos) {
    let elapsed = time.elapsed_secs() - shown.entered - 0.15;
    let active = shown.index.is_some() && (0.0..0.5).contains(&elapsed);
    *spark.1 = if active { Visibility::Inherited } else { Visibility::Hidden };
    if !active { return; }
    let end = Vec2::new(window.width() * 0.04 + 25.0, window.height() - 200.0);
    let start = camera.0.world_to_viewport(camera.1, shown.source).unwrap_or(end);
    let f = elapsed / 0.5;
    let pos = start.lerp(end, f) + Vec2::new(0.0, -80.0 * (f * std::f32::consts::PI).sin());
    spark.0.left = px(pos.x); spark.0.top = px(pos.y);
    // The original transmission advances from world point to card in 29 frames.
    for i in 0..3 {
        let radius = 50.0 + 180.0 * f + i as f32 * 20.0;
        gizmos.circle(Isometry3d::new(shown.source, camera.1.rotation()), radius, Color::srgba(0.2, 0.7, 1.0, 0.6 * (1.0 - f)));
    }
}

fn counter(save: Res<Save>, tod: Res<TimeOfDay>, mut text: Single<&mut Text, With<Counter>>) {
    text.0 = format!("ⓘ {} / 80", save.collected.iter().filter(|r| **r % 3 == tod.0 as u16).count());
}

fn hint(save: Res<Save>, texts: Res<Texts>, controls: Res<Controls>, mode: Res<State<crate::Mode>>, menu: Res<crate::menu::Menu>, mut hint: Single<&mut Text, With<BoardHint>>) {
    hint.0 = if menu.open || *mode.get() == crate::Mode::Fly { String::new() } else {
        let board = crate::input::binding(&save, 9);
        format!("{}: {} · Esc / Start: {}", crate::input::label(&board[controls.pad as usize]), texts.ui(crate::lang::Ui::BoardHint), texts.ui(crate::lang::Ui::Controls))
    };
}
