//! Optional Pilotwings Resort (3DS) extraction: vehicles, i-points, thermals
//! and texts that the game uses when `assets/pwr` exists. Requires a decrypted
//! (NoCrypto) cartridge dump made by the user. Nothing produced here may be
//! committed.
//!
//! Layers, outermost first: NCSD/NCCH → RomFS files → "0TSR" containers whose
//! payload may be QuickLZ (level 3) compressed → resources listed in the
//! `tocres.res` directory → typed resources (VALT key trees, LANG strings,
//! PICA200 textures, "0rtc" meshes).

use crate::gltf::Gltf;
use crate::tex::Image;
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

fn u16le(d: &[u8], o: usize) -> u16 { u16::from_le_bytes([d[o], d[o + 1]]) }
fn u32le(d: &[u8], o: usize) -> u32 { u32::from_le_bytes(d[o..o + 4].try_into().unwrap()) }
fn f32le(d: &[u8], o: usize) -> f32 { f32::from_bits(u32le(d, o)) }
fn cstr(d: &[u8], o: usize) -> &[u8] { let e = d[o..].iter().position(|&c| c == 0).map_or(d.len(), |n| o + n); &d[o..e] }

pub fn is_3ds(path: &Path) -> bool {
    let mut magic = [0u8; 4];
    fs::File::open(path).and_then(|mut f| { use std::io::{Read, Seek}; f.seek(std::io::SeekFrom::Start(0x100))?; f.read_exact(&mut magic) }).is_ok() && &magic == b"NCSD"
}

/// The first NCCH partition's ExeFS `.code` (decompressed) and RomFS files.
struct Rom {
    code: Vec<u8>,
    files: HashMap<String, Vec<u8>>,
}

impl Rom {
    fn open(path: &Path) -> Result<Self> {
        let d = fs::read(path).with_context(|| path.display().to_string())?;
        ensure!(&d[0x100..0x104] == b"NCSD", "not a 3DS cartridge image");
        let ncch = u32le(&d, 0x120) as usize * 0x200;
        let n = &d[ncch..];
        ensure!(&n[0x100..0x104] == b"NCCH", "partition 0 is not NCCH");
        let product = String::from_utf8_lossy(cstr(n, 0x150)).into_owned();
        ensure!(product.starts_with("CTR-P-AWA"), "{} is {product}, expected Pilotwings Resort (CTR-P-AWAx)", path.display());
        // Encrypted dumps would need console keys; only plain dumps are supported.
        ensure!(n[0x18F] & 4 != 0, "this dump is encrypted; please provide a decrypted (NoCrypto) dump");
        let exefs = &n[u32le(n, 0x1A0) as usize * 0x200..];
        let code = (0..10)
            .map(|i| i * 16)
            .find(|&e| cstr(exefs, e) == b".code")
            .map(|e| &exefs[0x200 + u32le(exefs, e + 8) as usize..][..u32le(exefs, e + 12) as usize])
            .context("ExeFS has no .code")?;
        let code = if n[0x18D] & 1 != 0 { blz(code) } else { code.to_vec() };
        let romfs = &n[u32le(n, 0x1B0) as usize * 0x200..];
        ensure!(&romfs[..4] == b"IVFC", "RomFS has no IVFC header");
        let block = 1usize << u32le(romfs, 0x4C);
        let lvl3 = &romfs[(0x60 + u32le(romfs, 8) as usize).div_ceil(block) * block..];
        let (dirs, files_meta, data) = (&lvl3[u32le(lvl3, 0x0C) as usize..], &lvl3[u32le(lvl3, 0x1C) as usize..], u32le(lvl3, 0x24) as usize);
        let mut files = HashMap::new();
        let utf16 = |d: &[u8], o: usize, n: usize| String::from_utf16_lossy(&(0..n / 2).map(|i| u16le(d, o + i * 2)).collect::<Vec<_>>());
        let mut stack = vec![(0usize, String::new())];
        while let Some((dir, path)) = stack.pop() {
            let mut f = u32le(dirs, dir + 12);
            while f != u32::MAX {
                let fo = f as usize;
                let (off, size) = (u32le(files_meta, fo + 8) as usize, u32le(files_meta, fo + 16) as usize);
                let name = utf16(files_meta, fo + 32, u32le(files_meta, fo + 28) as usize);
                files.insert(format!("{path}/{name}").trim_start_matches('/').to_ascii_lowercase(), lvl3[data + off..data + off + size].to_vec());
                f = u32le(files_meta, fo + 4);
            }
            let mut c = u32le(dirs, dir + 8);
            while c != u32::MAX {
                let co = c as usize;
                stack.push((co, format!("{path}/{}", utf16(dirs, co + 24, u32le(dirs, co + 20) as usize))));
                c = u32le(dirs, co + 4);
            }
        }
        Ok(Self { code, files })
    }
}

/// 3DS "backward LZ" used for ExeFS code: decoded from the end towards the start.
fn blz(d: &[u8]) -> Vec<u8> {
    let (enc, inc) = (u32le(d, d.len() - 8), u32le(d, d.len() - 4) as usize);
    let (hdr, comp) = ((enc >> 24) as usize, (enc & 0xFF_FFFF) as usize);
    let mut out = d.to_vec();
    out.resize(d.len() + inc, 0);
    let (mut src, mut dst, stop) = (d.len() - hdr, out.len(), d.len() - comp);
    while src > stop {
        src -= 1;
        let mut flags = d[src];
        for _ in 0..8 {
            if src <= stop { break }
            if flags & 0x80 != 0 {
                src -= 2;
                let v = u16le(d, src) as usize;
                let (n, disp) = ((v >> 12) + 3, (v & 0xFFF) + 3);
                for _ in 0..n { dst -= 1; out[dst] = out[dst + disp]; }
            } else {
                src -= 1;
                dst -= 1;
                out[dst] = d[src];
            }
            flags <<= 1;
        }
    }
    out
}

