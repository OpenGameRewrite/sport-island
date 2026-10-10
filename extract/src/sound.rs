//! BRSAR / BRSTM audio extraction. Wave sounds are decoded directly, sequenced
//! sounds (RSEQ + RBNK + RWAR) are rendered offline by a small sequence player,
//! and streams are decoded from their BRSTM files. Everything becomes 16-bit WAV
//! plus `index.json` ({name: loops}).

use crate::disc::{Disc, be16, be32, bef, cstr};
use anyhow::{Context, Result};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

const RATE: u32 = 32000;
/// Looping sequences are rendered for this long plus a crossfade tail.
const LOOP_SECS: f32 = 15.0;
const FADE_SECS: f32 = 0.5;
/// One-shot sequences are cut off after this long.
const MAX_SECS: f32 = 20.0;

/// Rendered audio: samples (interleaved), channels, sample rate, loops.
type Pcm = (Vec<f32>, u16, u32, bool);

fn wanted(name: &str) -> bool {
    name.starts_with("SE_PLN_") || name.starts_with("SE_CMN_") || name.starts_with("STM_PLN_") || name == "STM_TITLE"
}

pub fn extract(disc: &mut Disc, out: &Path) -> Result<()> {
    let sar = Rsar { d: disc.read("Sound/Static/Sp2Snd.brsar")? };
    let mut cache = WaveCache::default();
    let mut index = Map::new();
    let banks = sar.table(sar.info_ref(sar.info() + 8));
    for (i, s) in sar.table(sar.info_ref(sar.info())).into_iter().enumerate() {
        let name = sar.string(be32(&sar.d, s));
        if !wanted(&name) {
            continue;
        }
        let (fid, vol, kind, detail) = (be32(&sar.d, s + 4) as usize, sar.d[s + 0x14] as f32 / 127.0, sar.d[s + 0x16], sar.info_ref(s + 0x18));
        let rendered = match kind {
            1 => {
                let bank_fid = be32(&sar.d, banks[be32(&sar.d, detail + 4) as usize] + 4) as usize;
                let (Some((seq, _)), Some((rbnk, rwar))) = (sar.file(fid), sar.file(bank_fid)) else { continue };
                let waves = cache.get(bank_fid, rwar)?;
                Some(render_seq(seq, be32(&sar.d, detail) as usize, rbnk, waves, vol, i as u32 + 1))
            }
            2 => {
                let path = format!("Sound/Static/{}", cstr(&sar.d, sar.info_ref(sar.file_info(fid) + 0xC)));
                let data = disc.read(&path)?;
                // Preserve the introduction separately from the stream's loop.
                if name == "STM_TITLE" {
                    let head = be32(&data, 0x10) as usize + 8;
                    let si = head + be32(&data, head + 4) as usize;
                    let start = be32(&data, si + 8) as usize;
                    if data[si+1] != 0 && start > 0 {
                        let mut full = data.clone(); full[si+1] = 0;
                        let (pcm, channels, rate, _) = brstm(&full)?;
                        fs::write(out.join("STM_TITLE_intro.wav"), wav(&pcm[..(start*channels as usize).min(pcm.len())], channels, rate))?;
                        index.insert("STM_TITLE_intro".into(), Value::Bool(false));
                    }
                }
                Some(brstm(&data).with_context(|| path.clone())?)
            }
            3 => {
                let Some((rwsd, rwar)) = sar.file(fid) else { continue };
                let waves = cache.get(fid, rwar)?;
                wave_sound(rwsd, be32(&sar.d, detail) as usize, waves, vol)
            }
            _ => None,
        };
        let Some((pcm, channels, rate, loops)) = rendered else { continue };
        if pcm.is_empty() {
            continue;
        }
        fs::write(out.join(format!("{name}.wav")), wav(&pcm, channels, rate))?;
        index.insert(name, Value::Bool(loops));
    }
    println!("  audio: {} sounds", index.len());
    fs::write(out.join("index.json"), serde_json::to_vec(&Value::Object(index))?)?;
    Ok(())
}

