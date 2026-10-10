//! i-points: the 80 sightseeing spots of Island Flyover, each with a message
//! for daytime, evening and night (240 readings in total), plus the spots
//! Pilotwings Resort added when that optional add-on is installed.

use crate::input::Controls;
use crate::lang::Texts;
use crate::plane::Flying as Plane;
use crate::player::Player;
use crate::save::Save;
use crate::world::{MainCamera, TimeOfDay};
use crate::AppState;
use bevy::prelude::*;
use bevy::world_serialization::WorldInstanceReady;
use serde::Deserialize;

/// Upper bound for saved readings (three per point); the add-on appends points.
pub const MAX_READINGS: usize = 3 * 128;

#[derive(Deserialize, Clone)]
pub struct IPointDef {
    pub radius: f32,
    /// 0 fixed, 3 carried by a moving vehicle, 6 unused.
    pub kind: u32,
    /// BMG message group: name is `msg << 8`, readings are `+1..+3`.
    pub msg: u32,
    pub pos: Vec3,
    /// Pilotwings Resort eyepoint id: its text for added points, and for shared
    /// points when built with the `pwr-text` feature.
    #[serde(default)]
    pub pwr: Option<u32>,
}

impl IPointDef {
    fn pwr_text(&self, texts: &Texts, part: &str) -> Option<String> {
        let id = self.pwr?;
        if self.msg != 0 && !cfg!(feature = "pwr-text") { return None; }
        texts.pwr(&format!("Eyepoint:{id}_{part}"))
    }
    pub fn name(&self, texts: &Texts, pad: bool) -> String {
        self.pwr_text(texts, "friendly_name").unwrap_or_else(|| texts.msg(name_id(self), pad))
    }
    pub fn reading(&self, texts: &Texts, pad: bool, time: u8) -> String {
        self.pwr_text(texts, ["day", "evening", "night"][time as usize % 3]).unwrap_or_else(|| texts.msg(name_id(self) + 1 + time as u32, pad))
    }
}

#[derive(Resource)]
pub struct IPoints(pub Vec<IPointDef>);

#[derive(Component)]
pub struct Marker(pub usize);

#[derive(Component, Default)]
pub struct MarkerFade(f32);
#[derive(Component)]
struct MarkerIcon(usize);
#[derive(Component)]
struct IconSource(Handle<crate::gx::GxMaterial>);
#[derive(Component)]
struct PointGlow(usize);
#[derive(Resource)]
struct PointEffect(Handle<AnimationGraph>, AnimationNodeIndex);

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
    let mut defs: Vec<IPointDef> = crate::read_json("data/ipoints.json");
    crate::pwr::add_ipoints(&mut defs);
    app.insert_resource(IPoints(defs))
        .init_resource::<Shown>()
        .add_systems(Startup, setup_effect)
        .add_systems(OnEnter(AppState::Play), spawn)
        .add_systems(Update, (billboard, paint_icons, collect.run_if(crate::menu::playing), show_message, spark, counter, hint).chain().run_if(in_state(AppState::Play)));
}

pub fn name_id(def: &IPointDef) -> u32 { def.msg << 8 }

fn setup_effect(mut commands: Commands, assets: Res<AssetServer>, mut graphs: ResMut<Assets<AnimationGraph>>) {
    let (graph, node) = AnimationGraph::from_clip(assets.load(GltfAssetLabel::Animation(0).from_asset("models/ipoint_fx.glb")));
    commands.insert_resource(PointEffect(graphs.add(graph), node));
}

/// The core and its glow are separate original models. Keep the effect attached
/// to the billboard so fixed points and moving carriers use the same visuals.
pub fn attach_visuals(ready: On<WorldInstanceReady>, markers: Query<&Marker>, children: Query<&Children>, names: Query<&bevy::gltf::GltfMaterialName>, assets: Res<AssetServer>, mut commands: Commands) {
    let Ok(marker) = markers.get(ready.entity) else { return };
    for child in children.iter_descendants(ready.entity) {
        if names.get(child).is_ok_and(|n| n.0 == "lambert2") {
            commands.entity(child).insert(MarkerIcon(marker.0));
        }
    }
    commands.entity(ready.entity).with_children(|c| {
        c.spawn((WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset("models/ipoint_fx.glb"))), Transform::default(), Visibility::default(), PointGlow(marker.0))).observe(play_effect).observe(crate::player::no_culling);
    });
}