/// QuickLZ level-3 stream with its own 3- or 9-byte header.
fn quicklz3(src: &[u8]) -> Result<Vec<u8>> {
    const BITS: [usize; 16] = [4, 0, 1, 0, 2, 0, 1, 0, 3, 0, 1, 0, 2, 0, 1, 0];
    let long = src[0] & 2 != 0;
    let (hdr, size) = if long { (9, u32le(src, 5) as usize) } else { (3, src[2] as usize) };
    if src[0] & 1 == 0 { return Ok(src[hdr..hdr + size].to_vec()) }
    let at = |p: usize| -> u32 { (0..4).map(|k| (*src.get(p + k).unwrap_or(&0) as u32) << (8 * k)).sum() };
    let mut out = Vec::with_capacity(size);
    let (mut p, mut cw) = (hdr, 1u32);
    loop {
        if cw == 1 { cw = at(p); p += 4; }
        if cw & 1 != 0 {
            cw >>= 1;
            let f = at(p);
            let (off, n, len) = if f & 3 == 0 { ((f & 0xFF) >> 2, 3, 1) }
                else if f & 2 == 0 { ((f & 0xFFFF) >> 2, 3, 2) }
                else if f & 1 == 0 { ((f & 0xFFFF) >> 6, ((f >> 2) & 15) + 3, 2) }
                else if f & 0x7F != 3 { ((f & 0xFF_FFFF) >> 7, ((f >> 2) & 0x1F) + 2, 3) }
                else { (f >> 15, ((f >> 7) & 0xFF) + 3, 4) };
            p += len;
            let start = out.len().checked_sub(off as usize).context("QuickLZ back-reference before start")?;
            for k in 0..n as usize { out.push(out[start + k]); }
        } else if out.len() >= size.max(10) - 10 {
            while out.len() < size {
                if cw == 1 { p += 4; cw = 0x8000_0000; }
                out.push(src[p]);
                p += 1;
                cw >>= 1;
            }
            return Ok(out);
        } else {
            let n = BITS[(cw & 15) as usize];
            out.extend_from_slice(&src[p..p + n]);
            cw >>= n;
            p += n;
        }
    }
}

/// Payload of a "0TSR" container (.car/.trk/.res).
fn container(d: &[u8]) -> Result<Vec<u8>> {
    ensure!(&d[..4] == b"0TSR", "not a 0TSR container");
    let (kind, off, size) = (u32le(d, 0x0C), u32le(d, 0x18) as usize, u32le(d, 0x24) as usize);
    if kind == 1 { return Ok(d[off..off + size].to_vec()) }
    ensure!(&d[off..off + 4] == b"PMCr", "compressed chunk missing");
    let out = quicklz3(&d[off + 0x10..])?;
    ensure!(out.len() == size, "container size mismatch");
    Ok(out)
}

/// A resource inside a container payload.
struct Res { name: String, kind: String, off: usize, size: usize }

/// `tocres.res`: per-container resource directories ("COTE" records).
fn tocs(d: &[u8]) -> HashMap<String, Vec<Res>> {
    let mut n = 0;
    while d.get(n * 0x28 + 4..n * 0x28 + 8) == Some(b"COTE") { n += 1; }
    let names = n * 0x28;
    let s = |o: usize| String::from_utf8_lossy(cstr(d, o)).to_ascii_lowercase();
    (0..n).map(|i| {
        let (r, b) = (i * 0x28, u32le(d, i * 0x28 + 0x10) as usize);
        let cnt = u32le(d, b + 0xC) as usize;
        let sbase = b + 0x20 + cnt * 0x28;
        let list = (0..cnt).map(|k| {
            let e = b + 0x20 + k * 0x28;
            let mut kind = d[e + 4..e + 8].to_vec();
            kind.reverse();
            Res { name: s(sbase + u32le(d, e) as usize), kind: String::from_utf8_lossy(&kind).trim().to_string(), off: u32le(d, e + 0x10) as usize, size: u32le(d, e + 0xC) as usize }
        }).collect();
        (s(names + u32le(d, r) as usize).trim_end_matches(".toc").to_string(), list)
    }).collect()
}

/// VALT key hash: case-folded base-37 polynomial, low 28 bits.
pub fn key_hash(name: &str) -> u32 {
    name.bytes().fold(0u32, |h, c| h.wrapping_mul(37).wrapping_add((c | 0x20) as u32)) & 0x0FFF_FFFF
}

/// Hash → name from every identifier-like run in the given blobs.
fn dictionary<'a>(blobs: impl Iterator<Item = &'a [u8]>) -> HashMap<u32, String> {
    let mut names = HashMap::new();
    for b in blobs {
        let mut i = 0;
        while i < b.len() {
            if b[i].is_ascii_alphabetic() || b[i] == b'_' {
                let s = i;
                while i < b.len() && i - s < 64 && (b[i].is_ascii_alphanumeric() || b[i] == b'_' || b[i] == b'.') { i += 1; }
                if i - s >= 2 { let w = String::from_utf8_lossy(&b[s..i]).into_owned(); names.entry(key_hash(&w)).or_insert(w); }
            }
            i += 1;
        }
    }
    let base: Vec<String> = names.values().cloned().chain(["bound", "p", "pt", "v", "c"].map(String::from)).collect();
    for s in base { for d in 0..10 { let w = format!("{s}{d}"); names.entry(key_hash(&w)).or_insert(w); } }
    for c in 'a'..='z' { names.entry(key_hash(&c.to_string())).or_insert(c.to_string()); }
    names.entry(key_hash("__ver")).or_insert("__ver".into());
    names
}

