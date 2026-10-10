//! Convert the flight scene's REFF particle emitters and REFT textures from the
//! disc into JSON and PNGs for the game's particle player (`game/src/particles.rs`).
//! Container layouts follow BrawlCrate's REFF/REFT definitions:
//! https://github.com/soopercool101/BrawlCrate/tree/master/BrawlLib/SSBB/Types
//! Emitter fields follow NW4R's public `EmitterDesc` layout; this game's version
//! has four extra bytes before the TEV stages and a separate alpha input block.
use crate::disc::{Disc, be16, be32, bef, cstr};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{collections::{BTreeMap, BTreeSet}, fs, path::Path};

fn entries<'a>(d: &'a [u8], magic: &[u8]) -> Result<BTreeMap<String, &'a [u8]>> {
    ensure!(d.len() >= 40 && &d[..4] == magic, "invalid flight effect header");
    let table = 24 + be32(d, 24) as usize;
    ensure!(table + 8 <= d.len(), "invalid effect table");
    let mut cursor = table + 8;
    let mut out = BTreeMap::new();
    for _ in 0..be16(d, table + 4) {
        ensure!(cursor + 2 <= d.len(), "truncated effect name");
        let n = be16(d, cursor) as usize;
        ensure!(n > 0 && cursor + n + 10 <= d.len(), "truncated effect entry");
        let name = cstr(&d[..cursor + 2 + n], cursor + 2);
        let start = table + be32(d, cursor + 2 + n) as usize;
        // REFT lengths omit the 64-byte image header (32 bytes of fields, 32 of padding).
        let size = be32(d, cursor + 6 + n) as usize + if magic == b"REFT" {REFT_HEADER} else {0};
        out.insert(name, d.get(start..start + size).context("effect outside file")?);
        cursor += n + 10;
    }
    Ok(out)
}

const REFT_HEADER: usize = 64;

fn floats(d: &[u8], o: usize, n: usize) -> Vec<f32> { (0..n).map(|i| bef(d, o + 4 * i)).collect() }
fn bytes(d: &[u8], o: usize, n: usize) -> &[u8] { &d[o..o + n] }
fn i8s(d: &[u8], o: usize) -> i8 { d[o] as i8 }

/// Length-prefixed names (u16 length including the terminator).
fn names(d: &[u8], mut o: usize, count: usize) -> Vec<String> {
    (0..count).map(|_| { let n = be16(d, o) as usize; let s = cstr(&d[..o + 2 + n], o + 2); o += 2 + n; s }).collect()
}

