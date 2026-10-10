//! Game text from the disc's BMG files, plus a handful of UI strings of our own.

use crate::save::Save;
use bevy::prelude::*;
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Deserialize, Clone)]
pub struct Language {
    pub code: String,
    pub name: String,
}

#[derive(Resource)]
pub struct Languages(pub Vec<Language>);

/// Hold system font data as assets: the text shaper otherwise evicts file-backed
/// fonts between layouts and gives their glyph atlases a new identity on reload.
#[derive(Resource)]
pub struct UiFont(pub FontSource);

#[derive(Resource, Default)]
pub struct Texts {
    pub code: String,
    map: HashMap<u32, String>,
    /// Pilotwings Resort strings by key, when that add-on was extracted.
    pwr: HashMap<String, String>,
}

impl Texts {
    pub fn load(code: &str) -> Self {
        let raw: HashMap<String, String> = crate::read_json(&format!("text/{code}.json"));
        let map = raw.into_iter().filter_map(|(k, v)| Some((u32::from_str_radix(&k, 16).ok()?, v))).collect();
        let pwr = std::fs::read(crate::asset_dir().join(format!("pwr/text/{code}.json"))).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        Self { code: code.to_string(), map, pwr }
    }

    /// A Pilotwings Resort string; its inline icon escapes become the info glyph.
    pub fn pwr(&self, key: &str) -> Option<String> {
        let s = self.pwr.get(key)?;
        Some(s.replace("\\ipoint_icon_small", "ⓘ "))
    }

    /// A BMG message with icon glyphs replaced by readable labels.
    pub fn msg(&self, id: u32, pad: bool) -> String {
        let Some(s) = self.map.get(&id) else { return String::new() };
        s.chars()
            .map(|c| match c as u32 {
                0x332B => "ⓘ".to_string(),
                0xE041 => key(pad, self.ui(Ui::RightStick), self.ui(Ui::Mouse)),
                0xE042 => key(pad, "A", self.ui(Ui::Space)),
                0xE043 => key(pad, "B", "C"),
                0xE045 => key(pad, "Start", "Esc"),
                0xE047 => key(pad, "Y", "F"),
                0xE000..=0xF8FF | 0x3300..=0x33FF => String::new(),
                _ => c.to_string(),
            })
            .collect()
    }

    /// Our own UI strings (the disc has no standalone words for these).
    pub fn ui(&self, key: Ui) -> &'static str {
        let i = ["EN", "FR", "GE", "SP", "IT"].iter().position(|c| *c == self.code).unwrap_or(0);
        UI[key as usize][i]
    }

    pub fn action(&self, index: usize) -> &'static str {
        let i = ["EN", "FR", "GE", "SP", "IT"].iter().position(|c| *c == self.code).unwrap_or(0);
        ACTIONS[index][i]
    }

    pub fn binding(&self, raw: &str) -> String {
        let special = match raw {
            "Space" => Some(Ui::Space), "ShiftLeft" => Some(Ui::LeftShift), "ControlLeft" => Some(Ui::LeftCtrl),
            "MouseLeft" => Some(Ui::MouseLeft), "MouseRight" => Some(Ui::MouseRight), "MouseMiddle" => Some(Ui::MouseMiddle),
            "DPadLeft" => Some(Ui::DLeft), "DPadRight" => Some(Ui::DRight), "DPadUp" => Some(Ui::DUp), "DPadDown" => Some(Ui::DDown),
            "South" => Some(Ui::South), "East" => Some(Ui::East), "West" => Some(Ui::West), "North" => Some(Ui::North),
            "Escape" => Some(Ui::Escape), "Select" => Some(Ui::Select),
            _ => None,
        };
        special.map_or_else(|| crate::input::label(raw), |s| self.ui(s).into())
    }
}

fn key(pad: bool, gamepad: &str, keyboard: &str) -> String {
    format!("[{}]", if pad { gamepad } else { keyboard })
}

#[derive(Clone, Copy)]
pub enum Ui {
    Resume,
    TimeOfDay,
    Day,
    Dusk,
    Night,
    IPoints,
    Plane,
    Plane1,
    Plane2,
    InvertPitch,
    On,
    Off,
    Language,
    Quit,
    Back,
    BoardHint,
    Controls,
    Livery,
    ResetControls,
    Start, Settings, FlightSensitivity, LookSensitivity, Speedometer, Speed, Units,
    Boost, Slow, Engine, Waiting, MenuHint, ControlsHint, RecordHint, NextBoarding,
    Red, White, Yellow, Orange, Blue, Space, LeftShift, LeftCtrl, MouseLeft, MouseRight, MouseMiddle,
    DLeft, DRight, DUp, DDown,
    South, East, West, North, Escape, Select, RightStick, Mouse, Fullscreen, GroundAssist,
}