/// VALT v19 key tree → JSON. Child nodes become arrays under their key; each
/// node object carries its name in `_name`. Unknown keys are `#hash`.
fn valt(b: &[u8], names: &HashMap<u32, String>) -> Result<Value> {
    ensure!(&b[..4] == b"!ZAB", "not a VALT section");
    let n_nodes = u32le(b, 4) as usize * 4 / 24;
    let vsec = 8 + n_nodes * 24;
    let n_vals = ((u32le(b, vsec + 4) & 0x3FFF_FFFF) >> 1) as usize;
    let vals = vsec + 8;
    let asec = vals + n_vals * 8;
    let arr = asec + 8;
    let strs = arr + u32le(b, asec + 4) as usize * 4 + 8;
    let string = |o: u32| if o == 0 { Value::Null } else { Value::String(String::from_utf8_lossy(cstr(b, strs + o as usize - 4)).into_owned()) };
    let floats = |o: u32, n: usize| Value::from((0..n).map(|k| f32le(b, arr + o as usize - 4 + 4 * k) as f64).collect::<Vec<_>>());
    let mut referenced = vec![false; n_nodes];
    for i in 0..n_vals {
        if u32le(b, vals + i * 8) & 15 == 8 {
            let o = u32le(b, vals + i * 8 + 4);
            if o != 0 { referenced[(o as usize - 4) / 24] = true; }
        }
    }
    fn node(i: usize, depth: usize, b: &[u8], vals: usize, names: &HashMap<u32, String>, val: &dyn Fn(u32, u32) -> Value) -> Value {
        let o = 8 + i * 24;
        let mut m = Map::new();
        let name = u32le(b, o + 4);
        let (voff, count) = (u32le(b, o + 16) as usize, u32le(b, o + 20) as usize);
        m.insert("_name".into(), val(4, name));
        if voff != 0 {
            for k in 0..count {
                let e = vals + ((voff - 4) / 8 + k) * 8;
                let (key, data) = (u32le(b, e), u32le(b, e + 4));
                let kname = names.get(&(key >> 4)).cloned().unwrap_or_else(|| format!("#{:07x}", key >> 4));
                let v = if key & 15 == 8 {
                    if data == 0 || depth > 32 { Value::Null } else { node((data as usize - 4) / 24, depth + 1, b, vals, names, val) }
                } else { val(key & 15, data) };
                match m.get_mut(&kname) {
                    Some(Value::Array(a)) if key & 15 == 8 => a.push(v),
                    Some(old) => { let prev = old.take(); *old = Value::Array(vec![prev, v]); }
                    None => { m.insert(kname, if key & 15 == 8 { Value::Array(vec![v]) } else { v }); }
                }
            }
        }
        Value::Object(m)
    }
    let val = |t: u32, d: u32| -> Value {
        match t {
            0 => json!(f32::from_bits(d)),
            1 => json!(d as i32),
            2 => json!(d != 0),
            3 => json!(format!("#{d:08x}")),
            4 => string(d),
            5 => if d == 0 { Value::Null } else { floats(d, 3) },
            6 => if d == 0 { Value::Null } else { floats(d, 2) },
            7 => if d == 0 { Value::Null } else { floats(d, 12) },
            _ => json!(d),
        }
    };
    let roots: Vec<Value> = (0..n_nodes).filter(|&i| !referenced[i]).map(|i| node(i, 0, b, vals, names, &val)).collect();
    Ok(if roots.len() == 1 { roots.into_iter().next().unwrap() } else { Value::Array(roots) })
}

/// Font glyph order used by the LANG strings: Latin-1 À..ÿ without Ð × Þ ð ÷ þ
/// starting at U+0080, then Œ œ; a few punctuation marks sit at fixed points.
fn glyph(c: char) -> Option<char> {
    let latin = (0xC0u32..0x100).filter(|c| ![0xD0, 0xD7, 0xDE, 0xF0, 0xF7, 0xFE].contains(c)).filter_map(char::from_u32).chain(['Œ', 'œ']);
    match c as u32 {
        0x80..=0xBB => latin.clone().nth(c as usize - 0x80),
        0xED => Some('¿'),
        0xEE => Some('¡'),
        0xF2 => Some('©'),
        0xF3 => Some('«'),
        0xF6 => Some('»'),
        0xFA => Some('“'),
        0xFC => Some('„'),
        0x112 => Some('™'),
        0x1FB => Some('°'),
        0x20F => None,
        _ => Some(c),
    }
}

/// LANG v4: entry table, then `key\0\x01text\0` pairs. `|` becomes a newline.
fn lang(b: &[u8]) -> Map<String, Value> {
    let n = u32le(b, 0) as usize;
    let parts: Vec<&[u8]> = b[4 + 12 * n..].split(|&c| c == 0).collect();
    let mut out = Map::new();
    let mut i = 0;
    while i + 1 < parts.len() {
        if parts[i + 1].first() == Some(&1) {
            let text: String = String::from_utf8_lossy(&parts[i + 1][1..]).chars().filter_map(glyph).map(|c| if c == '|' { '\n' } else { c }).collect();
            out.insert(String::from_utf8_lossy(parts[i]).into_owned(), Value::String(text));
            i += 2;
        } else {
            i += 1;
        }
    }
    out
}

