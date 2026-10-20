//! Persistent progress and settings (a small JSON file in the user's data dir).

use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResDef {
    pub w: u32,
    pub h: u32,
    pub aspect: &'static str,
}

pub const STANDARD_RESOLUTIONS: &[ResDef] = &[
    // 16:9
    ResDef { w: 960,  h: 540,  aspect: "16:9" },
    ResDef { w: 1024, h: 576,  aspect: "16:9" },
    ResDef { w: 1280, h: 720,  aspect: "16:9" },
    ResDef { w: 1366, h: 768,  aspect: "16:9" },
    ResDef { w: 1600, h: 900,  aspect: "16:9" },
    ResDef { w: 1920, h: 1080, aspect: "16:9" },
    ResDef { w: 2560, h: 1440, aspect: "16:9" },
    ResDef { w: 3840, h: 2160, aspect: "16:9" },
    // 16:10
    ResDef { w: 960,  h: 600,  aspect: "16:10" },
    ResDef { w: 1280, h: 800,  aspect: "16:10" },
    ResDef { w: 1440, h: 900,  aspect: "16:10" },
    ResDef { w: 1680, h: 1050, aspect: "16:10" },
    ResDef { w: 1920, h: 1200, aspect: "16:10" },
    ResDef { w: 2560, h: 1600, aspect: "16:10" },
    ResDef { w: 2880, h: 1800, aspect: "16:10" },
    // 4:3 & 5:4
    ResDef { w: 800,  h: 600,  aspect: "4:3" },
    ResDef { w: 1024, h: 768,  aspect: "4:3" },
    ResDef { w: 1280, h: 960,  aspect: "4:3" },
    ResDef { w: 1280, h: 1024, aspect: "5:4" },
    ResDef { w: 1400, h: 1050, aspect: "4:3" },
    ResDef { w: 1600, h: 1200, aspect: "4:3" },
    // 21:9
    ResDef { w: 2560, h: 1080, aspect: "21:9" },
    ResDef { w: 3440, h: 1440, aspect: "21:9" },
    ResDef { w: 5120, h: 2160, aspect: "21:9" },
    // 3:2
    ResDef { w: 1920, h: 1280, aspect: "3:2" },
    ResDef { w: 2160, h: 1440, aspect: "3:2" },
    ResDef { w: 2256, h: 1504, aspect: "3:2" },
    ResDef { w: 3000, h: 2000, aspect: "3:2" },
];

pub const ASPECT_RATIO_LABELS: &[&str] = &["Auto", "16:9", "16:10", "4:3", "21:9", "All"];
pub const MSAA_LABELS: &[&str] = &["Off", "2×", "4×"];
pub const DEFAULT_RESOLUTION_W: u32 = 1600;
pub const DEFAULT_RESOLUTION_H: u32 = 900;

pub fn detect_aspect_ratio(w: u32, h: u32) -> &'static str {
    if h == 0 { return "16:9"; }
    let ratio = w as f32 / h as f32;
    if (1.70..=1.85).contains(&ratio) {
        "16:9"
    } else if (1.56..=1.65).contains(&ratio) {
        "16:10"
    } else if (1.20..=1.40).contains(&ratio) {
        "4:3"
    } else if (2.20..=2.50).contains(&ratio) {
        "21:9"
    } else if (1.45..=1.55).contains(&ratio) {
        "3:2"
    } else {
        "16:9"
    }
}

pub fn available_resolutions(aspect_idx: u8, monitor: Option<(u32, u32)>) -> Vec<(u32, u32, String)> {
    let active_aspect = match aspect_idx {
        0 => monitor.map(|(w, h)| detect_aspect_ratio(w, h)).unwrap_or("16:9"),
        1 => "16:9",
        2 => "16:10",
        3 => "4:3",
        4 => "21:9",
        _ => "All",
    };

    let mut list = Vec::new();
    let show_aspect_tag = active_aspect == "All";

    for r in STANDARD_RESOLUTIONS {
        let matches = match active_aspect {
            "All" => true,
            "4:3" => r.aspect == "4:3" || r.aspect == "5:4",
            tag => r.aspect == tag,
        };
        if matches {
            let label = if show_aspect_tag {
                format!("{} × {} ({})", r.w, r.h, r.aspect)
            } else {
                format!("{} × {}", r.w, r.h)
            };
            list.push((r.w, r.h, label));
        }
    }

    if let Some((mw, mh)) = monitor && mw > 0 && mh > 0 {
        let m_aspect = detect_aspect_ratio(mw, mh);
        let matches = match active_aspect {
            "All" => true,
            "4:3" => m_aspect == "4:3" || m_aspect == "5:4",
            tag => m_aspect == tag,
        };
        if matches && !list.iter().any(|(w, h, _)| *w == mw && *h == mh) {
            let label = format!("{} × {} (Native)", mw, mh);
            list.push((mw, mh, label));
            list.sort_by_key(|(w, h, _)| (*w, *h));
        }
    }

    if list.is_empty() {
        list.push((DEFAULT_RESOLUTION_W, DEFAULT_RESOLUTION_H, format!("{} × {}", DEFAULT_RESOLUTION_W, DEFAULT_RESOLUTION_H)));
    }
    list
}

