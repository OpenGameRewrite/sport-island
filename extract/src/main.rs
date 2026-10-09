//! Extracts and converts the assets Sport Island needs from the user's own
//! Wii Sports Resort disc image. Nothing produced here may be committed.

mod anim;
mod bmg;
mod disc;
mod gltf;
mod kcl;
mod mdl;
mod pmp;
mod sound;
mod tex;

use anyhow::{Context, Result, bail};
use disc::Disc;
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};

/// (archive on disc, BRRES inside it, output name)
const MODELS: &[(&str, &str, &str)] = &[
    ("Stage/Static/StageArc.carc", "Island/G3D/WS2_common_island.brres", "island"),
    ("Stage/Static/StageArc.carc", "Airship/G3D/WS2_common_airship.brres", "airship"),
    ("Stage/Static/StageArc.carc", "Balloon/G3D/WS2_common_balloon.brres", "balloon"),
    ("Stage/Static/StageArc.carc", "Bird/G3D/WS2_common_bird.brres", "bird"),
    ("Stage/Static/StageArc.carc", "Campfire/G3D/WS2_common_campfire.brres", "campfire"),
    ("Stage/Static/StageArc.carc", "Car/G3D/WS2_common_car.brres", "car"),
    ("Stage/Static/StageArc.carc", "Cruiser/G3D/WS2_common_cruiser.brres", "cruiser"),
    ("Stage/Static/StageArc.carc", "Fireworks/G3D/WS2_common_fireworks.brres", "fireworks"),
    ("Stage/Static/StageArc.carc", "Flag/G3D/WS2_common_FlagA1.brres", "flag_a"),
    ("Stage/Static/StageArc.carc", "Flag/G3D/WS2_common_FlagB1.brres", "flag_b"),
    ("Stage/Static/StageArc.carc", "Flag/G3D/WS2_common_FlagC1.brres", "flag_c"),
    ("Stage/Static/StageArc.carc", "Fountain/G3D/WS2_common_fountain.brres", "fountain"),
    ("Stage/Static/StageArc.carc", "Hawk/G3D/WS2_common_hawk.brres", "hawk"),
    ("Stage/Static/StageArc.carc", "Kayak/G3D/WS2_common_kayak.brres", "kayak"),
    ("Stage/Static/StageArc.carc", "Liner/G3D/WS2_common_move_liner.brres", "liner"),
    ("Stage/Static/StageArc.carc", "Parasol/G3D/WS2_common_parasol.brres", "parasol"),
    ("Stage/Static/StageArc.carc", "SeaGuard/G3D/WS2_common_seaguard.brres", "seaguard"),
    ("Stage/Static/StageArc.carc", "SeaPlane/G3D/WS2_common_seaplane.brres", "seaplane"),
    ("Stage/Static/StageArc.carc", "Sun/G3D/WS2_common_sun_model.brres", "sun"),
    ("Stage/Static/StageArc.carc", "Tent/G3D/WS2_common_tent.brres", "tent"),
    ("Stage/Static/StageArc.carc", "Torch/G3D/WS2_common_torch.brres", "torch"),
    ("Stage/Static/StageArc.carc", "Tree/G3D/WS2_common_tree.brres", "tree"),
    ("Stage/Static/StageArc.carc", "Tree/G3D/WS2_common_tree2.brres", "tree2"),
    ("Stage/Static/StageArc.carc", "Tree/G3D/WS2_common_tree3.brres", "tree3"),
    ("Stage/Static/StageArc.carc", "Whale/G3D/WS2_common_whale.brres", "whale"),
    ("Stage/Static/StageArc.carc", "WindMill/G3D/WS2_common_windmill.brres", "windmill"),
    ("Stage/Static/StageArc.carc", "Yacht/G3D/WS2_common_yacht.brres", "yacht"),
    ("Stage/Static/StageArc.carc", "Yadokari/G3D/WS2_common_yadokari.brres", "yadokari"),
    ("Stage/Static/StageArc.carc", "Audience/G3D/WS2_common_avaC0.brres", "walker"),
    ("Stage/PlnScene/PlnSub.carc", "G3D/WS2_pln_plane.brres", "plane"),
    ("Stage/PlnScene/PlnSub.carc", "G3D/WS2_pln_plane02.brres", "plane02"),
    ("Stage/PlnScene/PlnSub.carc", "G3D/WS2_pln_plane_water.brres", "plane_water"),
    ("Stage/PlnScene/PlnSub.carc", "G3D/WS2_pln_propeller.brres", "propeller"),
    ("Stage/PlnScene/PlnSub.carc", "G3D/WS2_pln_bullet.brres", "bullet"),
    ("Stage/PlnScene/PlnSub.carc", "G3D/WS2_pln_signalgun.brres", "signalgun"),
    ("Stage/PlnScene/PlnSub.carc", "G3D/WS2_pln_parachute.brres", "parachute"),
    ("Stage/PlnScene/Pln1P.carc", "G3D/WS2_pln_info_core.brres", "ipoint"),
    ("Stage/PlnScene/Pln1P.carc", "G3D/WS2_pln_guidplane.brres", "guideplane"),
    ("Stage/PlnScene/Pln1P.carc", "G3D/WS2_pln_small_balloon.brres", "small_balloon"),
    ("Stage/PlnScene/Pln1P.carc", "G3D/WS2_pln_giant_balloon.brres", "giant_balloon"),
    ("Stage/PlnScene/Pln1P.carc", "G3D/WS2_pln_tugboat.brres", "tugboat"),
];