fn play_effect(ready: On<WorldInstanceReady>, children: Query<&Children>, mut players: Query<&mut AnimationPlayer>, effect: Res<PointEffect>, mut commands: Commands) {
    for child in children.iter_descendants(ready.entity) {
        if let Ok(mut player) = players.get_mut(child) {
            player.play(effect.1).repeat();
            commands.entity(child).insert(AnimationGraphHandle(effect.0.clone()));
        }
    }
}

fn paint_icons(mut commands: Commands, save: Res<Save>, tod: Res<TimeOfDay>, assets: Res<AssetServer>, mut mats: ResMut<Assets<crate::gx::GxMaterial>>, mut icons: Query<(Entity, &MarkerIcon, Option<&IconSource>, &mut MeshMaterial3d<crate::gx::GxMaterial>)>, mut glow: Query<(&PointGlow, &mut Visibility)>, mut cache: Local<std::collections::HashMap<(AssetId<crate::gx::GxMaterial>, bool), Handle<crate::gx::GxMaterial>>>) {
    for (e, icon, original, mut handle) in &mut icons {
        let read = save.collected.contains(&((icon.0 * 3) as u16 + tod.0 as u16));
        let source = original.map_or_else(|| handle.0.clone(), |s| s.0.clone());
        let key = (source.id(), read);
        let replacement = if let Some(h) = cache.get(&key) { h.clone() } else {
            let Some(mut material) = mats.get(&source).cloned() else { continue };
            let texture = if read { "WS2_pln_info_core_check_grey" } else { "WS2_pln_info_core" };
            material.t0 = Some(assets.load_builder().with_settings(|s: &mut bevy::image::ImageLoaderSettings| s.is_srgb = false).load(format!("textures/{texture}.png")));
            let h = mats.add(material); cache.insert(key, h.clone()); h
        };
        if original.is_none() { commands.entity(e).insert(IconSource(source)); }
        handle.0 = replacement;
    }
    for (point, mut v) in &mut glow {
        *v = if save.collected.contains(&((point.0 * 3) as u16 + tod.0 as u16)) { Visibility::Hidden } else { Visibility::Inherited };
    }
}