const UI: &[[&str; 5]] = &[
    ["Resume", "Reprendre", "Weiter", "Continuar", "Riprendi"],
    ["Time of day", "Moment de la journée", "Tageszeit", "Hora del día", "Ora del giorno"],
    ["Daytime", "Jour", "Tag", "Día", "Giorno"],
    ["Evening", "Soir", "Abend", "Atardecer", "Sera"],
    ["Night", "Nuit", "Nacht", "Noche", "Notte"],
    ["i-points", "Points ⓘ", "ⓘ-Punkte", "Puntos ⓘ", "Punti ⓘ"],
    ["Plane", "Avion", "Flugzeug", "Avión", "Aereo"],
    ["Single-seater", "Monoplace", "Einsitzer", "Monoplaza", "Monoposto"],
    ["Two-seater", "Biplace", "Zweisitzer", "Biplaza", "Biposto"],
    ["Invert pitch", "Inverser la profondeur", "Höhenruder invertieren", "Invertir cabeceo", "Inverti beccheggio"],
    ["On", "Oui", "An", "Sí", "Sì"],
    ["Off", "Non", "Aus", "No", "No"],
    ["Language", "Langue", "Sprache", "Idioma", "Lingua"],
    ["Quit", "Quitter", "Beenden", "Salir", "Esci"],
    ["Back", "Retour", "Zurück", "Volver", "Indietro"],
    ["Board plane", "Monter dans l'avion", "Ins Flugzeug", "Subir al avión", "Sali sull'aereo"],
    ["Controls", "Commandes", "Steuerung", "Controles", "Comandi"],
    ["Livery", "Couleur", "Farbe", "Color", "Colore"],
    ["Reset controls", "Réinitialiser", "Zurücksetzen", "Restablecer controles", "Ripristina comandi"],
    ["Explore", "Explorer", "Erkunden", "Explorar", "Esplora"],
    ["Settings", "Options", "Einstellungen", "Opciones", "Impostazioni"],
    ["Flight sensitivity", "Sensibilité du pilotage", "Flugempfindlichkeit", "Sensibilidad de vuelo", "Sensibilità di volo"],
    ["Look sensitivity", "Sensibilité de la caméra", "Kameraempfindlichkeit", "Sensibilidad de cámara", "Sensibilità della visuale"],
    ["Speedometer", "Indicateur de vitesse", "Geschwindigkeitsanzeige", "Velocímetro", "Indicatore di velocità"],
    ["Speed", "Vitesse", "Geschwindigkeit", "Velocidad", "Velocità"],
    ["units/s", "unités/s", "Einheiten/s", "unidades/s", "unità/s"],
    ["Boost impulse", "Impulsion d'accélération", "Beschleunigungsimpuls", "Impulso de aceleración", "Impulso di accelerazione"],
    ["Slow impulse", "Impulsion de freinage", "Bremsimpuls", "Impulso de frenado", "Impulso di frenata"],
    ["Engine", "Moteur", "Motor", "Motor", "Motore"],
    ["Press a key or button… (Esc: cancel)", "Appuyez sur une touche… (Échap : annuler)", "Taste drücken… (Esc: abbrechen)", "Pulsa una tecla o botón… (Esc: cancelar)", "Premi un tasto… (Esc: annulla)"],
    ["Click to select · ‹ › to change · Esc: back", "Cliquez pour choisir · ‹ › modifier · Échap : retour", "Klicken: wählen · ‹ › ändern · Esc: zurück", "Clic para elegir · ‹ › cambiar · Esc: volver", "Clic per scegliere · ‹ › cambia · Esc: indietro"],
    ["Select an action, then press its new key or button", "Choisissez une action, puis sa nouvelle touche", "Aktion wählen, dann neue Taste drücken", "Elige una acción y pulsa su nueva tecla o botón", "Scegli un'azione, poi premi il nuovo tasto"],
    ["↑ ↓ / wheel: scroll · ‹ ›: page · Day / Evening / Night", "↑ ↓ / molette : défiler · ‹ › : page · Jour / Soir / Nuit", "↑ ↓ / Mausrad: scrollen · ‹ ›: Seite · Tag / Abend / Nacht", "↑ ↓ / rueda: desplazar · ‹ ›: página · Día / Atardecer / Noche", "↑ ↓ / rotella: scorri · ‹ ›: pagina · Giorno / Sera / Notte"],
    ["next boarding", "au prochain embarquement", "beim nächsten Einsteigen", "al volver a subir", "al prossimo imbarco"],
    ["Red", "Rouge", "Rot", "Rojo", "Rosso"], ["White", "Blanc", "Weiß", "Blanco", "Bianco"],
    ["Yellow", "Jaune", "Gelb", "Amarillo", "Giallo"], ["Orange", "Orange", "Orange", "Naranja", "Arancione"], ["Blue", "Bleu", "Blau", "Azul", "Blu"],
    ["Space", "Espace", "Leertaste", "Espacio", "Spazio"],
    ["Left Shift", "Maj gauche", "Umschalt links", "Mayús izquierda", "Maiusc sinistro"],
    ["Left Ctrl", "Ctrl gauche", "Strg links", "Ctrl izquierda", "Ctrl sinistro"],
    ["Left click", "Clic gauche", "Linksklick", "Clic izquierdo", "Clic sinistro"],
    ["Right click", "Clic droit", "Rechtsklick", "Clic derecho", "Clic destro"],
    ["Middle click", "Clic central", "Mittelklick", "Clic central", "Clic centrale"],
    ["D-pad ←", "Croix ←", "Steuerkreuz ←", "Cruceta ←", "Croce ←"],
    ["D-pad →", "Croix →", "Steuerkreuz →", "Cruceta →", "Croce →"],
    ["D-pad ↑", "Croix ↑", "Steuerkreuz ↑", "Cruceta ↑", "Croce ↑"],
    ["D-pad ↓", "Croix ↓", "Steuerkreuz ↓", "Cruceta ↓", "Croce ↓"],
    ["A / Cross", "A / Croix", "A / Kreuz", "A / Cruz", "A / Croce"],
    ["B / Circle", "B / Cercle", "B / Kreis", "B / Círculo", "B / Cerchio"],
    ["X / Square", "X / Carré", "X / Quadrat", "X / Cuadrado", "X / Quadrato"],
    ["Y / Triangle", "Y / Triangle", "Y / Dreieck", "Y / Triángulo", "Y / Triangolo"],
    ["Escape", "Échap", "Esc", "Esc", "Esc"],
    ["Back / Select", "Retour / Select", "Zurück / Select", "Atrás / Select", "Indietro / Select"],
    ["Right stick", "Stick droit", "Rechter Stick", "Stick derecho", "Stick destro"],
    ["Mouse", "Souris", "Maus", "Ratón", "Mouse"],
    ["Fullscreen", "Plein écran", "Vollbild", "Pantalla completa", "Schermo intero"],
    ["Ground assist", "Aide près du sol", "Bodenhilfe", "Ayuda cerca del suelo", "Aiuto vicino al suolo"],
];

