//! Persistent progress and settings (a small JSON file in the user's data dir).

use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::PathBuf;

#[derive(Resource, Serialize, Deserialize, Default, Clone)]
#[serde(default)]
pub struct Save {
    /// BMG language code (EN, FR, GE, SP, IT); None until chosen on the title screen.
    pub lang: Option<String>,
    /// Collected i-point readings: index * 3 + time of day (80 points x 3 times = 240, as in the original).
    pub collected: BTreeSet<u16>,
    /// 0 day, 1 dusk, 2 night.
    pub time: u8,
    /// 0 = standard plane, 1 = second plane model.
    pub plane: u8,
    pub invert_pitch: bool,
}

fn path() -> PathBuf {
    if let Some(p) = std::env::var_os("SI_SAVE") {
        return PathBuf::from(p);
    }
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("XDG_DATA_HOME").map(PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("SportIsland").join("save.json")
}

impl Save {
    pub fn store(&self) {
        let p = path();
        let _ = std::fs::create_dir_all(p.parent().unwrap());
        // Write then rename so a crash never leaves a truncated save.
        let tmp = p.with_extension("tmp");
        if std::fs::write(&tmp, serde_json::to_vec_pretty(self).unwrap()).is_ok() {
            let _ = std::fs::rename(&tmp, &p);
        }
    }
}

pub fn plugin(app: &mut App) {
    let save = std::fs::read(path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    app.insert_resource::<Save>(save);
}
