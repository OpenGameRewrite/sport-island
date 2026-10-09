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

#[derive(Resource, Default)]
pub struct Texts {
    pub code: String,
    map: HashMap<u32, String>,
}

impl Texts {
    pub fn load(code: &str) -> Self {
        let raw: HashMap<String, String> = crate::read_json(&format!("text/{code}.json"));
        let map = raw.into_iter().filter_map(|(k, v)| Some((u32::from_str_radix(&k, 16).ok()?, v))).collect();
        Self { code: code.to_string(), map }
    }

    /// A BMG message with icon glyphs replaced by readable labels.
    pub fn msg(&self, id: u32, pad: bool) -> String {
        let Some(s) = self.map.get(&id) else { return String::new() };
        s.chars()
            .map(|c| match c as u32 {
                0x332B => "ⓘ".to_string(),
                0xE041 => key(pad, "Right Stick", "Mouse"),
                0xE042 => key(pad, "A", "Space"),
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
    ChooseLanguage,
    BoardHint,
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
    ["Choose your language", "Choisissez votre langue", "Sprache wählen", "Elige tu idioma", "Scegli la lingua"],
    ["Board plane", "Monter dans l'avion", "Ins Flugzeug", "Subir al avión", "Sali sull'aereo"],
];

pub fn plugin(app: &mut App) {
    let langs: Vec<Language> = crate::read_json("text/languages.json");
    let code = app.world().resource::<Save>().lang.clone().unwrap_or_else(|| langs[0].code.clone());
    app.insert_resource(Texts::load(&code)).insert_resource(Languages(langs));
}