/// PICA200 texture: 8×8 Morton-ordered tiles, rows stored bottom-up; ETC1 in
/// 4×4 sub-blocks (optionally preceded by 4-bit alpha).
fn texture(b: &[u8]) -> Result<Image> {
    let (w, h, fmt) = (u16le(b, 0) as usize, u16le(b, 2) as usize, b[5]);
    let d = &b[0x80..];
    let mut rgba = vec![0u8; w * h * 4];
    let mut put = |x: usize, y: usize, c: [u8; 4]| { let o = ((h - 1 - y) * w + x) * 4; rgba[o..o + 4].copy_from_slice(&c); };
    if fmt == 40 || fmt == 41 {
        let mut o = 0;
        for ty in (0..h).step_by(8) {
            for tx in (0..w).step_by(8) {
                for (by, bx) in [(0, 0), (0, 4), (4, 0), (4, 4)] {
                    let alpha = (fmt == 41).then(|| { o += 8; u64::from_le_bytes(d[o - 8..o].try_into().unwrap()) });
                    let block = etc1(u64::from_le_bytes(d[o..o + 8].try_into().unwrap()));
                    o += 8;
                    for y in 0..4 {
                        for x in 0..4 {
                            let a = alpha.map_or(255, |a| ((a >> ((x * 4 + y) * 4)) & 15) as u8 * 17);
                            let [r, g, bl] = block[y * 4 + x];
                            put(tx + bx + x, ty + by + y, [r, g, bl, a]);
                        }
                    }
                }
            }
        }
        return Ok(Image { w, h, rgba });
    }
    let bits = match fmt { 32 => 32, 33 => 24, 34..=37 => 16, 38 | 39 | 42 => 8, 43 | 44 => 4, _ => bail!("unknown texture format {fmt}") };
    let mut i = 0usize;
    for ty in (0..h).step_by(8) {
        for tx in (0..w).step_by(8) {
            for m in 0..64 {
                let (x, y) = ((0..3).map(|k| ((m >> (2 * k)) & 1) << k).sum::<usize>(), (0..3).map(|k| ((m >> (2 * k + 1)) & 1) << k).sum::<usize>());
                let v = if bits == 4 { ((d[i / 2] >> (4 * (i & 1))) & 15) as u32 } else { (0..bits / 8).map(|k| (d[i * bits / 8 + k] as u32) << (8 * k)).sum() };
                i += 1;
                let x5 = |v: u32| ((v & 31) * 255 / 31) as u8;
                let c = match fmt {
                    32 => [(v >> 24) as u8, (v >> 16) as u8, (v >> 8) as u8, v as u8],
                    33 => [(v >> 16) as u8, (v >> 8) as u8, v as u8, 255],
                    34 => [x5(v >> 11), x5(v >> 6), x5(v >> 1), if v & 1 != 0 { 255 } else { 0 }],
                    35 => [(v >> 12 & 15) as u8 * 17, (v >> 8 & 15) as u8 * 17, (v >> 4 & 15) as u8 * 17, (v & 15) as u8 * 17],
                    36 => [x5(v >> 11), ((v >> 5 & 63) * 255 / 63) as u8, x5(v), 255],
                    37 => [(v >> 8) as u8, (v >> 8) as u8, (v >> 8) as u8, v as u8],
                    38 => [v as u8, v as u8, v as u8, 255],
                    39 => [255, 255, 255, v as u8],
                    42 => { let l = (v >> 4) as u8 * 17; [l, l, l, (v & 15) as u8 * 17] }
                    43 => { let l = v as u8 * 17; [l, l, l, 255] }
                    _ => [255, 255, 255, v as u8 * 17],
                };
                put(tx + x, ty + y, c);
            }
        }
    }
    Ok(Image { w, h, rgba })
}

fn etc1(v: u64) -> [[u8; 3]; 16] {
    const MODS: [[i32; 2]; 8] = [[2, 8], [5, 17], [9, 29], [13, 42], [18, 60], [24, 80], [33, 106], [47, 183]];
    let (hi, lo) = ((v >> 32) as u32, v as u32);
    let (c0, c1): ([i32; 3], [i32; 3]) = if hi & 2 != 0 {
        let base = |s: u32| ((hi >> s) & 31) as i32;
        let delta = |s: u32| { let d = ((hi >> s) & 7) as i32; if d >= 4 { d - 8 } else { d } };
        let b = [base(27), base(19), base(11)];
        let e = [b[0] + delta(24), b[1] + delta(16), b[2] + delta(8)];
        (b.map(|c| (c << 3) | (c >> 2)), e.map(|c| ((c & 31) << 3) | ((c & 31) >> 2)))
    } else {
        ([28, 20, 12].map(|s| ((hi >> s) & 15) as i32 * 17), [24, 16, 8].map(|s| ((hi >> s) & 15) as i32 * 17))
    };
    let (t0, t1) = (((hi >> 5) & 7) as usize, ((hi >> 2) & 7) as usize);
    let mut out = [[0u8; 3]; 16];
    for x in 0..4 {
        for y in 0..4 {
            let i = x * 4 + y;
            let second = if hi & 1 != 0 { y >= 2 } else { x >= 2 };
            let (base, table) = if second { (c1, t1) } else { (c0, t0) };
            let m = MODS[table][((lo >> i) & 1) as usize];
            let m = if (lo >> (16 + i)) & 1 != 0 { -m } else { m };
            out[y * 4 + x] = base.map(|c| (c + m).clamp(0, 255) as u8);
        }
    }
    out
}