struct Rsar {
    d: Vec<u8>,
}

impl Rsar {
    fn info(&self) -> usize { be32(&self.d, 0x18) as usize + 8 }
    /// INFO section data reference (8 bytes, offset relative to the section data).
    fn info_ref(&self, o: usize) -> usize { self.info() + be32(&self.d, o + 4) as usize }
    fn table(&self, o: usize) -> Vec<usize> {
        (0..be32(&self.d, o) as usize).map(|i| self.info_ref(o + 4 + 8 * i)).collect()
    }
    fn string(&self, i: u32) -> String {
        let symb = be32(&self.d, 0x10) as usize + 8;
        let st = symb + be32(&self.d, symb) as usize;
        cstr(&self.d, symb + be32(&self.d, st + 4 + 4 * i as usize) as usize)
    }
    fn file_info(&self, fid: usize) -> usize { self.table(self.info_ref(self.info() + 24))[fid] }
    /// A file and its wave archive, found through the first group that contains it.
    fn file(&self, fid: usize) -> Option<(&[u8], &[u8])> {
        let pos = *self.table(self.info_ref(self.file_info(fid) + 0x14)).first()?;
        let group = self.table(self.info_ref(self.info() + 32))[be32(&self.d, pos) as usize];
        let item = self.table(self.info_ref(group + 0x20))[be32(&self.d, pos + 4) as usize];
        let g = |k: usize| be32(&self.d, group + k) as usize;
        let it = |k: usize| be32(&self.d, item + k) as usize;
        let file = &self.d[g(0x10) + it(4)..][..it(8)];
        let waves = &self.d[g(0x18) + it(12)..][..it(16)];
        Some((file, waves))
    }
}

struct Wave {
    rate: f32,
    data: Vec<f32>,
    loop_start: Option<usize>,
}

#[derive(Default)]
struct WaveCache(HashMap<usize, Vec<Wave>>);

impl WaveCache {
    fn get(&mut self, fid: usize, rwar: &[u8]) -> Result<&[Wave]> {
        if !self.0.contains_key(&fid) {
            let waves = parse_rwar(rwar)?;
            self.0.insert(fid, waves);
        }
        Ok(&self.0[&fid])
    }
}

fn parse_rwar(d: &[u8]) -> Result<Vec<Wave>> {
    if d.len() < 0x20 || &d[..4] != b"RWAR" {
        return Ok(vec![]);
    }
    let (tabl, data) = (be32(d, 0x10) as usize + 8, be32(d, 0x18) as usize);
    (0..be32(d, tabl) as usize).map(|i| rwav(&d[data + be32(d, tabl + 4 + 12 * i + 4) as usize..])).collect()
}

/// Decodes an RWAV to mono (channels are averaged).
fn rwav(d: &[u8]) -> Result<Wave> {
    anyhow::ensure!(&d[..4] == b"RWAV", "bad RWAV");
    let wi = be32(d, 0x10) as usize + 8;
    let (format, looped, nch) = (d[wi], d[wi + 1] != 0, d[wi + 2] as usize);
    let rate = (d[wi + 3] as u32) << 16 | be16(d, wi + 4) as u32;
    let (loop_start, loop_end) = (be32(d, wi + 8) as usize, be32(d, wi + 12) as usize);
    let tab = wi + be32(d, wi + 0x10) as usize;
    // Channel offsets are relative to the DATA section payload.
    let data = wi + be32(d, wi + 0x14) as usize + 8;
    // ADPCM positions are nibble addresses that include the frame headers.
    let to_sample = |n: usize| if format == 2 { n / 16 * 14 + (n % 16).saturating_sub(2) } else { n };
    let len = to_sample(loop_end) + 1;
    let mut mix = vec![0f32; len];
    for c in 0..nch {
        let ci = wi + be32(d, tab + 4 * c) as usize;
        let src = &d[data + be32(d, ci) as usize..];
        let samples = match format {
            0 => src.iter().take(len).map(|&b| b as i8 as f32 / 128.0).collect(),
            1 => (0..len).map(|i| be16(src, i * 2) as i16 as f32 / 32768.0).collect(),
            _ => Adpcm::new(&d[wi + be32(d, ci + 4) as usize..]).decode(src, len),
        };
        for (m, s) in mix.iter_mut().zip(samples) {
            *m += s / nch as f32;
        }
    }
    Ok(Wave { rate: rate as f32, data: mix, loop_start: looped.then(|| to_sample(loop_start)) })
}

