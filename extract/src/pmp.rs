//! PMP placement files: object instances and point paths.

use crate::disc::{be16, be32, bef, cstr};
use serde_json::{Value, json};

pub fn to_json(d: &[u8]) -> Value {
    let (nobj, nroute, npts) = (be16(d, 0x10) as usize, be16(d, 0x12) as usize, be16(d, 0x14) as usize);
    let (oobj, oroute, opts) = (be32(d, 0x40) as usize, be32(d, 0x44) as usize, be32(d, 0x48) as usize);
    let f = |o: usize, n: usize| (0..n).map(|i| bef(d, o + 4 * i)).collect::<Vec<_>>();
    let objects: Vec<_> = (0..nobj)
        .map(|i| {
            let o = oobj + i * 0x58;
            json!({
                "group": be16(d, o), "kind": be16(d, o + 2),
                "pos": f(o + 8, 3), "scale": f(o + 20, 3), "rot": f(o + 32, 9),
                "params": (0..5).map(|k| be32(d, o + 68 + 4 * k)).collect::<Vec<_>>(),
            })
        })
        .collect();
    let points: Vec<_> = (0..npts).map(|i| f(opts + i * 20, 3)).collect();
    let routes: Vec<_> = (0..nroute)
        .map(|i| {
            let o = oroute + i * 32;
            let (n, start) = (be16(d, o) as usize, be32(d, o + 28) as usize);
            json!({ "name": cstr(d, o + 8), "flags": be16(d, o + 6), "points": points[start..start + n] })
        })
        .collect();
    json!({ "objects": objects, "routes": routes })
}