/// One "0rtc" mesh: model-space vertices, triangle list.
struct Mesh { material: String, pos: Vec<[f32; 3]>, nrm: Vec<[f32; 3]>, uv: Vec<[f32; 2]>, color: Vec<[f32; 4]>, idx: Vec<u32> }

/// MODL v26: mesh table, materials (texture or `.mat` name), joints ("TJRM").
fn model(b: &[u8]) -> Result<(Vec<Mesh>, Vec<(String, [f32; 12])>)> {
    let (n_mesh, mesh_tab) = (u32le(b, 0) as usize, u32le(b, 4) as usize);
    let mut meshes = vec![];
    for e in 0..n_mesh {
        let r = mesh_tab + e * 16;
        let (mat, off) = (u32le(b, r + 8) as usize, u32le(b, r + 12) as usize);
        let material = String::from_utf8_lossy(cstr(b, u32le(b, mat + 16) as usize)).to_ascii_lowercase();
        let w = |i: usize| u32le(b, off + i * 4);
        ensure!(&b[off..off + 4] == b"0rtc", "mesh {e} is not a 0rtc chunk");
        let (vb, n_idx, ib) = (off + w(27) as usize, w(28) as usize, off + w(32) as usize);
        ensure!(w(29) >> 16 == 0x1403, "mesh {e}: unsupported index type");
        let idx: Vec<u32> = (0..n_idx).map(|i| u16le(b, ib + i * 2) as u32).collect();
        let nv = idx.iter().max().map_or(0, |m| m + 1) as usize;
        let mut mesh = Mesh { material, pos: vec![], nrm: vec![], uv: vec![], color: vec![], idx };
        for a in 0..w(8).count_ones() as usize {
            let (desc, stride, ao) = (w(9 + a * 3), w(10 + a * 3) as usize, w(11 + a * 3) as usize);
            let (ty, comps, slot) = (desc >> 16, (desc >> 8 & 0xFF) as usize, desc & 0xFF);
            let get = |v: usize, k: usize| -> f32 {
                let o = vb + v * stride + ao;
                match ty { 0x1406 => f32le(b, o + 4 * k), 0x1401 => b[o + k] as f32 / 255.0, 0x1400 => b[o + k] as i8 as f32 / 127.0, 0x1402 => u16le(b, o + 2 * k) as i16 as f32 / 32767.0, _ => 0.0 }
            };
            // PWR is mirrored against glTF on Z; texture rows are bottom-up.
            for v in 0..nv {
                match (slot, comps) {
                    (0, _) => mesh.pos.push([get(v, 0), get(v, 1), -get(v, 2)]),
                    (1, 4) => mesh.color.push([get(v, 0), get(v, 1), get(v, 2), get(v, 3)]),
                    (2, 3) => mesh.nrm.push([get(v, 0), get(v, 1), -get(v, 2)]),
                    (3, _) => mesh.uv.push([get(v, 0), 1.0 - get(v, 1)]),
                    (5, 4) => {
                        // Tangent frame quaternion; the normal is its rotated +Z.
                        let (x, y, z, q) = (get(v, 0), get(v, 1), get(v, 2), get(v, 3));
                        mesh.nrm.push([2.0 * (x * z + q * y), 2.0 * (y * z - q * x), -(1.0 - 2.0 * (x * x + y * y))]);
                    }
                    _ => {}
                }
            }
        }
        // Index lists are unrolled strips whose odd triangles keep strip order;
        // the 3DS draws them unculled. Orient each one along its vertex normals.
        if mesh.nrm.len() == mesh.pos.len() {
            for t in mesh.idx.chunks_mut(3) {
                let [a, b, c] = [t[0], t[1], t[2]].map(|i| mesh.pos[i as usize]);
                let (e1, e2) = ([b[0] - a[0], b[1] - a[1], b[2] - a[2]], [c[0] - a[0], c[1] - a[1], c[2] - a[2]]);
                let face = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
                let n: f32 = t.iter().map(|&i| { let v = mesh.nrm[i as usize]; v[0] * face[0] + v[1] * face[1] + v[2] * face[2] }).sum();
                if n < 0.0 { t.swap(1, 2); }
            }
        }
        meshes.push(mesh);
    }
    let mut joints = vec![];
    if let Some(o) = b.windows(4).position(|w| w == b"TJRM") {
        for j in 0..u32le(b, o + 8) as usize {
            let p = o + 12 + j * 0x54;
            joints.push((String::from_utf8_lossy(cstr(b, p)).into_owned(), std::array::from_fn(|k| f32le(b, p + 32 + 4 * k))));
        }
    }
    Ok((meshes, joints))
}

/// A Mii favourite colour as `colortex.val` defines it (ARGB).
struct Favorite { name: String, primary: u32, complement: u32, accent: u32, logo: u32 }

fn argb(v: &Value) -> Option<u32> { u32::from_str_radix(v.as_str()?.trim_start_matches('#'), 16).ok() }