/// GameCube/Wii DSP ADPCM: 8-byte frames of a header byte and 14 nibbles.
struct Adpcm {
    coef: [i32; 16],
    h1: i32,
    h2: i32,
}

impl Adpcm {
    fn new(info: &[u8]) -> Self { Adpcm { coef: std::array::from_fn(|i| be16(info, i * 2) as i16 as i32), h1: 0, h2: 0 } }

    fn decode(&mut self, src: &[u8], len: usize) -> Vec<f32> {
        let mut out = Vec::with_capacity(len);
        for frame in src.chunks(8) {
            let (scale, idx) = (1i32 << (frame[0] & 15), (frame[0] >> 4) as usize & 7);
            for n in 0..14 {
                if out.len() >= len || 1 + n / 2 >= frame.len() {
                    return out;
                }
                let b = frame[1 + n / 2];
                let nib = if n % 2 == 0 { (b as i8 >> 4) as i32 } else { ((b << 4) as i8 >> 4) as i32 };
                let s = ((((nib * scale) << 11) + 1024 + self.coef[idx * 2] * self.h1 + self.coef[idx * 2 + 1] * self.h2) >> 11).clamp(-32768, 32767);
                (self.h2, self.h1) = (self.h1, s);
                out.push(s as f32 / 32768.0);
            }
        }
        out
    }
}

/// Envelope + resampler for one playing note.
struct Voice<'a> {
    wave: &'a Wave,
    pos: f64,
    step: f64,
    gain: f32,
    /// Attenuation in dB, 0 = full.
    db: f32,
    attack: f32,
    level: f32,
    decay: f32,
    sustain: f32,
    release: f32,
    off_at: Option<u64>,
    done: bool,
    track: usize,
    pan: f32,
    base_step: f64,
}

/// NW4R-like decay/release rate in dB per output sample.
fn rate_db(v: u8) -> f32 {
    let per_ms = match v {
        127.. => 1e4,
        126 => 24.0,
        0..50 => (v as f32 * 2.0 + 1.0) / 128.0 / 5.0,
        _ => 60.0 / (127 - v) as f32 / 5.0,
    };
    per_ms * 1000.0 / RATE as f32
}

impl<'a> Voice<'a> {
    fn new(wave: &'a Wave, semis: f32, tune: f32, gain: f32, adsr: [u8; 4]) -> Self {
        // ponytail: attack is a squared ramp (127 = instant); NW4R uses a lookup table.
        let attack_secs = ((127 - adsr[0].min(127)) as f32 / 127.0).powi(2) * 2.0;
        Voice {
            wave,
            pos: 0.0,
            step: (2f32.powf(semis / 12.0) * tune * wave.rate / RATE as f32) as f64,
            gain,
            db: 0.0,
            attack: if attack_secs <= 0.0 { 1.0 } else { 1.0 / (attack_secs * RATE as f32) },
            level: 0.0,
            decay: rate_db(adsr[1]),
            sustain: 40.0 * (adsr[2].max(1) as f32 / 127.0).log10(),
            release: rate_db(adsr[3]),
            off_at: None,
            done: false,
            track: 0, pan: 0.0,
            base_step: (2f32.powf(semis / 12.0) * tune * wave.rate / RATE as f32) as f64,
        }
    }