const LANGS: &[(&str, &str)] = &[("EN", "English"), ("FR", "Français"), ("GE", "Deutsch"), ("SP", "Español"), ("IT", "Italiano")];

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let image = match args.next() {
        Some(p) => PathBuf::from(p),
        None => ask_path().context("usage: sport-island-extract <Wii Sports Resort disc image> [output dir]")?,
    };
    let out = PathBuf::from(args.next().unwrap_or_else(|| "assets".into()));
    // Optional third argument limits work while developing: text, data, audio, model[:name].
    let only = args.next();
    let want = |what: &str| only.as_deref().is_none_or(|o| o.starts_with(what));
    let mut disc = Disc::open(&image)?;
    if !disc.game_id.starts_with("RZT") {
        bail!("{} is {}, expected Wii Sports Resort (RZTxxx)", image.display(), disc.game_id);
    }
    println!("Extracting {} from {}", disc.game_id, image.display());
    for d in ["models", "text", "data", "audio"] {
        fs::create_dir_all(out.join(d))?;
    }
    if want("text") {
        text(&mut disc, &out)?;
    }
    if want("data") {
        data(&mut disc, &out)?;
    }
    if want("model") {
        models(&mut disc, &out, only.as_deref().and_then(|o| o.strip_prefix("model:")))?;
    }
    if want("audio") {
        sound::extract(&mut disc, &out.join("audio"))?;
    }
    fs::write(out.join("VERSION"), format!("{}\n", env!("CARGO_PKG_VERSION")))?;
    println!("Done. Assets written to {}", out.display());
    Ok(())
}

/// No GUI dependency: ask on stdin when no path is given.
fn ask_path() -> Option<PathBuf> {
    println!("Path to your Wii Sports Resort disc image (.iso/.rvz/.wbfs/.wia):");
    let mut s = String::new();
    std::io::stdin().read_line(&mut s).ok()?;
    let s = s.trim().trim_matches('"');
    (!s.is_empty()).then(|| PathBuf::from(s))
}

fn write_json(path: &Path, v: &Value) -> Result<()> {
    fs::write(path, serde_json::to_vec(v)?).with_context(|| path.display().to_string())
}

fn text(disc: &mut Disc, out: &Path) -> Result<()> {
    let mut langs = vec![];
    for (code, name) in LANGS {
        let Ok(arc) = disc.archive(&format!("{code}/Message/message.carc")) else { continue };
        let Some(bmg) = arc.iter().find(|(k, _)| k.ends_with(".bmg")).map(|(_, v)| v) else { continue };
        write_json(&out.join(format!("text/{code}.json")), &bmg::to_json(bmg))?;
        langs.push(json!({ "code": code, "name": name }));
        println!("  text {code}");
    }
    write_json(&out.join("text/languages.json"), &Value::Array(langs))
}