/// One animation curve (NW4R `AnimCurveHeader` plus its key, range, random, name and info tables).
fn curve(d: &[u8]) -> Result<Value> {
    ensure!(d.len() >= 0x20 && matches!(d[0], 0xAC | 0xAB), "invalid particle curve");
    let (kind, ty, enable) = (d[1], d[2], d[3]);
    let [keys, ranges, random, name_table, info] = std::array::from_fn(|i| be32(d, 0x0C + 4 * i) as usize);
    let body = &d[0x20..];
    ensure!(keys + ranges + random + name_table + info <= body.len(), "truncated particle curve");
    let n = enable.count_ones() as usize;
    // Baked curves (0xAB) store one value set per frame in the key table.
    if d[0] == 0xAB {
        // Other baked kinds (texture/child) are not needed by the player.
        if !matches!(ty, 0 | 3 | 6) { return Ok(json!({ "kind": kind, "ty": ty, "enable": enable, "proc": d[4], "loops": d[5], "seed": be16(d, 6), "frames": be16(d, 8) })); }
        let stride = if ty == 0 { n } else { 4 * n };
        let frames = (be16(d, 8) as usize).min(keys / stride.max(1));
        let baked: Vec<Vec<f32>> = (0..frames).map(|f| (0..n).map(|c| if ty == 0 { body[f * n + c] as f32 } else { bef(body, f * stride + 4 * c) }).collect()).collect();
        return Ok(json!({ "kind": kind, "ty": ty, "enable": enable, "proc": d[4], "loops": d[5], "seed": be16(d, 6), "frames": be16(d, 8), "baked": baked }));
    }
    let decoded = matches!(ty, 0 | 3 | 5 | 6 | 7);
    let range_size = match ty { 0 => 2 * n, 6 => 8 * n + 4, 5 => 12, _ => 8 * n };
    let table = |start: usize, size: usize| -> Vec<Value> {
        if size < 4 || !decoded || range_size == 0 { return vec![]; }
        let count = (be16(body, start) as usize).min((size - 4) / range_size);
        (0..count).map(|i| {
            let e = start + 4 + i * range_size;
            match ty {
                // U8 entries pack (base, span) as bytes.
                0 => json!({ "v": (0..n).map(|c| [body[e + 2 * c] as f32, body[e + 2 * c + 1] as f32]).collect::<Vec<_>>(), "sign": false }),
                // Spans are pre-divided by 65536 for a 16-bit random value; rotation adds a random-sign byte.
                3 | 6 | 7 => json!({ "v": (0..n).map(|c| [bef(body, e + 8 * c), bef(body, e + 8 * c + 4)]).collect::<Vec<_>>(), "sign": ty == 6 && body[e + 8 * n] != 0 }),
                _ => json!(null),
            }
        }).collect()
    };
    let key_size = match ty { 0 => (n + 13) & !1, 5 => 24, _ => 12 + 4 * n };
    // Texture (4), post-field (2) and emitter (11) keys are not used by the player.
    let key_count = if keys >= 4 && decoded { be16(body, 0) as usize } else { 0 };
    ensure!(4 + key_count * key_size <= keys.max(4), "truncated curve keys");
    let key_list: Vec<Value> = if key_count > 0 { (0..key_count).map(|i| {
        let k = 4 + i * key_size;
        let (frame, mode) = (be16(body, k), body[k + 2]);
        let interp = bytes(body, k + 4, 4);
        match ty {
            5 => json!({ "frame": frame, "mode": mode,
                "speed": be16(body, k + 12) as i16, "scale": body[k + 14], "alpha": body[k + 15], "color": body[k + 16],
                "weight": body[k + 17], "kind": body[k + 18], "flag": body[k + 19], "name": be16(body, k + 22) }),
            _ if mode != 0 => json!({ "frame": frame, "mode": mode, "interp": interp, "index": be16(body, k + 12) }),
            0 => json!({ "frame": frame, "mode": 0, "interp": interp, "values": bytes(body, k + 12, n) }),
            _ => json!({ "frame": frame, "mode": 0, "interp": interp, "values": floats(body, k + 12, n) }),
        }
    }).collect() } else { vec![] };
    let names_at = keys + ranges + random;
    let name_list = if name_table >= 4 { let c = be16(body, names_at) as usize; names(body, names_at + 4 + 4 * c, c) } else { vec![] };
    let i = names_at + name_table;
    // Fields: space, target (velocity/position), flags, power, then per-field parameters.
    let field = (ty == 7 && info >= 8).then(|| json!({ "space": body[i], "target": body[i + 1], "flags": body[i + 3], "power": bef(body, i + 4),
        "params": floats(body, i + 8, (info - 8).min(20) / 4) }));
    Ok(json!({ "kind": kind, "ty": ty, "enable": enable, "proc": d[4], "loops": d[5], "seed": be16(d, 6), "frames": be16(d, 8),
        "keys": key_list, "ranges": table(keys, ranges), "random": table(keys + ranges, random), "names": name_list, "field": field }))
}

