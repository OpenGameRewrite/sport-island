//! BMG message files -> `{ "id": "text" }`.

use crate::disc::{be16, be32};
use serde_json::{Map, Value};

pub fn to_json(d: &[u8]) -> Value {
    let mut sections = std::collections::HashMap::new();
    let mut off = 32;
    while off + 8 <= d.len() {
        let size = be32(d, off + 4) as usize;
        if size == 0 {
            break;
        }
        sections.insert(d[off..off + 4].to_vec(), off);
        off += size;
    }
    let (inf, dat, mid) = (sections[&b"INF1"[..]], sections[&b"DAT1"[..]] + 8, sections[&b"MID1"[..]]);
    let (count, entry) = (be16(d, inf + 8) as usize, be16(d, inf + 10) as usize);
    let mut out = Map::new();
    for i in 0..count.min(be16(d, mid + 8) as usize) {
        let id = be32(d, mid + 16 + 4 * i);
        let mut p = dat + be32(d, inf + 16 + i * entry) as usize;
        let mut s = String::new();
        loop {
            let c = be16(d, p);
            match c {
                0 => break,
                // Control escape: skip it (colours, sizes, variables, ruby).
                0x1A => p += d[p + 2] as usize,
                _ => {
                    s.push(char::from_u32(c as u32).unwrap_or('?'));
                    p += 2;
                }
            }
        }
        out.insert(format!("{id:08x}"), Value::String(s));
    }
    Value::Object(out)
}