    fn sample(&mut self, t: u64) -> f32 {
        let len = self.wave.data.len();
        if self.done || len == 0 {
            self.done = true;
            return 0.0;
        }
        if self.pos as usize >= len {
            match self.wave.loop_start {
                Some(ls) if ls < len => self.pos -= (len - ls) as f64,
                _ => {
                    self.done = true;
                    return 0.0;
                }
            }
        }
        let i = self.pos as usize;
        let f = (self.pos - i as f64) as f32;
        let next = match self.wave.data.get(i + 1) {
            Some(&n) => n,
            None => self.wave.loop_start.map_or(0.0, |ls| self.wave.data[ls.min(len - 1)]),
        };
        let s = self.wave.data[i] * (1.0 - f) + next * f;
        self.pos += self.step;
        if self.off_at.is_some_and(|o| t >= o) {
            self.db -= self.release;
        } else if self.level < 1.0 {
            self.level = (self.level + self.attack).min(1.0);
        } else if self.db > self.sustain {
            self.db = (self.db - self.decay).max(self.sustain);
        }
        if self.db < -90.0 {
            self.done = true;
        }
        s * self.gain * self.level * 10f32.powf(self.db / 20.0)
    }
}

fn vol_sq(v: u8) -> f32 { (v as f32 / 127.0).powi(2) }

fn wave_sound(rwsd: &[u8], sub: usize, waves: &[Wave], vol: f32) -> Option<Pcm> {
    let base = be32(rwsd, 0x10) as usize + 8;
    let r = |o: usize| base + be32(rwsd, o + 4) as usize;
    let wsd = r(base + 4 + 8 * sub);
    let notes = r(wsd + 0x10);
    if be32(rwsd, notes) == 0 {
        return None;
    }
    let note = r(notes + 4);
    let wave = waves.get(be32(rwsd, note) as usize)?;
    let mut v = Voice::new(wave, 0.0, bef(rwsd, note + 0x10) * bef(rwsd, r(wsd)), vol * vol_sq(rwsd[note + 0xD]), [127; 4]);
    if wave.loop_start.is_some() {
        // ponytail: looping waves are written as one pass from the start, the game loops the whole file.
        return Some(((0..wave.data.len() as u64).map(|t| v.sample(t)).collect(), 1, RATE, true));
    }
    let pcm: Vec<f32> = (0..).map_while(|t| (!v.done).then(|| v.sample(t))).collect();
    Some((pcm, 1, RATE, false))
}

#[derive(Clone)]
struct Track {
    pc: usize,
    wait: u32,
    active: bool,
    calls: Vec<usize>,
    loops: Vec<(usize, u8)>,
    vol: f32,
    vol2: f32,
    transpose: i32,
    bend: f32,
    bend_range: f32,
    pan: f32,
    prg: usize,
    note_wait: bool,
    adsr: [Option<u8>; 4],
    cmp: bool,
    vars: [i32; 16],
}

impl Track {
    fn new(pc: usize) -> Self {
        Track {
            pc,
            wait: 0,
            active: true,
            calls: vec![],
            loops: vec![],
            vol: 1.0,
            vol2: 1.0,
            transpose: 0,
            bend: 0.0,
            bend_range: 2.0,
            pan: 0.0,
            prg: 0,
            note_wait: true,
            adsr: [None; 4],
            cmp: false,
            vars: [0; 16],
        }
    }
}

enum Arg {
    U8,
    S8,
    S16,
    VarLen,
}

/// A note started this tick, waiting for its note-off time to be resolved.
struct NoteOn<'a> {
    voice: Voice<'a>,
    ticks: u32,
}

struct Seq<'a> {
    d: &'a [u8],
    bank: &'a [u8],
    waves: &'a [Wave],
    vars: [i32; 32],
    tempo: f32,
    timebase: f32,
    rng: u32,
    looped: bool,
    volume: f32,
}

