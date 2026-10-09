//! Original sounds, with distance-limited emitters and time-of-day ambience.

use bevy::audio::{AudioSinkPlayback, SpatialScale, Volume};
use bevy::prelude::*;
use std::collections::HashMap;
use crate::{menu::Menu, plane::Plane, world::MainCamera};

#[derive(Message)]
pub struct Sound {
    pub name: &'static str,
    pub pos: Option<Vec3>,
    pub range: f32,
}

impl Sound {
    pub fn effect(name: &'static str, pos: Option<Vec3>) -> Self { Self { name, pos, range: 1500.0 } }
}

#[derive(Resource)]
pub struct Sounds(HashMap<String, bool>);

#[derive(Component)]
pub struct Emitter {
    pub range: f32,
    pub gain: f32,
    pub loops: bool,
}

#[derive(Component)]
struct Engine;

pub fn plugin(app: &mut App) {
    let path = crate::asset_dir().join("audio/index.json");
    let index = std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    app.insert_resource(Sounds(index))
        .add_message::<Sound>()
        .add_systems(Update, (effects, engine, volumes).chain());
}

/// Adds a source to an entity without disturbing its model or animation.
pub fn source(commands: &mut Commands, assets: &AssetServer, sounds: &Sounds, entity: Entity, name: &str, range: f32) {
    let Some(&loops) = sounds.0.get(name) else { return };
    // One-shot ambience repeats with silence between calls rather than chirping continuously.
    let settings = if loops { PlaybackSettings::LOOP } else { PlaybackSettings::DESPAWN };
    commands.entity(entity).with_child((
        AudioPlayer::new(assets.load(format!("audio/{name}.wav"))),
        settings.with_spatial(true).with_spatial_scale(SpatialScale::new(0.001)).with_volume(Volume::SILENT),
        Transform::default(),
        Visibility::default(),
        Emitter { range, gain: 0.65, loops },
    ));
}

fn effects(mut commands: Commands, assets: Res<AssetServer>, sounds: Res<Sounds>, mut events: MessageReader<Sound>) {
    for sound in events.read() {
        if !sounds.0.contains_key(sound.name) { continue; }
        let mut e = commands.spawn((
            AudioPlayer::new(assets.load(format!("audio/{}.wav", sound.name))),
            PlaybackSettings::DESPAWN.with_volume(Volume::Linear(0.65)).with_spatial(sound.pos.is_some()).with_spatial_scale(SpatialScale::new(0.001)),
            Transform::from_translation(sound.pos.unwrap_or_default()),
        ));
        if sound.pos.is_some() { e.insert(Emitter { range: sound.range, gain: 0.65, loops: false }); }
    }
}

fn engine(
    mut commands: Commands, assets: Res<AssetServer>, sounds: Res<Sounds>,
    plane: Query<&Plane>, engines: Query<Entity, With<Engine>>,
    mut sinks: Query<&mut AudioSink, With<Engine>>, menu: Res<Menu>,
) {
    let Ok(plane) = plane.single() else {
        for e in &engines { commands.entity(e).despawn(); }
        return;
    };
    if engines.is_empty() && sounds.0.contains_key("SE_PLN_ENGINE") {
        commands.spawn((AudioPlayer::new(assets.load("audio/SE_PLN_ENGINE.wav")), PlaybackSettings::LOOP.with_volume(Volume::Linear(0.25)), Engine));
    }
    for mut sink in &mut sinks {
        if menu.open { sink.pause(); } else { sink.play(); }
        sink.set_speed(0.7 + plane.throttle * 0.35 + plane.vel.length().min(12.0) * 0.025);
        sink.set_volume(Volume::Linear(0.1 + plane.throttle * 0.2));
    }
}

fn volumes(
    cam: Single<&GlobalTransform, With<MainCamera>>, menu: Res<Menu>,
    mut emitters: Query<(&Emitter, &GlobalTransform, Option<&InheritedVisibility>, &mut SpatialAudioSink)>,
) {
    for (emitter, transform, visible, mut sink) in &mut emitters {
        let on = !menu.open && visible.is_none_or(|v| v.get());
        let gain = if on { attenuation(cam.translation().distance(transform.translation()), emitter.range) * emitter.gain } else { 0.0 };
        sink.set_volume(Volume::Linear(gain));
        // Muted one-shots must drain so their DESPAWN policy can reclaim them.
        if menu.open || (gain == 0.0 && emitter.loops) { sink.pause(); } else { sink.play(); }
    }
}

pub fn attenuation(distance: f32, range: f32) -> f32 {
    (1.0 - distance / range.max(1.0)).clamp(0.0, 1.0).powi(2)
}

/// Sound-area class mappings recovered from the disc's scenery factory.
pub fn ambience(kind: u16, time: u8) -> Option<&'static str> {
    Some(match kind {
        0 => "SE_CMN_RIVER_L",
        2 => "SE_CMN_CAT",
        3 => ["SE_CMN_BIRD_CHIRP", "SE_CMN_EVENING_BIRD_CHIRP", "SE_CMN_NIGHT_BIRD_CHIRP"][time as usize],
        4 => "SE_CMN_WAVE_BEACH",
        5 => "SE_CMN_WAVE_QUAY",
        6 => "SE_CMN_AUDIENCE_CHEER",
        7 | 8 => "SE_CMN_WATER_FALL",
        9 => "SE_CMN_WAVE_LAKE",
        10 if time == 2 => "SE_CMN_NIGHT_FOREST_L",
        10 => "SE_CMN_HIGHLAND_BIRD_CHIRP",
        11 => "SE_CMN_SEA_CAVE",
        12 => "SE_CMN_SHIP_HORN",
        18 | 19 => "SE_CMN_RUIN_SONG_CAVE",
        21 => "SE_CMN_GOLF_ISLAND",
        22 => "SE_CMN_BOWLING_ARRAY_OUT",
        23 | 25 => "SE_CMN_HOUSE_01",
        26 => "SE_CMN_BASKET_COURT",
        27 => "SE_CMN_RIVER_CAVE",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sound_radius_is_bounded() {
        assert_eq!(attenuation(0.0, 100.0), 1.0);
        assert_eq!(attenuation(50.0, 100.0), 0.25);
        assert_eq!(attenuation(101.0, 100.0), 0.0);
        assert!(attenuation(0.0, 0.0).is_finite());
    }
}