const ACTIONS: &[[&str; 5]] = &[
    ["Left", "Gauche", "Links", "Izquierda", "Sinistra"], ["Right", "Droite", "Rechts", "Derecha", "Destra"],
    ["Forward / nose down", "Avancer / piquer", "Vorwärts / Nase senken", "Avanzar / bajar morro", "Avanti / picchia"],
    ["Back / nose up", "Reculer / cabrer", "Rückwärts / Nase heben", "Retroceder / subir morro", "Indietro / cabra"],
    ["Run / boost impulse", "Courir / accélération", "Laufen / Beschleunigungsimpuls", "Correr / impulso de aceleración", "Corri / accelerazione"],
    ["Slow impulse", "Impulsion de freinage", "Bremsimpuls", "Impulso de frenado", "Impulso di frenata"],
    ["Jump / shoot", "Sauter / tirer", "Springen / schießen", "Saltar / disparar", "Salta / spara"],
    ["Cut engine", "Couper le moteur", "Motor aus", "Apagar motor", "Spegni motore"],
    ["Signal flare", "Fusée éclairante", "Leuchtkugel", "Bengala de señales", "Razzo di segnalazione"],
    ["Board / parachute", "Embarquer / parachute", "Einsteigen / Fallschirm", "Subir / paracaídas", "Sali / paracadute"],
    ["Pause", "Pause", "Pause", "Pausa", "Pausa"], ["Time of day", "Moment de la journée", "Tageszeit", "Hora del día", "Ora del giorno"],
    ["i-point list", "Liste des points ⓘ", "ⓘ-Punkteliste", "Lista de puntos ⓘ", "Elenco punti ⓘ"],
    ["Speedometer", "Indicateur de vitesse", "Geschwindigkeitsanzeige", "Velocímetro", "Indicatore di velocità"],
    ["Recenter camera", "Recentrer la caméra", "Kamera zentrieren", "Centrar cámara", "Centra la telecamera"],
];

pub fn plugin(app: &mut App) {
    let font = app.world_mut().resource_scope(|world, mut cx: Mut<bevy::text::FontCx>| {
        let mut names: Vec<String> = [FontSource::sans_serif(), FontSource::Generic(bevy::text::GenericFontFamily::Emoji)]
            .iter().filter_map(|s| cx.get_family(s).map(str::to_owned)).collect();
        // Windows separates some arrows and enclosed information symbols.
        names.push("Segoe UI Symbol".into());
        let mut sources = Vec::new();
        for name in names {
            if let Some(family) = cx.collection.family_by_name(&name)
                && let Some(data) = family.default_font().and_then(|f| f.load(Some(&mut cx.source_cache))) {
                let handle = world.resource_mut::<Assets<Font>>().add(Font {data, alias:String::new()});
                sources.push(FontSource::Handle(handle));
            }
        }
        sources.push(FontSource::sans_serif());
        FontSource::List(sources)
    });
    app.insert_resource(UiFont(font));
    let langs: Vec<Language> = crate::read_json("text/languages.json");
    let code = app.world().resource::<Save>().lang.clone().unwrap_or_else(|| langs[0].code.clone());
    app.insert_resource(Texts::load(&code)).insert_resource(Languages(langs));
}