impl<'a> Seq<'a> {
    fn u8(&self, t: &mut Track) -> u8 {
        t.pc += 1;
        *self.d.get(t.pc - 1).unwrap_or(&0xFF)
    }
    fn u16(&self, t: &mut Track) -> u16 { (self.u8(t) as u16) << 8 | self.u8(t) as u16 }
    fn u24(&self, t: &mut Track) -> usize { (self.u16(t) as usize) << 8 | self.u8(t) as usize }
    fn varlen(&self, t: &mut Track) -> i32 {
        let mut v = 0i32;
        for _ in 0..4 {
            let b = self.u8(t);
            v = v << 7 | (b & 0x7F) as i32;
            if b & 0x80 == 0 {
                break;
            }
        }
        v
    }
    fn random(&mut self, lo: i32, hi: i32) -> i32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        lo + (self.rng % ((hi - lo).max(0) as u32 + 1)) as i32
    }
    fn var<'t>(&'t mut self, t: &'t mut Track, i: u8) -> &'t mut i32 {
        match i {
            32..48 => &mut t.vars[i as usize - 32],
            // ponytail: game-provided global values start at zero; preserve a
            // separate namespace so they cannot overwrite player variables.
            _ => &mut self.vars[i as usize % 32],
        }
    }
    /// Reads the last argument of a command, honouring the random/variable prefixes.
    fn arg(&mut self, t: &mut Track, kind: Arg, prefix: u8) -> i32 {
        match prefix {
            0xA0 => {
                let lo = self.u16(t) as i16 as i32;
                let hi = self.u16(t) as i16 as i32;
                self.random(lo, hi)
            }
            0xA1 => {
                let i = self.u8(t);
                *self.var(t, i)
            }
            _ => match kind {
                Arg::U8 => self.u8(t) as i32,
                Arg::S8 => self.u8(t) as i8 as i32,
                Arg::S16 => self.u16(t) as i16 as i32,
                Arg::VarLen => self.varlen(t),
            },
        }
    }

    /// Finds the instrument parameters for a program/key/velocity.
    fn region(&self, prg: usize, key: i32, vel: i32) -> Option<usize> {
        let b = self.bank;
        let base = be32(b, 0x10) as usize + 8;
        if prg >= be32(b, base) as usize {
            return None;
        }
        let mut r = base + 4 + 8 * prg;
        for split in [key, vel, vel] {
            let (kind, o) = (b[r + 1], base + be32(b, r + 4) as usize);
            match kind {
                1 => return Some(o),
                2 => {
                    let n = b[o] as usize;
                    let i = (0..n).find(|&i| split <= b[o + 1 + i] as i32)?;
                    r = o + (n + 1).next_multiple_of(4) + 8 * i;
                }
                3 => {
                    let (lo, hi) = (b[o] as i32, b[o + 1] as i32);
                    if split < lo || split > hi {
                        return None;
                    }
                    r = o + 4 + 8 * (split - lo) as usize;
                }
                _ => return None,
            }
        }
        None
    }

    /// Runs one track until it waits.
    fn run(&mut self, ti: usize, tracks: &mut [Track], notes: &mut Vec<NoteOn<'a>>) {
        let mut t = tracks[ti].clone();
        for _ in 0..10_000 {
            if !t.active || t.wait > 0 {
                break;
            }
            let mut cmd = self.u8(&mut t);
            let mut exec = true;
            if cmd == 0xA2 {
                exec = t.cmp;
                cmd = self.u8(&mut t);
            }
            let mut prefix = 0;
            if (0xA0..=0xA5).contains(&cmd) {
                prefix = cmd;
                cmd = self.u8(&mut t);
            }
            match cmd {
                0x00..=0x7F => {
                    let vel = self.u8(&mut t) as i32;
                    let len = self.arg(&mut t, Arg::VarLen, prefix).max(0) as u32;
                    if exec {
                        if let Some(voice) = self.note(&t, cmd as i32, vel) {
                            let mut voice = voice; voice.track = ti;
                            notes.push(NoteOn { voice, ticks: len });
                        }
                        if t.note_wait {
                            t.wait = len;
                        }
                    }
                }
                0x80 => {
                    let w = self.arg(&mut t, Arg::VarLen, prefix);
                    if exec {
                        t.wait = w.max(0) as u32;
                    }
                }
                0x81 => {
                    let p = self.arg(&mut t, Arg::VarLen, prefix);
                    if exec {
                        t.prg = p.max(0) as usize;
                    }
                }
                0x88 => {
                    let (n, off) = (self.u8(&mut t) as usize, self.u24(&mut t));
                    if exec && n < tracks.len() && n != ti {
                        tracks[n] = Track::new(off);
                    }
                }
                0x89 => {
                    let off = self.u24(&mut t);
                    if exec {
                        self.looped |= off <= t.pc;
                        t.pc = off;
                    }
                }
                0x8A => {
                    let off = self.u24(&mut t);
                    if exec {
                        t.calls.push(t.pc);
                        t.pc = off;
                    }
                }
                0xB0..=0xDF => {
                    let v = self.arg(&mut t, if matches!(cmd, 0xC3 | 0xC4) { Arg::S8 } else { Arg::U8 }, prefix);
                    if exec {
                        match cmd {
                            0xB0 => self.timebase = v.max(1) as f32,
                            0xC0 => t.pan = (v as f32 - 64.0) / 63.0,
                            0xC1 => t.vol = v as f32 / 127.0,
                            0xC2 => self.volume = v as f32 / 127.0,
                            0xD5 => t.vol2 = v as f32 / 127.0,
                            0xC3 => t.transpose = v,
                            0xC4 => t.bend = v as f32 / 127.0,
                            0xC5 => t.bend_range = v as f32,
                            0xC7 => t.note_wait = v != 0,
                            0xD0..=0xD3 => t.adsr[(cmd - 0xD0) as usize] = Some(v as u8),
                            0xD4 => t.loops.push((t.pc, v as u8)),
                            _ => {}
                        }
                    }
                }
                0xE0..=0xEF => {
                    let v = self.arg(&mut t, Arg::S16, prefix);
                    if exec && cmd == 0xE1 {
                        self.tempo = v.max(1) as f32;
                    }
                }
                0xF0 => {
                    let sub = self.u8(&mut t);
                    if matches!(sub, 0x80..=0x8B | 0x90..=0x95) {
                        let i = self.u8(&mut t);
                        let v = self.arg(&mut t, Arg::S16, prefix);
                        if exec {
                            let r = if sub == 0x86 { self.random(0, v.abs()) * v.signum() } else { 0 };
                            let x = *self.var(&mut t, i);
                            let set = match sub {
                                0x80 => Some(v),
                                0x81 => Some(x + v),
                                0x82 => Some(x - v),
                                0x83 => Some(x * v),
                                0x84 => Some(if v != 0 { x / v } else { x }),
                                0x85 => Some(if v >= 0 { x << v.min(31) } else { x >> (-v).min(31) }),
                                0x86 => Some(r),
                                0x87 => Some(x & v),
                                0x88 => Some(x | v),
                                0x89 => Some(x ^ v),
                                0x8A => Some(!v),
                                0x8B => Some(if v != 0 { x % v } else { x }),
                                _ => {
                                    t.cmp = match sub {
                                        0x90 => x == v,
                                        0x91 => x >= v,
                                        0x92 => x > v,
                                        0x93 => x <= v,
                                        0x94 => x < v,
                                        _ => x != v,
                                    };
                                    None
                                }
                            };
                            if let Some(s) = set {
                                *self.var(&mut t, i) = s;
                            }
                        }
                    } else if sub == 0xE0 {
                        self.u16(&mut t);
                    }
                }
                0xFC if exec => match t.loops.pop() {
                    Some((start, 0)) => {
                        self.looped = true;
                        t.loops.push((start, 0));
                        t.pc = start;
                    }
                    Some((start, n)) if n > 1 => {
                        t.loops.push((start, n - 1));
                        t.pc = start;
                    }
                    _ => {}
                },
                0xFD if exec => match t.calls.pop() {
                    Some(pc) => t.pc = pc,
                    None => t.active = false,
                },
                0xFE => {
                    self.u16(&mut t);
                }
                0xFF if exec => t.active = false,
                _ => {}
            }
        }
        tracks[ti] = t;
    }

    fn note(&self, t: &Track, key: i32, vel: i32) -> Option<Voice<'a>> {
        let key = key + t.transpose;
        let p = self.region(t.prg, key, vel)?;
        let b = self.bank;
        let wave = self.waves.get(be32(b, p) as usize)?;
        let adsr = std::array::from_fn(|i| t.adsr[i].unwrap_or(b[p + 4 + i]));
        let semis = (key - b[p + 0xC] as i32) as f32;
        let gain = (vel as f32 / 127.0).powi(2) * vol_sq(b[p + 0xD]);
        let mut voice = Voice::new(wave, semis, bef(b, p + 0x10), gain, adsr);
        voice.pan = (b[p + 0xE] as f32 - 64.0) / 63.0;
        Some(voice)
    }
}