fn data(disc: &mut Disc, out: &Path) -> Result<()> {
    let env = disc.archive("Stage/PlnScene/PlnEnv0.carc")?;
    let sub = disc.archive("Stage/PlnScene/PlnSub.carc")?;
    let stage = disc.archive("Stage/Static/StageArc.carc")?;
    let pmps = [
        (&env, "WS2_pln_island_tag0.pmp", "island_tag"),
        (&env, "WS2_pln_guide_path0.pmp", "guide_path"),
        (&sub, "WS2_pln_island_F0.pmp", "island_f0"),
        (&stage, "SeaPlane/WS2_common_pln_path.pmp", "seaplane_path"),
    ];
    for (arc, name, outname) in pmps {
        let d = arc.iter().find(|(k, _)| k.ends_with(name)).map(|(_, v)| v).with_context(|| name.to_string())?;
        write_json(&out.join(format!("data/{outname}.json")), &pmp::to_json(d))?;
    }
    for i in 0..3 {
        let arc = disc.archive(&format!("Stage/PlnScene/PlnEnv{i}.carc"))?;
        let find = |ext: &str| arc.iter().find(|(k, _)| k.ends_with(ext)).map(|(_, v)| v.as_slice());
        write_json(&out.join(format!("data/env{i}.json")), &env_json(find(".plight"), find(".pfog")))?;
    }
    // i-points ("landmarks"): 0xBC-byte records after a 16-byte header.
    let act = disc.archive("Stage/Shared/Activity.carc")?;
    let lm = act.iter().find(|(k, _)| k.ends_with("LandMarkData.exbin")).map(|(_, v)| v).context("LandMarkData")?;
    let ipoints: Vec<Value> = (0..80)
        .map(|i| {
            let r = 0x10 + i * 0xBC;
            let f = |o: usize| disc::bef(lm, r + o);
            json!({
                "radius": f(0), "kind": disc::be32(lm, r + 4),
                "msg": disc::be32(lm, r + 8), "msg_alt": disc::be32(lm, r + 0xC),
                "pos": [f(0x9C), f(0xA0), f(0xA4)],
            })
        })
        .collect();
    write_json(&out.join("data/ipoints.json"), &Value::Array(ipoints))?;
    // Collision: island terrain and props.
    for arc in ["Stage/PlnScene/PlnCol_1.carc", "Stage/PlnScene/PlnCol_2.carc"] {
        for (k, v) in &disc.archive(arc)? {
            if let Some(name) = k.rsplit('/').next().and_then(|n| n.strip_suffix(".kcl")) {
                fs::write(out.join(format!("data/{name}.col")), kcl::to_tris(v))?;
            }
        }
    }
    println!("  data");
    Ok(())
}

/// Lights (LGHT/LOBJ) and fog (FOGM/FOGD) for one time of day.
fn env_json(light: Option<&[u8]>, fog: Option<&[u8]>) -> Value {
    use disc::{be16, be32, bef};
    let rgba = |d: &[u8], o: usize| [d[o], d[o + 1], d[o + 2], d[o + 3]];
    let v3 = |d: &[u8], o: usize| [bef(d, o), bef(d, o + 4), bef(d, o + 8)];
    let mut lights = vec![];
    let mut ambient = vec![];
    if let Some(d) = light {
        let (n, na) = (be16(d, 0x10) as usize, be16(d, 0x12) as usize);
        for i in 0..n {
            let o = 0x28 + i * 0x50;
            lights.push(json!({ "kind": be32(d, o + 0x10), "flags": be16(d, o + 0x16), "pos": v3(d, o + 0x18), "aim": v3(d, o + 0x24), "intensity": bef(d, o + 0x30), "color": rgba(d, o + 0x34) }));
        }
        for i in 0..na {
            ambient.push(json!(rgba(d, 0x28 + n * 0x50 + i * 8)));
        }
    }
    let mut fogs = vec![];
    if let Some(d) = fog {
        let n = be16(d, 0x10) as usize;
        for i in 0..n {
            let o = 0x14 + i * 0x30;
            fogs.push(json!({ "kind": d[o + 0x24], "start": bef(d, o + 0x10), "end": bef(d, o + 0x14), "color": rgba(d, o + 0x20) }));
        }
    }
    json!({ "lights": lights, "ambient": ambient, "fog": fogs })
}

fn models(disc: &mut Disc, out: &Path, only: Option<&str>) -> Result<()> {
    let mut cache: std::collections::HashMap<&str, disc::Archive> = Default::default();
    for (arc_path, brres_path, name) in MODELS {
        if only.is_some_and(|o| o != *name) {
            continue;
        }
        if !cache.contains_key(arc_path) {
            cache.insert(arc_path, disc.archive(arc_path)?);
        }
        let Some(d) = cache[arc_path].get(*brres_path) else {
            eprintln!("  missing {brres_path}");
            continue;
        };
        match convert(d, name) {
            Ok((glb, info)) => {
                fs::write(out.join(format!("models/{name}.glb")), glb)?;
                write_json(&out.join(format!("models/{name}.json")), &info)?;
                println!("  model {name}");
            }
            Err(e) => eprintln!("  model {name}: {e:#}"),
        }
    }
    Ok(())
}

fn convert(d: &[u8], name: &str) -> Result<(Vec<u8>, Value)> {
    let folders = mdl::brres(d);
    let images = mdl::textures(d, &folders);
    let mut models = vec![];
    for (mname, o) in folders.get("3DModels(NW4R)").into_iter().flatten() {
        models.push(mdl::model(d, *o, mname).with_context(|| format!("{name}/{mname}"))?);
    }
    let anims: Vec<_> = folders.get("AnmChr(NW4R)").into_iter().flatten().map(|(n, o)| anim::chr(d, *o, n)).collect();
    let srts: Vec<_> = folders.get("AnmTexSrt(NW4R)").into_iter().flatten().map(|(_, o)| anim::srt0(d, *o)).collect();
    let conv = mdl::to_glb(&models, &images, &anims);
    Ok((conv.glb, json!({ "materials": conv.info, "srt": srts })))
}
