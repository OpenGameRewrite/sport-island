use bevy::asset::AssetPlugin;
use bevy::prelude::*;
use std::path::PathBuf;

mod collide;
mod debug;
mod gx;
mod input;
mod ipoint;
mod lang;
mod menu;
mod motion;
mod particles;
mod plane;
mod player;
mod pwr;
mod save;
mod scenery;
mod sound;
mod vehicles;
mod world;

#[derive(States, Default, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AppState {
    /// Language select / title screen.
    #[default]
    Title,
    Play,
}

/// Whether the player is walking or flying.
#[derive(States, Default, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mode {
    #[default]
    Walk,
    Fly,
}

/// Extracted assets live in the working directory, next to the executable, or at the repo root.
pub fn asset_dir() -> PathBuf {
    let candidates = [
        std::env::current_dir().ok().map(|d| d.join("assets")),
        std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.join("assets"))),
        Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../assets")),
    ];
    candidates.into_iter().flatten().find(|p| p.join("VERSION").exists()).unwrap_or_else(|| {
        eprintln!("No extracted assets found. Run sport-island-extract with your Wii Sports Resort disc first.");
        std::process::exit(1);
    })
}

/// Reads an extracted JSON file.
pub fn read_json<T: serde::de::DeserializeOwned>(rel: &str) -> T {
    let path = asset_dir().join(rel);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_slice(&bytes).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn main() {
    App::new()
        .add_plugins(
            DefaultPlugins
                .set(AssetPlugin { file_path: asset_dir().to_string_lossy().into_owned(), ..default() })
                .set(WindowPlugin { primary_window: Some(Window { title: "Sport Island".into(), resolution: (1600, 900).into(),
                    // Bevy assumes focus until Windows reports otherwise; a window opened in the
                    // background would then confine the cursor. Wait for a real focus event.
                    focused: false, ..default() }), ..default() }),
        )
        .insert_resource(Time::<Fixed>::from_hz(60.0))
        .init_state::<AppState>()
        .init_state::<Mode>()
        .add_plugins((save::plugin, lang::plugin, input::plugin, collide::plugin, gx::plugin, particles::plugin, world::plugin))
        .add_plugins((player::plugin, plane::plugin, ipoint::plugin, menu::plugin, scenery::plugin, sound::plugin, debug::plugin, pwr::plugin, vehicles::plugin, motion::plugin))
        .run();
}