/// Renders the original track/instrument panning and live volume/pitch changes.
fn render_seq(rseq: &[u8], label: usize, rbnk: &[u8], waves: &[Wave], vol: f32, seed: u32) -> Pcm {
    let data = be32(rseq, 0x10) as usize;
    let start = data + be32(rseq, data + 8) as usize;
    let mut seq = Seq { d: &rseq[start..], bank: rbnk, waves, vars: [0; 32], tempo: 120.0, timebase: 48.0, rng: seed.wrapping_mul(2654435761) | 1, looped: false, volume: 1.0 };
    let mut tracks = vec![Track { active: false, ..Track::new(0) }; 16];
    tracks[0] = Track::new(label);
    let (mut voices, mut notes, mut out) = (Vec::<Voice>::new(), vec![], Vec::<f32>::new());
    let mut clock = 0f64;
    let limit = ((LOOP_SECS + FADE_SECS).max(MAX_SECS) * RATE as f32) as usize * 2;
    while out.len() < limit && (tracks.iter().any(|t| t.active) || !voices.is_empty()) {
        for ti in 0..tracks.len() {
            if tracks[ti].active {
                seq.run(ti, &mut tracks, &mut notes);
            }
        }
        let tick = 60.0 * RATE as f64 / (seq.tempo as f64 * seq.timebase as f64);
        for NoteOn { mut voice, ticks } in notes.drain(..) {
            voice.off_at = (ticks > 0).then(|| (clock + ticks as f64 * tick) as u64);
            voices.push(voice);
        }
        for t in tracks.iter_mut() {
            t.wait = t.wait.saturating_sub(1);
        }
        clock += tick;
        while (out.len() as f64 / 2.0) < clock && out.len() < limit {
            let now = out.len() as u64 / 2;
            let mut sample = [0.0; 2];
            for v in &mut voices {
                let t = &tracks[v.track];
                v.step = v.base_step * 2f64.powf((t.bend * t.bend_range) as f64 / 12.0);
                let s = v.sample(now) * t.vol * t.vol2 * seq.volume;
                let pan = stereo_pan(v.pan + t.pan);
                sample[0] += s * pan[0]; sample[1] += s * pan[1];
            }
            out.extend(sample.map(|s| s * vol));
        }
        voices.retain(|v| !v.done);
    }
    let loops = seq.looped || out.len() >= limit;
    if loops {
        // Crossfade the rendered tail into the start so the file loops seamlessly.
        let (l, f) = ((LOOP_SECS * RATE as f32) as usize * 2, (FADE_SECS * RATE as f32) as usize * 2);
        out.resize(l + f, 0.0);
        for i in 0..f {
            let w = i as f32 / f as f32;
            out[i] = out[i] * w + out[l + i] * (1.0 - w);
        }
        out.truncate(l);
    }
    (out, 2, RATE, loops)
}