fn favorites(tree: &Value) -> Vec<Favorite> {
    tree["color"].as_array().into_iter().flatten().filter_map(|c| Some(Favorite {
        name: c["name"].as_str()?.to_string(),
        primary: argb(&c["primary"])?,
        complement: argb(&c["complement"])?,
        accent: argb(&c["accent"]).unwrap_or(0xFFFF_FFFF),
        logo: argb(&c["logo"]).unwrap_or(0xFFF0_F0F0),
    })).collect()
}

/// Bakes a vehicle's livery from `body_colors.val` layers for one favourite
/// colour: an untextured layer fills, "decal" layers blend by alpha and
/// "overlay" layers apply the usual screen/multiply split around half grey.
fn paint(pwr: &Pwr, containers: &[&str], layers: &Value, exceptions: &[Value], fav: &Favorite) -> Option<Image> {
    let base = pwr.texture(containers, &layers["base_texture"].as_str()?.to_ascii_lowercase())?;
    let (w, h) = (base.w, base.h);
    let mut c = vec![[1.0f32; 3]; w * h];
    for (i, layer) in layers["layer"].as_array()?.iter().enumerate() {
        let name = layer["texture"].as_str().unwrap_or("").to_ascii_lowercase();
        let tex = if name.is_empty() { None } else { Some(pwr.texture(containers, &name).filter(|t| t.w == w && t.h == h)?) };
        let source = layer["color_source"].as_str().unwrap_or("color");
        let mut col = match source {
            "favorite" | "favorite_sat" => fav.primary,
            "complement" | "complement_sat" => fav.complement,
            "accent" => fav.accent,
            "logo" => fav.logo,
            _ => argb(&layer["color"]).unwrap_or(0xFFFF_FFFF),
        };
        if let Some(e) = exceptions.iter().find(|e| e["name"].as_str() == Some(&fav.name) && e["index"].as_i64() == Some(i as i64)) {
            col = argb(&e["color"]).unwrap_or(col);
        }
        let l = [(col >> 16) & 255, (col >> 8) & 255, col & 255, col >> 24].map(|v| v as f32 / 255.0);
        let overlay = layer["operation"].as_str() == Some("overlay");
        for (p, px) in c.iter_mut().enumerate() {
            let t = tex.as_ref().map_or([1.0; 4], |t| std::array::from_fn(|k| t.rgba[p * 4 + k] as f32 / 255.0));
            let a = t[3] * l[3];
            for k in 0..3 {
                let b = t[k] * l[k];
                let target = if !overlay { b } else if px[k] < 0.5 { 2.0 * px[k] * b } else { 1.0 - 2.0 * (1.0 - px[k]) * (1.0 - b) };
                px[k] += (target - px[k]) * a;
            }
        }
    }
    Some(Image { w, h, rgba: c.iter().flat_map(|p| [p[0], p[1], p[2]].map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8).into_iter().chain([255])).collect() })
}

/// Everything needed to resolve names across containers.
struct Pwr { payloads: HashMap<String, Vec<u8>>, tocs: HashMap<String, Vec<Res>>, names: HashMap<u32, String>, livery: Option<(String, Image, String)> }

impl Pwr {
    fn res(&self, container: &str, name: &str) -> Option<(&Res, &[u8])> {
        let r = self.tocs.get(container)?.iter().find(|r| r.name == name)?;
        Some((r, &self.payloads.get(container)?[r.off..r.off + r.size]))
    }
    /// A texture by name, searched in the given containers in order.
    fn texture(&self, containers: &[&str], name: &str) -> Option<Image> {
        if let Some((orig, img, _)) = self.livery.as_ref().filter(|(o, _, _)| o == name) {
            return Some(Image { w: img.w, h: img.h, rgba: img.rgba.clone() }).filter(|_| !orig.is_empty());
        }
        containers.iter().find_map(|c| self.res(c, name).filter(|(r, _)| r.kind == "TEX")).and_then(|(_, d)| texture(d).ok())
    }
    /// The base texture of a material: a texture name directly, or the first
    /// textured layer of a `.mat` key tree.
    fn base_texture(&self, containers: &[&str], material: &str) -> Option<(String, Image)> {
        if material.ends_with(".mat") {
            let tree = self.material(containers, material)?;
            let tex = tree["layer"].as_array()?.iter().find_map(|l| l["texture"].as_str())?.to_ascii_lowercase();
            return self.texture(containers, &tex).map(|i| (tex, i));
        }
        self.texture(containers, material).map(|i| (material.to_string(), i))
    }
    fn material(&self, containers: &[&str], name: &str) -> Option<Value> {
        let (_, d) = containers.iter().find_map(|c| self.res(c, name))?;
        valt(d, &self.names).ok()
    }
}