/// One emitter: `EmitterDesc`, its draw setting, particle parameters and curves.
fn emitter(e: &[u8], textures: &mut BTreeSet<String>) -> Result<Value> {
    ensure!(e.len() >= 0x150, "truncated emitter");
    let ed = 8;
    let ds = ed + 0x94;
    let pp = be32(e, 4) as usize + 8;
    let p = pp + 4;
    ensure!(p + 136 <= e.len(), "truncated particle parameters");
    let tex: Vec<String> = names(e, p + 136, 3);
    textures.extend(tex.iter().filter(|t| !t.is_empty()).cloned());
    let draw = json!({
        "flags": be16(e, ds), "acmp": bytes(e, ds + 2, 3), "tevs": e[ds + 5], "tev_tex": bytes(e, ds + 8, 4),
        "tev_color": (0..4).map(|i| bytes(e, ds + 0x0C + 4 * i, 4)).collect::<Vec<_>>(),
        "color_op": (0..4).map(|i| bytes(e, ds + 0x1C + 5 * i, 5)).collect::<Vec<_>>(),
        "tev_alpha": (0..4).map(|i| bytes(e, ds + 0x30 + 4 * i, 4)).collect::<Vec<_>>(),
        "alpha_op": (0..4).map(|i| bytes(e, ds + 0x40 + 5 * i, 5)).collect::<Vec<_>>(),
        "kcsel": bytes(e, ds + 0x54, 4), "kasel": bytes(e, ds + 0x58, 4), "blend": bytes(e, ds + 0x5C, 4),
        "color_in": bytes(e, ds + 0x60, 8), "alpha_in": bytes(e, ds + 0x68, 8), "zcmp": e[ds + 0x70],
        "flick": [e[ds + 0x71] as u16, be16(e, ds + 0x72), e[ds + 0x74] as u16, e[ds + 0x75] as u16],
        "pivot": [i8s(e, ds + 0xA9), i8s(e, ds + 0xAA)], "ptype": bytes(e, ds + 0xAC, 7), "zoff": bef(e, ds + 0xB4),
    });
    let ptcl = json!({
        "colors": (0..4).map(|i| bytes(e, p + 4 * i, 4)).collect::<Vec<_>>(),
        "size": floats(e, p + 16, 2), "scale": floats(e, p + 24, 2), "rotate": floats(e, p + 32, 3),
        "tex_scale": (0..3).map(|i| floats(e, p + 44 + 8 * i, 2)).collect::<Vec<_>>(), "tex_rot": floats(e, p + 68, 3),
        "tex_trans": (0..3).map(|i| floats(e, p + 80 + 8 * i, 2)).collect::<Vec<_>>(),
        "wrap": be16(e, p + 116), "reverse": e[p + 118], "acmp_ref": bytes(e, p + 119, 2),
        "rot_offset_rnd": bytes(e, p + 121, 3), "rot_offset": floats(e, p + 124, 3), "textures": tex,
    });
    // Track tables: particle tracks then emitter tracks, each a count, an
    // init-only count, offsets and sizes; curves follow back to back.
    let t = pp + be32(e, pp) as usize + 4;
    let (n, ni) = (be16(e, t) as usize, be16(e, t + 2));
    let t2 = t + 4 + 8 * n;
    let (m, mi) = (be16(e, t2) as usize, be16(e, t2 + 2));
    let mut at = t2 + 4 + 8 * m;
    let mut tracks = |count: usize, sizes: usize| -> Result<Vec<Value>> {
        (0..count).map(|i| {
            let size = be32(e, sizes + 4 * i) as usize;
            let c = curve(e.get(at..at + size).context("curve outside emitter")?);
            at += size;
            c
        }).collect()
    };
    let ptcl_tracks = tracks(n, t + 4 + 4 * n)?;
    let emit_tracks = tracks(m, t2 + 4 + 4 * m)?;
    Ok(json!({
        "common": be32(e, ed), "emit_flag": be32(e, ed + 4), "emit_life": be16(e, ed + 8), "ptcl_life": be16(e, ed + 0x0A),
        "life_rnd": i8s(e, ed + 0x0C), "inherit_child_ptcl_t": i8s(e, ed + 0x0D), "interval_rnd": i8s(e, ed + 0x0E), "emit_rnd": i8s(e, ed + 0x0F),
        "emit": bef(e, ed + 0x10), "emit_start": be16(e, ed + 0x14), "emit_past": be16(e, ed + 0x16), "interval": be16(e, ed + 0x18),
        "inherit_ptcl_t": i8s(e, ed + 0x1A), "inherit_child_emit_t": i8s(e, ed + 0x1B), "params": floats(e, ed + 0x1C, 6),
        "emit_div": be16(e, ed + 0x34), "vel_init_rnd": i8s(e, ed + 0x36), "vel_momentum_rnd": i8s(e, ed + 0x37),
        "vel": floats(e, ed + 0x38, 7), "spec_dir": floats(e, ed + 0x54, 3),
        "scale": floats(e, ed + 0x60, 3), "rotate": floats(e, ed + 0x6C, 3), "translate": floats(e, ed + 0x78, 3), "seed": be32(e, ed + 0x88),
        "draw": draw, "ptcl": ptcl, "ptcl_init": ni, "ptcl_tracks": ptcl_tracks, "emit_init": mi, "emit_tracks": emit_tracks,
    }))
}