fn stereo_pan(pan: f32) -> [f32; 2] {
    let angle = (pan.clamp(-1.0, 1.0) + 1.0) * std::f32::consts::FRAC_PI_4;
    [angle.cos(), angle.sin()]
}

/// Decodes a BRSTM (ADPCM or PCM16, first two channels).
fn brstm(d: &[u8]) -> Result<Pcm> {
    anyhow::ensure!(&d[..4] == b"RSTM", "bad BRSTM");
    let head = be32(d, 0x10) as usize + 8;
    let r = |o: usize| head + be32(d, o + 4) as usize;
    let si = r(head);
    let (format, looped, nch) = (d[si], d[si + 1] != 0, d[si + 2] as usize);
    let rate = be16(d, si + 4) as u32;
    let (loop_start, total) = (be32(d, si + 8) as usize, be32(d, si + 0xC) as usize);
    let data = be32(d, si + 0x10) as usize;
    let (blocks, block_size, block_samples) = (be32(d, si + 0x14) as usize, be32(d, si + 0x18) as usize, be32(d, si + 0x1C) as usize);
    let last_padded = be32(d, si + 0x28) as usize;
    let chans = r(head + 16);
    let used = nch.min(2);
    let mut dec: Vec<Adpcm> = (0..used).map(|c| Adpcm::new(&d[r(r(chans + 4 + 8 * c))..])).collect();
    let mut pcm: Vec<Vec<f32>> = vec![Vec::with_capacity(total); used];
    let mut o = data;
    for b in 0..blocks {
        let size = if b + 1 == blocks { last_padded } else { block_size };
        for (c, out) in pcm.iter_mut().enumerate() {
            let src = &d[o + c * size..][..size];
            let want = (total - out.len()).min(block_samples);
            match format {
                2 => out.extend(dec[c].decode(src, want)),
                1 => out.extend((0..want).map(|i| be16(src, i * 2) as i16 as f32 / 32768.0)),
                _ => out.extend(src[..want].iter().map(|&s| s as i8 as f32 / 128.0)),
            }
        }
        o += size * nch;
    }
    // ponytail: looping streams keep only their loop body (the intro is dropped), since the game loops whole files.
    let from = if looped { loop_start } else { 0 };
    let len = pcm.iter().map(Vec::len).min().unwrap_or(0);
    let pcm = if used == 1 { pcm[0][from..len].to_vec() } else { (from..len).flat_map(|i| [pcm[0][i], pcm[1][i]]).collect() };
    Ok((pcm, used as u16, rate, looped))
}

