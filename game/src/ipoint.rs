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
}

#[derive(Component)]
struct MessageBox;
#[derive(Component)]
struct MessageTitle;
#[derive(Component)]
struct MessageBody;
#[derive(Component)]
struct Counter;

pub fn plugin(app: &mut App) {
    let defs: Vec<IPointDef> = crate::read_json("data/ipoints.json");
    app.insert_resource(IPoints(defs))
        .init_resource::<Shown>()
        .add_systems(OnEnter(AppState::Play), spawn)
        .add_systems(Update, (billboard, collect.run_if(crate::menu::playing), show_message, counter).run_if(in_state(AppState::Play)));
}

pub fn name_id(def: &IPointDef) -> u32 { def.msg << 8 }

fn spawn(mut commands: Commands, assets: Res<AssetServer>, points: Res<IPoints>, existing: Query<(), With<Marker>>) {
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
    commands.spawn((
        Node { position_type: PositionType::Absolute, right: px(24), top: px(20), padding: UiRect::axes(px(14), px(6)), border_radius: BorderRadius::all(px(12)), ..default() },
        BackgroundColor(Color::srgba(0.0, 0.1, 0.25, 0.55)),
        DespawnOnExit(AppState::Play),
        children![(Text::new(""), font.clone().with_font_size(FontSize::Px(26.0)), TextColor(Color::WHITE), Counter)],
    ));
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            left: percent(20),
            right: percent(20),
            bottom: px(40),
            padding: UiRect::all(px(18)),
            flex_direction: FlexDirection::Column,
            row_gap: px(8),
            border_radius: BorderRadius::all(px(16)),
            ..default()
        },
        BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.88)),
        Visibility::Hidden,
        MessageBox,
        DespawnOnExit(AppState::Play),
        children![
            (Text::new(""), font.clone().with_font_size(FontSize::Px(30.0)), TextColor(Color::srgb(0.05, 0.3, 0.75)), MessageTitle),
            (Text::new(""), font.with_font_size(FontSize::Px(24.0)), TextColor(Color::srgb(0.1, 0.1, 0.1)), MessageBody),
        ],
    ));
}

fn billboard(cam: Single<&GlobalTransform, With<MainCamera>>, time: Res<Time>, save: Res<Save>, tod: Res<TimeOfDay>, mut q: Query<(&Marker, &mut Transform)>) {
    let c = cam.translation();
    for (m, mut t) in &mut q {
        let d = c - t.translation;
        t.rotation = Quat::from_rotation_y(d.x.atan2(d.z));
        // Already-read points shrink, like the greyed check marks in the original.
        let read = save.collected.contains(&((m.0 * 3) as u16 + tod.0 as u16));
        let target = if read { 1.4 } else { 2.5 + 0.15 * (time.elapsed_secs() * 3.0 + m.0 as f32).sin() };
        t.scale = Vec3::splat(target);
    }
}

fn collect(
    points: Res<IPoints>,
    tod: Res<TimeOfDay>,
    time: Res<Time>,
    mut save: ResMut<Save>,
    mut shown: ResMut<Shown>,
    mut inside: Local<Option<usize>>,
    player: Query<&Transform, With<Player>>,
    plane: Query<&Transform, With<Plane>>,
) {
    let pos = plane.single().or(player.single()).map(|t| t.translation);
    let Ok(pos) = pos else { return };
    let hit = points.0.iter().enumerate().find(|(_, p)| p.kind == 0 && p.pos.distance(pos) < p.radius).map(|(i, _)| i);
    if hit.is_some() && hit != *inside {
        let i = hit.unwrap();
        save.collected.insert((i * 3) as u16 + tod.0 as u16);
        save.store();
        shown.index = Some(i);
        shown.until = time.elapsed_secs() + 7.0;
    }
    *inside = hit;
}

fn show_message(
    shown: Res<Shown>,
    time: Res<Time>,
    texts: Res<Texts>,
    c: Res<Controls>,
    tod: Res<TimeOfDay>,
    points: Res<IPoints>,
    mut boxv: Single<&mut Visibility, With<MessageBox>>,
    mut title: Single<&mut Text, (With<MessageTitle>, Without<MessageBody>)>,
    mut body: Single<&mut Text, (With<MessageBody>, Without<MessageTitle>)>,
) {
    let visible = shown.index.is_some() && time.elapsed_secs() < shown.until;
    **boxv = if visible { Visibility::Inherited } else { Visibility::Hidden };
    if let (true, Some(i)) = (visible, shown.index) {
        let def = &points.0[i];
        title.0 = texts.msg(name_id(def), c.pad);
        body.0 = texts.msg(name_id(def) + 1 + tod.0 as u32, c.pad);
    }
}

fn counter(save: Res<Save>, mut text: Single<&mut Text, With<Counter>>) {
    if save.is_changed() {
        text.0 = format!("ⓘ {} / {}", save.collected.len(), READINGS);
    }
}