/// Texture file name; `#` would start an asset label in Bevy paths.
fn file_name(texture: &str) -> String { texture.replace('#', "~") }

pub fn extract(disc: &mut Disc, out: &Path) -> Result<()> {
    let archive = disc.archive("Effect/PlnScene/effect.carc")?;
    let reff = archive.get("WS2_effect_pln.breff").context("flight REFF missing")?;
    let reft = archive.get("WS2_effect_pln.breft").context("flight REFT missing")?;
    let mut used = BTreeSet::new();
    let mut emitters = BTreeMap::new();
    for (name, d) in entries(reff, b"REFF")? {
        emitters.insert(name.clone(), emitter(d, &mut used).with_context(|| name)?);
    }
    let textures = entries(reft, b"REFT")?;
    for name in &used {
        let image = textures.get(name).with_context(|| name.clone())?;
        let (w, h, bytes) = (be16(image, 4) as usize, be16(image, 6) as usize, be32(image, 8) as usize);
        let pixels = image.get(REFT_HEADER..REFT_HEADER + bytes).context("truncated particle pixels")?;
        let palette: Vec<_> = image.get(REFT_HEADER + bytes..).unwrap_or_default().chunks_exact(2).take(be16(image, 14) as usize)
            .map(|c| crate::tex::palette_color(image[13] as u32, u16::from_be_bytes([c[0], c[1]]))).collect();
        fs::write(out.join(format!("effects/{}.png", file_name(name))), crate::tex::decode(pixels, image[12] as u32, w, h, Some(&palette))?.png()?)?;
    }
    fs::write(out.join("effects/pln.json"), serde_json::to_vec(&emitters)?)?;
    println!("  {} flight particle emitters, {} textures", emitters.len(), used.len());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn invalid_or_truncated_effect_tables_are_rejected() {
        assert!(entries(b"REFT", b"REFT").is_err());
        let mut d = vec![0; 40]; d[..4].copy_from_slice(b"REFT");
        d[24..28].copy_from_slice(&1000u32.to_be_bytes());
        assert!(entries(&d, b"REFT").is_err());
    }
    #[test] fn curve_with_inline_float_keys_decodes() {
        // F32 curve, one component, one key at frame 3 with value 2.5.
        let mut d = vec![0u8; 0x20];
        d[0] = 0xAC; d[1] = 16; d[2] = 3; d[3] = 1; d[8..10].copy_from_slice(&4u16.to_be_bytes());
        d[0x0C..0x10].copy_from_slice(&20u32.to_be_bytes());
        let mut keys = vec![0u8; 20];
        keys[1] = 1; keys[5] = 3; keys[16..20].copy_from_slice(&2.5f32.to_be_bytes());
        d.extend(keys);
        let c = curve(&d).unwrap();
        assert_eq!(c["keys"][0]["frame"], 3);
        assert_eq!(c["keys"][0]["values"][0], 2.5);
    }
}