fn wav(pcm: &[f32], channels: u16, rate: u32) -> Vec<u8> {
    let bytes = pcm.len() as u32 * 2;
    let mut w = Vec::with_capacity(44 + bytes as usize);
    for part in [&b"RIFF"[..], &(36 + bytes).to_le_bytes(), b"WAVEfmt ", &16u32.to_le_bytes(), &1u16.to_le_bytes(), &channels.to_le_bytes()] {
        w.extend_from_slice(part);
    }
    for part in [&rate.to_le_bytes()[..], &(rate * channels as u32 * 2).to_le_bytes(), &(channels * 2).to_le_bytes(), &16u16.to_le_bytes(), b"data", &bytes.to_le_bytes()] {
        w.extend_from_slice(part);
    }
    for s in pcm {
        w.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    w
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sequence_pan_has_equal_power_and_hard_left_right_endpoints() {
        assert_eq!(stereo_pan(-1.0),[1.0,0.0]);
        assert!(stereo_pan(1.0)[0].abs()<1e-6);
        for pan in [-1.0,-0.5,0.0,0.5,1.0] { let [l,r]=stereo_pan(pan); assert!((l*l+r*r-1.0).abs()<1e-6); }
    }
    #[test]
    fn adpcm_silence_and_wav_header() {
        // Zero frames decode to silence; the WAV header carries the stated format.
        let s = Adpcm::new(&[0; 32]).decode(&[0; 16], 28);
        assert_eq!(s, vec![0.0; 28]);
        let w = wav(&[0.5, -0.5], 2, 44100);
        assert_eq!(&w[..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(w[24..28].try_into().unwrap()), 44100);
        assert_eq!(w.len(), 44 + 4);
    }
}