fn spawn(mut commands: Commands, assets: Res<AssetServer>, points: Res<IPoints>, existing: Query<(), With<MessageBox>>, font: Res<crate::lang::UiFont>) {
    if !existing.is_empty() {
        return;
    }
    let scene = assets.load(GltfAssetLabel::Scene(0).from_asset("models/ipoint.glb"));
    for (i, p) in points.0.iter().enumerate() {
        // Points carried by vehicles appear once the vehicles do.
        if p.kind != 0 {
            continue;
        }
        commands.spawn((WorldAssetRoot(scene.clone()), Transform::from_translation(p.pos), Marker(i), MarkerFade::default())).observe(attach_visuals);
    }
    let font = TextFont::from(font.0.clone());
    commands.spawn((Node { position_type: PositionType::Absolute, left: percent(4), bottom: px(8), ..default() }, Text::new(""), font.clone().with_font_size(FontSize::Px(18.0)), TextColor(Color::WHITE), BoardHint, DespawnOnExit(AppState::Play)));
    commands.spawn((
        bevy::ui_widgets::Button, crate::menu::RecordButton,
        Node { position_type: PositionType::Absolute, right: px(24), top: px(20), padding: UiRect::axes(px(16), px(5)), border: UiRect::all(px(3)), border_radius: BorderRadius::all(px(999)), ..default() },
        BackgroundColor(crate::menu::PANEL), BorderColor::all(crate::menu::OUTLINE),
        DespawnOnExit(AppState::Play),
        children![(Text::new(""), font.clone().with_font_size(FontSize::Px(26.0)), TextColor(crate::menu::ACCENT), Counter)],
    )).observe(crate::menu::record_click);
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

fn billboard(cam: Single<&GlobalTransform, With<MainCamera>>, time: Res<Time>, points: Res<IPoints>, player: Query<&Transform, (With<Player>, Without<Marker>)>, plane: Query<&Transform, (With<Plane>, Without<Marker>)>, parents: Query<&GlobalTransform>, mut q: Query<(&Marker, &GlobalTransform, Option<&ChildOf>, &mut Transform, &mut Visibility, &mut MarkerFade)>) {
    let c = cam.translation();
    let viewer = plane.single().or(player.single()).map_or(c, |t| t.translation);
    for (m, global, parent, mut t, mut vis, mut fade) in &mut q {
        // Original range: per-landmark approach radius (minimum 200) and a
        // 600-unit outer fade band. Previously every point was visible globally.
        let range = points.0[m.0].radius.max(200.0);
        let distance = viewer.distance(global.translation());
        let toward = (global.translation() - c).normalize_or_zero();
        let facing = ((toward.dot(*cam.forward()) - 0.8) / 0.1).clamp(0.0, 1.0);
        let target = (1.0 - (distance - range).max(0.0) / 600.0).clamp(0.0, 1.0) * facing;
        // Original appearance is immediate; disappearance takes up to 20 ticks.
        let rate = if target < fade.0 { 0.05 } else { 1.0 } * time.delta_secs() * 60.0;
        fade.0 += (target - fade.0).clamp(-rate, rate);
        *vis = if fade.0 > 0.001 { Visibility::Inherited } else { Visibility::Hidden };
        let rotation = cam.rotation();
        t.rotation = parent.and_then(|p| parents.get(p.parent()).ok()).map_or(rotation, |p| p.rotation().inverse() * rotation);
        // The original bounds distance-dependent icon enlargement to 1..2.
        t.scale = Vec3::splat((c.distance(global.translation()) * 0.002).clamp(1.0, 2.0) * fade.0);
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
    // Collection is a physical 50-unit check in the original, independent of
    // the camera-facing fade. A nearby point behind the camera still counts.
    let hit = markers.iter().map(|(m, t, visible)| (m.0, t.translation(), t.translation().distance(pos), visible.get())).filter(|(i, _, d, visible)| *d <= points.0[*i].radius.max(200.0) && (*visible || *d <= 50.0)).map(|(i, source, distance, _)| (i, source, distance)).min_by(|a, b| a.2.total_cmp(&b.2));
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
        title.set_if_neq(Text::new(format!("ⓘ  {}", def.name(&texts, c.pad))));
        body.set_if_neq(Text::new(def.reading(&texts, c.pad, tod.0)));
        // Only the Wii points have photos.
        image.image = if def.msg != 0 { assets.load(format!("ui/WS2_pln_picture_{i:02}.png")) } else { Handle::default() };
    }
}

fn spark(shown: Res<Shown>, time: Res<Time>, camera: Single<(&Camera, &GlobalTransform), With<MainCamera>>, window: Single<&Window>, mut spark: Single<(&mut Node, &mut Visibility), (With<Spark>, Without<MessageBox>)>) {
    let elapsed = time.elapsed_secs() - shown.entered - 0.15;
    const DURATION: f32 = 29.0 / 60.0;
    let active = shown.index.is_some() && (0.0..DURATION).contains(&elapsed);
    *spark.1 = if active { Visibility::Inherited } else { Visibility::Hidden };
    if !active { return; }
    let end = Vec2::new(window.width() * 0.04 + 25.0, window.height() - 200.0);
    let start = camera.0.world_to_viewport(camera.1, shown.source).unwrap_or(end);
    let f = elapsed / DURATION;
    let pos = start.lerp(end, f) + Vec2::new(0.0, -80.0 * (f * std::f32::consts::PI).sin());
    spark.0.left = px(pos.x); spark.0.top = px(pos.y);
}

fn counter(save: Res<Save>, tod: Res<TimeOfDay>, points: Res<IPoints>, mut text: Single<&mut Text, With<Counter>>) {
    text.set_if_neq(Text::new(format!("ⓘ {} / {}", save.collected.iter().filter(|r| **r % 3 == tod.0 as u16).count(), points.0.len())));
}

fn hint(save: Res<Save>, texts: Res<Texts>, controls: Res<Controls>, mode: Res<State<crate::Mode>>, menu: Res<crate::menu::Menu>, mut hint: Single<&mut Text, With<BoardHint>>) {
    hint.set_if_neq(Text::new(if menu.open || *mode.get() == crate::Mode::Fly { String::new() } else {
        let board = crate::input::binding(&save, 9);
        format!("{}: {} · Esc / Start: {}", crate::input::label(&board[controls.pad as usize]), texts.ui(crate::lang::Ui::BoardHint), texts.ui(crate::lang::Ui::Controls))
    }));
}