#[derive(Resource, Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Save {
    /// BMG language code (EN, FR, GE, SP, IT); None until chosen on the title screen.
    pub lang: Option<String>,
    /// Collected i-point readings: index * 3 + time of day (80 points x 3 times = 240, as in the original).
    pub collected: BTreeSet<u16>,
    /// 0 day, 1 dusk, 2 night.
    pub time: u8,
    /// 0 = standard plane, 1 = second plane model, 2.. = Pilotwings Resort
    /// vehicles (`vehicles::KINDS`) when that add-on is installed.
    pub plane: u8,
    pub invert_pitch: bool,
    /// Only overrides; absent entries keep the published defaults.
    pub bindings: std::collections::BTreeMap<String, [String; 2]>,
    pub livery: u8,
    pub flight_sensitivity: f32,
    pub look_sensitivity: f32,
    pub speedometer: bool,
    pub fullscreen: bool,
    /// 0 = Auto, 1 = 16:9, 2 = 16:10, 3 = 4:3, 4 = 21:9, 5 = All.
    pub aspect_ratio: u8,
    pub resolution_w: u32,
    pub resolution_h: u32,
    /// Index into `MSAA_LABELS` (0 = Off, 1 = 2x, 2 = 4x).
    pub msaa: u8,
    /// Strength of the original lift away from nearby ground (1 = original).
    pub ground_assist: f32,
    /// Mii favourite colour (`colortex.val` order) for add-on vehicle liveries.
    pub favorite: u8,
    /// Fly the 3DS plane and jet with the Wii plane's physics instead of their own.
    pub plane_wii_style: bool,
    /// Pilotwings Resort collectibles found, by set (`collect::SETS`).
    pub found: std::collections::BTreeMap<String, BTreeSet<u16>>,
}

impl Default for Save {
    fn default() -> Self {
        Self {
            lang: None, collected: BTreeSet::new(), time: 0, plane: 0, invert_pitch: false,
            bindings: Default::default(), livery: 0, flight_sensitivity: 1.0, look_sensitivity: 1.0,
            speedometer: false, fullscreen: false,
            aspect_ratio: 0,
            resolution_w: DEFAULT_RESOLUTION_W,
            resolution_h: DEFAULT_RESOLUTION_H,
            msaa: 2,
            ground_assist: 1.0, favorite: 0, plane_wii_style: false, found: Default::default(),
        }
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

pub fn load_from(p: &std::path::Path) -> Save {
    let mut save: Save = std::fs::read(p).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    save.time = save.time.min(2);
    save.plane = save.plane.min(1 + if crate::pwr::available() { crate::vehicles::KINDS.len() as u8 } else { 0 });
    save.favorite = save.favorite.min(11);
    save.livery = save.livery.min(2);
    if save.resolution_w == 0 || save.resolution_h == 0 {
        save.resolution_w = DEFAULT_RESOLUTION_W;
        save.resolution_h = DEFAULT_RESOLUTION_H;
    }
    save.aspect_ratio = save.aspect_ratio.min((ASPECT_RATIO_LABELS.len() - 1) as u8);
    save.msaa = save.msaa.min((MSAA_LABELS.len() - 1) as u8);
    for v in [&mut save.flight_sensitivity, &mut save.look_sensitivity] {
        *v = if v.is_finite() { v.clamp(0.25, 1.5) } else { 1.0 };
    }
    save.ground_assist = if save.ground_assist.is_finite() { save.ground_assist.clamp(0.0, 1.0) } else { 1.0 };
    save.collected.retain(|i| (*i as usize) < crate::ipoint::MAX_READINGS);
    save
}

pub fn load() -> Save {
    load_from(&path())
}

pub fn msaa_sample(msaa: u8) -> Msaa {
    match msaa {
        0 => Msaa::Off,
        1 => Msaa::Sample2,
        _ => Msaa::Sample4,
    }
}

pub fn plugin(app: &mut App) {
    if !app.world().contains_resource::<Save>() {
        let save = load();
        app.insert_resource::<Save>(save);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn save_can_replace_an_existing_file_and_keep_readings() {
        let old: Save = serde_json::from_str("{}").unwrap();
        assert_eq!(old.flight_sensitivity, 1.0);
        assert_eq!(old.look_sensitivity, 1.0);
        assert_eq!(old.resolution_w, DEFAULT_RESOLUTION_W);
        assert_eq!(old.resolution_h, DEFAULT_RESOLUTION_H);
        assert_eq!(old.msaa, 2);
        let p = std::env::temp_dir().join(format!("sport-island-save-{}.json", std::process::id()));
        let mut save = Save::default();
        save.write_to(&p).unwrap();
        save.collected.insert(239);
        save.lang = Some("SP".into());
        save.plane = 1;
        save.livery = 1;
        save.flight_sensitivity = 0.6;
        save.speedometer = true;
        save.aspect_ratio = 2;
        save.resolution_w = 1920;
        save.resolution_h = 1200;
        save.msaa = 2;
        save.bindings.insert("Board / eject".into(), ["KeyQ".into(), "RightThumb".into()]);
        save.write_to(&p).unwrap();
        let read: Save = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        assert_eq!(read.collected, save.collected);
        assert_eq!(read.lang, save.lang);
        assert_eq!(read.plane, 1);
        assert_eq!(read.livery, 1);
        assert_eq!(read.flight_sensitivity, 0.6);
        assert!(read.speedometer);
        assert_eq!(read.aspect_ratio, 2);
        assert_eq!(read.resolution_w, 1920);
        assert_eq!(read.resolution_h, 1200);
        assert_eq!(read.msaa, 2);
        assert_eq!(read.bindings, save.bindings);
        std::fs::remove_file(p).unwrap();
    }

    #[test]
    fn aspect_ratio_filtering_works() {
        let r16_9 = available_resolutions(1, None);
        assert!(r16_9.iter().all(|(w, h, _)| (*w as f32 / *h as f32 - 1.777).abs() < 0.05));
        let r16_10 = available_resolutions(2, None);
        assert!(r16_10.iter().all(|(w, h, _)| (*w as f32 / *h as f32 - 1.6).abs() < 0.05));
        let all = available_resolutions(5, None);
        assert!(all.len() >= STANDARD_RESOLUTIONS.len());
    }
}
