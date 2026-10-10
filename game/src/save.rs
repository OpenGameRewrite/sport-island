//! Persistent progress and settings (a small JSON file in the user's data dir).

use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::PathBuf;

#[derive(Resource, Serialize, Deserialize, Clone)]
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
    /// Only overrides; absent entries keep the published defaults.
    pub bindings: std::collections::BTreeMap<String, [String; 2]>,
    pub livery: u8,
    pub flight_sensitivity: f32,
    pub look_sensitivity: f32,
    pub speedometer: bool,
}

impl Default for Save {
    fn default() -> Self {
        Self { lang: None, collected: BTreeSet::new(), time: 0, plane: 0, invert_pitch: false,
            bindings: Default::default(), livery: 0, flight_sensitivity: 1.0, look_sensitivity: 1.0, speedometer: false }
    }
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
        if let Err(e) = self.write_to(&p) { error!("Could not save progress to {}: {e}", p.display()); }
    }

    fn write_to(&self, p: &std::path::Path) -> std::io::Result<()> {
        if let Some(parent) = p.parent().filter(|p| !p.as_os_str().is_empty()) { std::fs::create_dir_all(parent)?; }
        let tmp = p.with_extension("tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(tmp, p)
    }
}

pub fn plugin(app: &mut App) {
    let mut save: Save = std::fs::read(path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    save.time = save.time.min(2);
    save.plane = save.plane.min(1);
    save.livery = save.livery.min(2);
    for v in [&mut save.flight_sensitivity, &mut save.look_sensitivity] {
        *v = if v.is_finite() { v.clamp(0.25, 1.5) } else { 1.0 };
    }
    save.collected.retain(|i| (*i as usize) < crate::ipoint::READINGS);
    app.insert_resource::<Save>(save);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn save_can_replace_an_existing_file_and_keep_readings() {
        let old: Save = serde_json::from_str("{}").unwrap();
        assert_eq!(old.flight_sensitivity, 1.0);
        assert_eq!(old.look_sensitivity, 1.0);
        let p = std::env::temp_dir().join(format!("sport-island-save-{}.json", std::process::id()));
        let mut save = Save::default();
        save.write_to(&p).unwrap();
        save.collected.insert(239);
        save.lang = Some("SP".into());
        save.plane = 1;
        save.livery = 1;
        save.flight_sensitivity = 0.6;
        save.speedometer = true;
        save.bindings.insert("Board / eject".into(), ["KeyQ".into(), "RightThumb".into()]);
        save.write_to(&p).unwrap();
        let read: Save = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        assert_eq!(read.collected, save.collected);
        assert_eq!(read.lang, save.lang);
        assert_eq!(read.plane, 1);
        assert_eq!(read.livery, 1);
        assert_eq!(read.flight_sensitivity, 0.6);
        assert!(read.speedometer);
        assert_eq!(read.bindings, save.bindings);
        std::fs::remove_file(p).unwrap();
    }
}
