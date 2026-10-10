//! Convert the flight flare's REFF parameters and REFT textures from the disc.
//! The runtime implements a small subset of the original particle renderer.
//! Header layouts verified against BrawlCrate's REFT/REFF format definitions:
//! https://github.com/soopercool101/BrawlCrate/tree/master/BrawlLib/SSBB/Types
use crate::disc::{Disc, be16, be32, bef, cstr};
use anyhow::{Context, Result, ensure};
use serde_json::json;
use std::{collections::BTreeMap, fs, path::Path};

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
        // REFT lengths omit their 32-byte image header.
        let size = be32(d, cursor + 6 + n) as usize + if magic == b"REFT" {32} else {0};
        out.insert(name, d.get(start..start + size).context("effect outside file")?);
        cursor += n + 10;
    }
    Ok(out)
}

pub fn extract(disc: &mut Disc, out: &Path) -> Result<()> {
    let archive = disc.archive("Effect/PlnScene/effect.carc")?;
    let reff = archive.get("WS2_effect_pln.breff").context("flight REFF missing")?;
    let reft = archive.get("WS2_effect_pln.breft").context("flight REFT missing")?;
    let params = entries(reff, b"REFF")?;
    let textures = entries(reft, b"REFT")?;
    let mut definitions = BTreeMap::new();
    for name in ["WS2_pln_sign_st", "WS2_pln_sign_tail", "WS2_pln_sign_loop", "WS2_pln_sign_end", "WS2_pln_sign_tail_smk", "WS2_pln_sign_smk_bl_loop"] {
        let d = params.get(name).with_context(|| name.to_string())?;
        let p = 8 + be32(d, 4) as usize;
        ensure!(p + 142 <= d.len(), "truncated flare parameters");
        let texture = cstr(d, p + 142);
        let image = textures.get(&texture).with_context(|| texture.clone())?;
        let (w, h, bytes) = (be16(image, 4) as usize, be16(image, 6) as usize, be32(image, 8) as usize);
        let pixels = image.get(32..32 + bytes).context("truncated particle pixels")?;
        let palette: Vec<_> = image.get(32 + bytes..).unwrap_or_default().chunks_exact(2).take(be16(image, 14) as usize)
            .map(|c| crate::tex::palette_color(image[13] as u32, u16::from_be_bytes([c[0], c[1]]))).collect();
        fs::write(out.join(format!("effects/{texture}.png")), crate::tex::decode(pixels, image[12] as u32, w, h, Some(&palette))?.png()?)?;
        definitions.insert(name, json!({
            "texture": texture, "color": &d[p+4..p+8],
            "size": [bef(d,p+20)*bef(d,p+28), bef(d,p+24)*bef(d,p+32)],
            "life": be16(d,18) as f32 / 60.0,
            "wrap": [be16(d,p+120) & 3, (be16(d,p+120) >> 2) & 3],
        }));
    }
    fs::write(out.join("effects/flare.json"), serde_json::to_vec(&definitions)?)?;
    println!("  original flare textures and parameters");
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
}
