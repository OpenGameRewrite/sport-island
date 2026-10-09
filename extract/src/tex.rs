//! GX texture decoding (TEX0 / BTI payloads) to RGBA8 and PNG.

use crate::disc::{be16, be32};
use anyhow::{Result, bail};

pub struct Image {
    pub w: usize,
    pub h: usize,
    pub rgba: Vec<u8>,
}

impl Image {
    pub fn png(&self) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        let mut enc = png::Encoder::new(&mut out, self.w as u32, self.h as u32);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header()?.write_image_data(&self.rgba)?;
        Ok(out)
    }
    pub fn has_alpha(&self) -> bool { self.rgba.chunks(4).any(|p| p[3] < 250) }
}

fn rgb565(c: u16) -> [u8; 4] {
    let (r, g, b) = ((c >> 11) & 31, (c >> 5) & 63, c & 31);
    [(r << 3 | r >> 2) as u8, (g << 2 | g >> 4) as u8, (b << 3 | b >> 2) as u8, 255]
}

fn rgb5a3(c: u16) -> [u8; 4] {
    if c & 0x8000 != 0 {
        let (r, g, b) = ((c >> 10) & 31, (c >> 5) & 31, c & 31);
        [(r << 3 | r >> 2) as u8, (g << 3 | g >> 2) as u8, (b << 3 | b >> 2) as u8, 255]
    } else {
        let (a, r, g, b) = ((c >> 12) & 7, (c >> 8) & 15, (c >> 4) & 15, c & 15);
        [(r * 17) as u8, (g * 17) as u8, (b * 17) as u8, (a << 5 | a << 2 | a >> 1) as u8]
    }
}

pub fn palette_color(fmt: u32, c: u16) -> [u8; 4] {
    match fmt {
        0 => [c as u8, c as u8, c as u8, (c >> 8) as u8],
        1 => rgb565(c),
        _ => rgb5a3(c),
    }
}

fn block(fmt: u32) -> (usize, usize, usize) {
    match fmt {
        0 | 8 | 14 => (8, 8, 32),
        1 | 2 | 9 => (8, 4, 32),
        6 => (4, 4, 64),
        _ => (4, 4, 32),
    }
}

pub fn decode(d: &[u8], fmt: u32, w: usize, h: usize, pal: Option<&[[u8; 4]]>) -> Result<Image> {
    let (bw, bh, bytes) = block(fmt);
    let mut rgba = vec![0u8; w * h * 4];
    let pal = |i: usize| pal.and_then(|p| p.get(i).copied()).unwrap_or([255, 0, 255, 255]);
    let mut put = |x: usize, y: usize, c: [u8; 4]| {
        if x < w && y < h {
            rgba[(y * w + x) * 4..][..4].copy_from_slice(&c);
        }
    };
    let mut src = 0;
    for by in (0..h).step_by(bh) {
        for bx in (0..w).step_by(bw) {
            let b = &d[src..src + bytes];
            src += bytes;
            match fmt {
                0 | 8 => {
                    for i in 0..64 {
                        let v = (b[i / 2] >> if i % 2 == 0 { 4 } else { 0 }) & 15;
                        let c = if fmt == 0 { let v = v * 17; [v, v, v, v] } else { pal(v as usize) };
                        put(bx + i % 8, by + i / 8, c);
                    }
                }
                1 | 9 => {
                    for i in 0..32 {
                        let v = b[i];
                        put(bx + i % 8, by + i / 8, if fmt == 1 { [v, v, v, v] } else { pal(v as usize) });
                    }
                }
                2 => {
                    for i in 0..32 {
                        let (a, l) = ((b[i] >> 4) * 17, (b[i] & 15) * 17);
                        put(bx + i % 8, by + i / 8, [l, l, l, a]);
                    }
                }
                3 => {
                    for i in 0..16 {
                        let (a, l) = (b[i * 2], b[i * 2 + 1]);
                        put(bx + i % 4, by + i / 4, [l, l, l, a]);
                    }
                }
                4 | 5 | 10 => {
                    for i in 0..16 {
                        let c = be16(b, i * 2);
                        let px = match fmt {
                            4 => rgb565(c),
                            5 => rgb5a3(c),
                            _ => pal((c & 0x3FFF) as usize),
                        };
                        put(bx + i % 4, by + i / 4, px);
                    }
                }
                6 => {
                    for i in 0..16 {
                        put(bx + i % 4, by + i / 4, [b[i * 2 + 1], b[32 + i * 2], b[32 + i * 2 + 1], b[i * 2]]);
                    }
                }
                14 => {
                    for sub in 0..4 {
                        let s = &b[sub * 8..];
                        let (c0, c1) = (be16(s, 0), be16(s, 2));
                        let (a, bb) = (rgb565(c0), rgb565(c1));
                        let mix = |x: u16, y: u16, i: usize| -> u8 { ((a[i] as u16 * x + bb[i] as u16 * y) / (x + y)) as u8 };
                        let cols = if c0 > c1 {
                            [a, bb, [mix(2, 1, 0), mix(2, 1, 1), mix(2, 1, 2), 255], [mix(1, 2, 0), mix(1, 2, 1), mix(1, 2, 2), 255]]
                        } else {
                            [a, bb, [mix(1, 1, 0), mix(1, 1, 1), mix(1, 1, 2), 255], [0, 0, 0, 0]]
                        };
                        let idx = be32(s, 4);
                        for i in 0..16 {
                            let ci = (idx >> (30 - 2 * i)) & 3;
                            put(bx + (sub % 2) * 4 + i % 4, by + (sub / 2) * 4 + i / 4, cols[ci as usize]);
                        }
                    }
                }
                _ => bail!("unsupported texture format {fmt}"),
            }
        }
    }
    Ok(Image { w, h, rgba })
}