fn to_glb(pwr: &Pwr, containers: &[&str], meshes: &[Mesh], joints: &[(String, [f32; 12])], name: &str) -> Vec<u8> {
    let mut g = Gltf::default();
    g.samplers.push(json!({ "magFilter": 9729, "minFilter": 9987, "wrapS": 10497, "wrapT": 10497 }));
    let mut materials: HashMap<String, usize> = HashMap::new();
    let mut prims = vec![];
    for m in meshes.iter().filter(|m| !m.pos.is_empty() && !m.idx.is_empty()) {
        let mat = match materials.get(&m.material) {
            Some(&i) => i,
            None => {
                let mut mj = json!({ "name": m.material, "pbrMetallicRoughness": { "metallicFactor": 0.0, "roughnessFactor": 0.8 }, "doubleSided": false });
                if let Some((tex, img)) = pwr.base_texture(containers, &m.material) {
                    // The game swaps this texture for the chosen favourite colour.
                    if let Some((_, _, vehicle)) = pwr.livery.as_ref().filter(|(o, _, _)| *o == tex) {
                        mj["extras"] = json!({ "livery": vehicle });
                    }
                    if let Ok(png) = img.png() {
                        let image = g.image_png(&tex, &png);
                        g.textures.push(json!({ "sampler": 0, "source": image }));
                        mj["pbrMetallicRoughness"]["baseColorTexture"] = json!({ "index": g.textures.len() - 1 });
                        if img.has_alpha() { mj["alphaMode"] = json!("MASK"); }
                        // Blending and texture scrolling from the `.mat` key tree.
                        if let Some(tree) = m.material.ends_with(".mat").then(|| pwr.material(containers, &m.material)).flatten() {
                            let global = &tree["global"][0];
                            match (global["src_blend"].as_str(), global["dst_blend"].as_str()) {
                                (_, Some("one")) => { mj["alphaMode"] = json!("BLEND"); mj["extras"]["blend"] = json!("add"); }
                                (Some("srca"), Some("inv_srca")) => mj["alphaMode"] = json!("BLEND"),
                                _ => {}
                            }
                            if let Some(layer) = tree["layer"].as_array().and_then(|l| l.iter().find(|l| l["texture"].is_string())) {
                                let (u, v) = (layer["tex_anim_u"].as_f64().unwrap_or(0.0), layer["tex_anim_v"].as_f64().unwrap_or(0.0));
                                if u != 0.0 || v != 0.0 { mj["extras"]["scroll"] = json!([u, v]); }
                            }
                        }
                    }
                }
                g.materials.push(mj);
                materials.insert(m.material.clone(), g.materials.len() - 1);
                g.materials.len() - 1
            }
        };
        // The 3DS draws these unculled. Bevy culls back faces, so add the
        // reverse side as real geometry with flipped normals.
        let back = |v: &[[f32; 3]]| v.iter().map(|n| n.map(|c| -c)).collect::<Vec<_>>();
        let twice = |v: &[[f32; 3]]| [v, v].concat();
        let n = m.pos.len() as u32;
        let pos = twice(&m.pos);
        let nrm = [m.nrm.clone(), back(&m.nrm)].concat();
        let uv = [m.uv.clone(), m.uv.clone()].concat();
        let color = [m.color.clone(), m.color.clone()].concat();
        let idx: Vec<u32> = m.idx.iter().copied().chain(m.idx.chunks(3).flat_map(|t| [t[0] + n, t[2] + n, t[1] + n])).collect();
        let mut attrs = Map::new();
        attrs.insert("POSITION".into(), json!(g.floats(&pos.concat(), "VEC3", 3, true, Some(34962))));
        if nrm.len() == pos.len() { attrs.insert("NORMAL".into(), json!(g.floats(&nrm.concat(), "VEC3", 3, false, Some(34962)))); }
        if uv.len() == pos.len() { attrs.insert("TEXCOORD_0".into(), json!(g.floats(&uv.concat(), "VEC2", 2, false, Some(34962)))); }
        if color.len() == pos.len() { attrs.insert("COLOR_0".into(), json!(g.floats(&color.concat(), "VEC4", 4, false, Some(34962)))); }
        let indices = g.u32s(&idx);
        prims.push(json!({ "attributes": attrs, "indices": indices, "material": mat }));
    }
    g.meshes.push(json!({ "name": name, "primitives": prims }));
    g.nodes.push(json!({ "name": name, "mesh": 0 }));
    let mut roots = vec![0];
    // Joints become named empty nodes so the game can find attachment points.
    for (jname, m) in joints {
        let t = [m[9], m[10], -m[11]];
        g.nodes.push(json!({ "name": jname, "translation": t }));
        roots.push(g.nodes.len() - 1);
    }
    g.scenes.push((name.to_string(), roots));
    g.glb()
}

/// Vehicle containers: (shared model container, per-vehicle data container).
const VEHICLES: &[&str] = &["plane", "jet", "rocket", "superrocket", "glider", "pglider", "mecahawk", "ufo"];
const LANGS: &[(&str, &str)] = &[("EN", "euenglish"), ("FR", "eufrench"), ("GE", "eugerman"), ("SP", "euspanish"), ("IT", "euitalian")];

