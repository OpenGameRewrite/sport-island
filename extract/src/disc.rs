//! Disc image access (via `nod`), Yaz0 decompression and U8 archives.

use anyhow::{Context, Result, bail};
use nod::common::PartitionKind;
use nod::read::{DiscOptions, DiscReader, PartitionOptions, PartitionReader};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;

pub struct Disc {
    part: Box<dyn PartitionReader>,
    pub game_id: String,
}

impl Disc {
    pub fn open(path: &Path) -> Result<Self> {
        let opts = DiscOptions { preloader_threads: 4, ..Default::default() };
        let disc = DiscReader::new(path, &opts).with_context(|| format!("opening {path:?}"))?;
        let game_id = String::from_utf8_lossy(&disc.header().game_id).into_owned();
        let part = disc.open_partition_kind(PartitionKind::Data, &PartitionOptions::default())?;
        Ok(Self { part, game_id })
    }

    pub fn read(&mut self, path: &str) -> Result<Vec<u8>> {
        let meta = self.part.meta()?;
        let fst = meta.fst().map_err(anyhow::Error::msg)?;
        let (_, node) = fst.find(path).with_context(|| format!("{path} not on disc"))?;
        let mut buf = Vec::with_capacity(node.length() as usize);
        self.part.open_file(node)?.read_to_end(&mut buf)?;
        Ok(buf)
    }

    /// Read a data range mapped by the executable's DOL section table. Extracted
    /// palette data stays in assets; the executable itself is never written out.
    pub fn dol_data(&mut self, address: u32, length: u32) -> Result<Vec<u8>> {
        let meta = self.part.meta()?;
        let d = &meta.raw_dol;
        for i in 0..18 {
            let (offset, base, size) = (be32(d, i * 4), be32(d, 0x48 + i * 4), be32(d, 0x90 + i * 4));
            if address >= base && address.checked_add(length).is_some_and(|end| base.checked_add(size).is_some_and(|limit| end <= limit)) {
                let start = (offset + address - base) as usize;
                return d.get(start..start + length as usize).map(|v| v.to_vec()).context("DOL data range outside file");
            }
        }
        bail!("DOL data address not mapped: {address:08x}")
    }

    /// Reads a Yaz0-compressed U8 archive (`.carc`) and returns its files keyed by path.
    pub fn archive(&mut self, path: &str) -> Result<Archive> {
        u8_archive(&yaz0(&self.read(path)?)?).with_context(|| path.to_string())
    }
}

pub type Archive = BTreeMap<String, Vec<u8>>;

pub fn be16(d: &[u8], o: usize) -> u16 { u16::from_be_bytes([d[o], d[o + 1]]) }
pub fn be32(d: &[u8], o: usize) -> u32 { u32::from_be_bytes(d[o..o + 4].try_into().unwrap()) }
pub fn bef(d: &[u8], o: usize) -> f32 { f32::from_bits(be32(d, o)) }
pub fn cstr(d: &[u8], o: usize) -> String {
    let end = d[o..].iter().position(|&b| b == 0).map_or(d.len(), |e| o + e);
    String::from_utf8_lossy(&d[o..end]).into_owned()
}

pub fn yaz0(d: &[u8]) -> Result<Vec<u8>> {
    if &d[..4] != b"Yaz0" {
        return Ok(d.to_vec());
    }
    let size = be32(d, 4) as usize;
    let mut out = Vec::with_capacity(size);
    let mut i = 16;
    while out.len() < size {
        let code = d[i];
        i += 1;
        for bit in 0..8 {
            if out.len() >= size {
                break;
            }
            if code & (0x80 >> bit) != 0 {
                out.push(d[i]);
                i += 1;
            } else {
                let (b1, b2) = (d[i] as usize, d[i + 1] as usize);
                i += 2;
                let dist = ((b1 & 0xF) << 8 | b2) + 1;
                let len = if b1 >> 4 == 0 {
                    i += 1;
                    d[i - 1] as usize + 0x12
                } else {
                    (b1 >> 4) + 2
                };
                let start = out.len().checked_sub(dist).context("bad Yaz0 back-reference")?;
                for k in 0..len {
                    out.push(out[start + k]);
                }
            }
        }
    }
    Ok(out)
}

pub fn u8_archive(d: &[u8]) -> Result<Archive> {
    if be32(d, 0) != 0x55AA382D {
        bail!("not a U8 archive");
    }
    let root = be32(d, 4) as usize;
    let total = be32(d, root + 8) as usize;
    let strings = root + total * 12;
    let mut out = Archive::new();
    // (end index, path prefix)
    let mut stack: Vec<(usize, String)> = vec![(total, String::new())];
    for i in 1..total {
        while stack.last().is_some_and(|(end, _)| i >= *end) {
            stack.pop();
        }
        let e = root + i * 12;
        let name = cstr(d, strings + (be32(d, e) & 0xFFFFFF) as usize);
        let path = format!("{}{}", stack.last().map_or("", |s| &s.1), name);
        let (a, b) = (be32(d, e + 4) as usize, be32(d, e + 8) as usize);
        if d[e] == 1 {
            stack.push((b, format!("{path}/")));
        } else {
            out.insert(path, d[a..a + b].to_vec());
        }
    }
    Ok(out)
}