pub fn extract(path: &Path, out: &Path) -> Result<()> {
    let rom = Rom::open(path)?;
    println!("Extracting Pilotwings Resort from {}", path.display());
    let toc_file = rom.files.iter().find(|(k, _)| k.ends_with("tocres.res")).map(|(_, v)| v).context("tocres.res missing")?;
    let tocs = tocs(&container(toc_file)?);
    let mut payloads = HashMap::new();
    for (file, d) in &rom.files {
        let base = file.rsplit('/').next().unwrap_or(file);
        let Some((stem, ext)) = base.rsplit_once('.') else { continue };
        if ["car", "trk", "res"].contains(&ext) && tocs.contains_key(stem) {
            payloads.insert(stem.to_string(), container(d).with_context(|| file.clone())?);
        }
    }
    let names = dictionary(std::iter::once(rom.code.as_slice()).chain(payloads.values().map(Vec::as_slice)));
    let mut pwr = Pwr { payloads, tocs, names, livery: None };
    let out = out.join("pwr");
    for d in ["data", "models", "text", "textures"] { fs::create_dir_all(out.join(d))?; }
    // Every key tree, as JSON, for the game to pick from.
    let mut count = 0;
    for (cname, list) in &pwr.tocs {
        for r in list.iter().filter(|r| r.kind == "VALT") {
            let Some(p) = pwr.payloads.get(cname) else { continue };
            let tree = valt(&p[r.off..r.off + r.size], &pwr.names).with_context(|| format!("{cname}/{}", r.name))?;
            let dir = out.join("data").join(cname);
            fs::create_dir_all(&dir)?;
            fs::write(dir.join(format!("{}.json", r.name.replace(['\\', '/'], "__"))), serde_json::to_vec(&tree)?)?;
            count += 1;
        }
    }
    println!("  {count} data tables");
    for (code, cname) in LANGS {
        let Some((_, d)) = pwr.res(cname, "lang.lng") else { continue };
        fs::write(out.join(format!("text/{code}.json")), serde_json::to_vec(&Value::Object(lang(d)))?)?;
    }
    println!("  text");
    let favs = pwr.res("race_", "colortex.val").and_then(|(_, d)| valt(d, &pwr.names).ok()).map(|t| favorites(&t)).unwrap_or_default();
    let mut liveries = Map::new();
    for v in VEHICLES {
        let data = format!("{v}_");
        let containers = [*v, data.as_str(), "common", "freeflight_"];
        // Liveries are painted per favourite colour; models embed the first.
        pwr.livery = None;
        if let Some(body) = pwr.res(&data, "body_colors.val").and_then(|(_, d)| valt(d, &pwr.names).ok()) {
            let body = &body["body_colors"][0];
            let layers = &body["vehicle_layers"][0];
            let exceptions = body["exception"].as_array().cloned().unwrap_or_default();
            if let Some(orig) = layers["orig_texture"].as_str().map(str::to_ascii_lowercase) {
                for (i, fav) in favs.iter().enumerate() {
                    let Some(img) = paint(&pwr, &containers, layers, &exceptions, fav) else { continue };
                    fs::write(out.join(format!("textures/{v}_{}.png", fav.name)), img.png()?)?;
                    if i == 0 { pwr.livery = Some((orig.clone(), img, v.to_string())); }
                }
                liveries.insert(v.to_string(), json!(orig));
            }
        }
        let Some(list) = pwr.tocs.get(*v) else { continue };
        for r in list.iter().filter(|r| r.kind == "MODL") {
            let (meshes, joints) = model(&pwr.payloads[*v][r.off..r.off + r.size]).with_context(|| format!("{v}/{}", r.name))?;
            let stem = r.name.trim_end_matches(".mod");
            fs::write(out.join(format!("models/{v}_{stem}.glb")), to_glb(&pwr, &containers, &meshes, &joints, stem))?;
        }
        println!("  vehicle {v}");
    }
    pwr.livery = None;
    // Course objects (pads, rings, gates, thermals, trophies) and free-flight extras.
    for c in ["race", "freeflight"] {
        let data = format!("{c}_");
        let containers = [c, data.as_str(), "common", "wuhu", "wuhu_sunny"];
        let Some(list) = pwr.tocs.get(c) else { continue };
        for r in list.iter().filter(|r| r.kind == "MODL" && r.name.ends_with(".mod")) {
            let Ok((meshes, joints)) = model(&pwr.payloads[c][r.off..r.off + r.size]) else { println!("  skipped {c}/{}", r.name); continue };
            let stem = r.name.trim_end_matches(".mod");
            fs::write(out.join(format!("models/{c}_{stem}.glb")), to_glb(&pwr, &containers, &meshes, &joints, stem))?;
        }
        println!("  objects {c}");
    }
    let names: Vec<_> = favs.iter().map(|f| json!({ "name": f.name, "primary": format!("#{:08x}", f.primary) })).collect();
    fs::write(out.join("data/liveries.json"), serde_json::to_vec(&json!({ "favorites": names, "vehicles": liveries }))?)?;
    fs::write(out.join("VERSION"), format!("{}\n", env!("CARGO_PKG_VERSION")))?;
    println!("Done. Pilotwings Resort assets written to {}", out.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_hash_is_case_insensitive_base_37() {
        assert_eq!(key_hash("bound0"), 0x1d15eec);
        assert_eq!(key_hash("BOUND0"), key_hash("bound0"));
        assert_eq!(key_hash("__ver"), 0xe948e43);
    }

    #[test]
    fn glyphs_follow_the_font_order() {
        let s: String = "caf\u{a6} \u{112} n\u{1fb}".chars().filter_map(glyph).collect();
        assert_eq!(s, "café ™ n°");
        assert_eq!(glyph('\u{80}'), Some('À'));
        assert_eq!(glyph('\u{bb}'), Some('œ'));
    }

    #[test]
    fn quicklz_literal_stream_round_trips() {
        // Uncompressed short-header stream: flags 0, size 5.
        assert_eq!(quicklz3(&[0x04, 8, 5, b'h', b'e', b'l', b'l', b'o']).unwrap(), b"hello");
    }

    #[test]
    fn etc1_flat_block_decodes_to_its_base_colour() {
        // Individual mode, both halves colour (8,4,2)*17, table 0, all modifiers +2.
        let hi: u32 = (8 << 28) | (8 << 24) | (4 << 20) | (4 << 16) | (2 << 12) | (2 << 8);
        let px = etc1((hi as u64) << 32);
        assert!(px.iter().all(|p| *p == [138, 70, 36]));
    }
}
