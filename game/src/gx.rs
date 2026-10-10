//! Replaces glTF fallback materials with a GX/TEV emulating material built from
//! the original register state stored in each material's `extras.gx`.

use bevy::asset::embedded_asset;
use bevy::gltf::GltfMaterialExtras;
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::render_resource::{AsBindGroup, ShaderType, TextureFormat};
use bevy::render::storage::ShaderBuffer;
use bevy::shader::ShaderRef;
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Clone, Default, ShaderType, Debug)]
pub struct TevUniform {
    pub stages: [UVec4; 16],
    pub regs: [Vec4; 4],
    pub konst: [Vec4; 4],
    pub swap: [UVec4; 4],
    pub texmtx: [Vec4; 16],
    pub texgen: [UVec4; 8],
    pub chan: UVec4,
    pub mat_color: Vec4,
    pub amb_color: Vec4,
    pub alpha_test: UVec4,
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct GxMaterial {
    #[uniform(0)]
    pub tev: TevUniform,
    #[texture(1)]
    #[sampler(2)]
    pub t0: Option<Handle<Image>>,
    #[texture(3)]
    #[sampler(4)]
    pub t1: Option<Handle<Image>>,
    #[texture(5)]
    #[sampler(6)]
    pub t2: Option<Handle<Image>>,
    #[texture(7)]
    #[sampler(8)]
    pub t3: Option<Handle<Image>>,
    #[texture(9)]
    #[sampler(10)]
    pub t4: Option<Handle<Image>>,
    #[texture(11)]
    #[sampler(12)]
    pub t5: Option<Handle<Image>>,
    #[texture(13)]
    #[sampler(14)]
    pub t6: Option<Handle<Image>>,
    #[texture(15)]
    #[sampler(16)]
    pub t7: Option<Handle<Image>>,
    #[storage(17, read_only)]
    pub globals: Handle<ShaderBuffer>,
    pub alpha_mode: AlphaMode,
    /// Texture SRTs (scale x/y, rotation degrees, translation x/y) per texcoord, for animation.
    pub srt: [[f32; 5]; 8],
    /// Unanimated CLR0 targets (material, -, ambient, -, C0..C2, K0..K3) for masked colour animation.
    pub base: [Vec4; 11],
}

impl Material for GxMaterial {
    fn fragment_shader() -> ShaderRef { "embedded://sport_island/gx.wesl".into() }
    fn alpha_mode(&self) -> AlphaMode { self.alpha_mode }
}

impl GxMaterial {
    /// Applies one CLR0 value (big-endian RGBA8). Mask bits keep the original colour.
    pub fn set_color(&mut self, target: usize, mask: u32, value: u32) {
        let base = self.base[target.min(10)];
        let c = Vec4::from_array(std::array::from_fn(|i| {
            let m = (mask >> (24 - 8 * i)) & 0xFF;
            let b = (base[i] * 255.0).round() as u32;
            ((b & m) | ((value >> (24 - 8 * i)) & 0xFF & !m)) as f32 / 255.0
        }));
        match target {
            0 => self.tev.mat_color = c,
            2 => self.tev.amb_color = c,
            4..=6 => self.tev.regs[target - 3] = c,
            7..=10 => self.tev.konst[target - 7] = c,
            _ => {}
        }
    }

    pub fn set_srt(&mut self, i: usize, srt: [f32; 5]) {
        self.srt[i] = srt;
        let m = tex_mtx(srt);
        self.tev.texmtx[i * 2] = m[0];
        self.tev.texmtx[i * 2 + 1] = m[1];
    }
}

/// An untextured material lit like the island's models: sun, ambient and fog
/// for the current time of day.
pub fn shaded(color: Color, globals: Handle<ShaderBuffer>) -> GxMaterial {
    let c = color.to_srgba();
    // One stage passing the lit material colour through (d = rasterized colour/alpha, clamped).
    let mut tev = TevUniform { chan: UVec4::new(2, 0, 1, 1), swap: [UVec4::new(0, 1, 2, 3); 4], ..default() };
    tev.stages[0] = UVec4::new(15 << 12 | 15 << 8 | 15 << 4 | 10 | 1 << 19, 7 << 13 | 7 << 10 | 7 << 7 | 5 << 4 | 1 << 19, 0, 0);
    tev.mat_color = Vec4::new(c.red, c.green, c.blue, 1.0);
    tev.alpha_test = UVec4::new(7 | 7 << 4, 0, 0, 0);
    GxMaterial { tev, t0: None, t1: None, t2: None, t3: None, t4: None, t5: None, t6: None, t7: None, globals, alpha_mode: AlphaMode::Opaque, srt: [[1.0, 1.0, 0.0, 0.0, 0.0]; 8], base: [Vec4::ONE; 11] }
}

/// A texture modulated by the lit, fogged material colour: what non-Wii
/// models (the Pilotwings Resort add-on) need to sit in the island's lighting.
pub fn textured(texture: Option<Handle<Image>>, cutout: bool, globals: Handle<ShaderBuffer>) -> GxMaterial {
    let mut m = shaded(Color::WHITE, globals);
    if texture.is_some() {
        // TEXC × RASC and TEXA × RASA, texmap 0 from UV0, colour channel 0.
        m.tev.stages[0] = UVec4::new(15 << 12 | 8 << 8 | 10 << 4 | 15 | 1 << 19, 7 << 13 | 4 << 10 | 5 << 7 | 7 << 4 | 1 << 19, 1 << 6, 0);
        m.tev.texgen[0] = UVec4::new(5, 0, 0, 0);
        [m.tev.texmtx[0], m.tev.texmtx[1]] = tex_mtx([1.0, 1.0, 0.0, 0.0, 0.0]);
    }
    if cutout {
        // Alpha GREATER 127, second comparison ALWAYS.
        m.tev.alpha_test = UVec4::new(4 | 7 << 4, 127, 0, 0);
        m.alpha_mode = AlphaMode::Mask(0.0);
    }
    m.tev.chan.w = 1;
    m.t0 = texture;
    m
}

/// NW4R's Maya-convention texture matrix. Scale pivots on the texture centre:
/// without rotation, u' = sx (u - 0.5 - tx) + 0.5 and t' = sy (t - 0.5 + ty) + 0.5.
/// The firework burst relies on it: its texture holds one mirrored quadrant
/// centred on (1, 1), drawn with scale 2 and translation (-0.25, 0.25).
pub fn tex_mtx([sx, sy, rot, tx, ty]: [f32; 5]) -> [Vec4; 2] {
    let (s, c) = rot.to_radians().sin_cos();
    [
        Vec4::new(sx * c, -sx * s, sx * ((-0.5 * c) - (0.5 * s) - tx) + 0.5, 0.0),
        Vec4::new(sy * s, sy * c, sy * ((-0.5 * c) + (0.5 * s) + ty) + 0.5, 0.0),
    ]
}

#[derive(Deserialize)]
struct GxDef {
    name: String,
    xlu: bool,
    stages: u32,
    fog: i32,
    chan: Vec<(u32, [u8; 4], [u8; 4])>,
    maps: Vec<MapDef>,
    texmtx: Vec<TexMtxDef>,
    texgen: [u32; 8],
    bp: HashMap<String, u32>,
    regs: [[i32; 4]; 4],
    konst: [[i32; 4]; 4],
}

#[derive(Deserialize)]
struct MapDef {
    slot: usize,
    tex: Option<usize>,
}

#[derive(Deserialize)]
struct TexMtxDef {
    srt: [f32; 5],
    mode: u32,
}

#[derive(Deserialize)]
struct Extras {
    gx: GxDef,
}

/// Shared lighting/fog/camera values read by every GX material.
#[derive(Resource)]
pub struct GxGlobals(pub Handle<ShaderBuffer>);

/// Data written to the globals buffer each frame (see `gx.wgsl`).
#[derive(Resource, Default, Clone)]
pub struct GxEnv {
    pub sun_dir: Vec3,
    pub sun_color: Vec3,
    pub ambient: Vec3,
    /// The scene's first two fogs (materials select one by index): (colour, (start, end, type)).
    pub fogs: [(Vec3, Vec3); 2],
}

/// Converted materials: (model path, material name, glTF material label) -> handle.
/// The label matters: different scenes reuse names (the day and evening skies'
/// `mt_za_cloud` have different textures).
#[derive(Resource, Default)]
pub struct GxMaterials(pub HashMap<(String, String, String), Handle<GxMaterial>>);

#[derive(Deserialize)]
struct TextureAnimation {
    name: String,
    #[serde(rename = "loop")]
    looping: bool,
    materials: HashMap<String, HashMap<usize, Vec<[f32; 5]>>>,
}
/// CLR0 track for one material target: values per frame, mask bits keep the original.
#[derive(Deserialize, Clone)]
pub struct ColorTrack {
    pub mask: u32,
    pub values: Vec<u32>,
}
#[derive(Deserialize, Clone)]
pub struct ColorAnimation {
    #[serde(rename = "loop")]
    pub looping: bool,
    pub materials: HashMap<String, HashMap<usize, ColorTrack>>,
}
#[derive(Deserialize, Default)]
struct ModelAnimations {
    #[serde(default)]
    srt: Vec<TextureAnimation>,
    #[serde(default)]
    clr: Vec<ColorAnimation>,
}
#[derive(Resource, Default)]
struct TextureAnimations(HashMap<String, ModelAnimations>);

/// NW4R bone billboard mode from a glTF node's extras.
#[derive(Component)]
pub struct Billboard(u32);

pub fn plugin(app: &mut App) {
    embedded_asset!(app, "gx.wesl");
    let globals = app.world_mut().resource_mut::<Assets<ShaderBuffer>>().add(ShaderBuffer::from(vec![[0f32; 4]; 11]));
    app.add_plugins(MaterialPlugin::<GxMaterial>::default())
        .insert_resource(GxGlobals(globals))
        .init_resource::<GxEnv>()
        .init_resource::<GxMaterials>()
        .init_resource::<TextureAnimations>()
        .add_systems(Update, (convert, animate_textures, fix_image_formats, tag_billboards).chain())
        .add_systems(PostUpdate, billboards.after(bevy::app::AnimationSystems).before(TransformSystems::Propagate))
        // After propagation, so lighting/fog use this frame's camera, not last frame's.
        .add_systems(PostUpdate, update_globals.after(TransformSystems::Propagate));
}

fn animate_textures(time: Res<Time>, cache: Res<GxMaterials>, mut animations: ResMut<TextureAnimations>, mut materials: ResMut<Assets<GxMaterial>>) {
    let frame = time.elapsed_secs() * 60.0;
    for ((path, name, _), handle) in &cache.0 {
        let anims = animations.0.entry(path.clone()).or_insert_with(|| {
            let metadata = crate::asset_dir().join(path).with_extension("json");
            std::fs::read(metadata).ok().and_then(|b| serde_json::from_slice::<ModelAnimations>(&b).ok()).unwrap_or_default()
        });
        // Only looping colour tracks are ambient; one-shots (fireworks) are driven by their owners.
        let colors: Vec<_> = anims.clr.iter().filter(|a| a.looping).filter_map(|a| a.materials.get(name)).collect();
        if colors.is_empty() && !anims.srt.iter().any(|t| t.materials.contains_key(name)) { continue; }
        let Some(mut material) = materials.get_mut(handle) else { continue };
        for targets in colors {
            for (&target, track) in targets {
                if let Some(&v) = track.values.get(frame as usize % track.values.len().max(1)) { material.set_color(target, track.mask, v); }
            }
        }
        for track in &anims.srt {
            // The three sea variants share material names. Use the original base-water track.
            if matches!(track.name.as_str(), "WS2_common_sea_P" | "WS2_common_sea_S") { continue; }
            let Some(slots) = track.materials.get(name) else { continue };
            for (&slot, frames) in slots {
                if slot >= 8 || frames.is_empty() { continue; }
                let span = (frames.len() - 1).max(1) as f32;
                let frame = if track.looping { frame % span } else { frame.min(span) };
                let a = (frame.floor() as usize).min(frames.len() - 1);
                let b = (a + 1).min(frames.len() - 1);
                let srt = std::array::from_fn(|i| frames[a][i] + (frames[b][i] - frames[a][i]) * frame.fract());
                material.set_srt(slot, srt);
            }
        }
    }
}

fn decode(def: &GxDef, path: &str, assets: &AssetServer, globals: Handle<ShaderBuffer>) -> GxMaterial {
    let bp = |r: u32| def.bp.get(&format!("{r:02x}")).copied().unwrap_or(0);
    let mut tev = TevUniform { chan: UVec4::new(def.chan[0].0, def.chan.get(1).map_or(0, |c| c.0), def.stages.clamp(1, 16), 0), ..default() };
    for i in 0..tev.chan.z as usize {
        let (half, order) = (i % 2, bp(0x28 + i as u32 / 2));
        let o = (order >> (half * 12)) & 0x3FF;
        let ksel = bp(0xF6 + i as u32 / 2);
        let (kc, ka) = ((ksel >> (4 + half * 10)) & 31, (ksel >> (9 + half * 10)) & 31);
        tev.stages[i] = UVec4::new(bp(0xC0 + 2 * i as u32), bp(0xC1 + 2 * i as u32), o, kc | ka << 8);
    }
    for t in 0..4u32 {
        let (a, b) = (bp(0xF6 + 2 * t), bp(0xF7 + 2 * t));
        tev.swap[t as usize] = UVec4::new(a & 3, (a >> 2) & 3, b & 3, (b >> 2) & 3);
    }
    let c = |v: [i32; 4]| Vec4::from_array(v.map(|x| x as f32 / 255.0));
    tev.regs = def.regs.map(c);
    tev.konst = def.konst.map(c);
    tev.mat_color = Vec4::from_array(def.chan[0].1.map(|x| x as f32 / 255.0));
    tev.amb_color = Vec4::from_array(def.chan[0].2.map(|x| x as f32 / 255.0));
    let mut srt = [[1.0, 1.0, 0.0, 0.0, 0.0]; 8];
    for i in 0..8 {
        let (src_row, mode) = ((def.texgen[i] >> 7) & 31, def.texmtx.get(i).map_or(0, |m| m.mode));
        // Rows 5.. are texture coordinate sets; anything else falls back to UV0.
        tev.texgen[i] = UVec4::new(if src_row >= 5 { src_row } else { 5 }, mode, 0, 0);
        if let Some(m) = def.texmtx.get(i) {
            srt[i] = m.srt;
        }
        let m = tex_mtx(srt[i]);
        tev.texmtx[i * 2] = m[0];
        tev.texmtx[i * 2 + 1] = m[1];
    }
    let at = bp(0xF3);
    tev.alpha_test = UVec4::new((at >> 16) & 7 | ((at >> 19) & 7) << 4 | ((at >> 22) & 3) << 8, at & 0xFF, (at >> 8) & 0xFF, 0);
    // Blending: PE CMODE0 (enable, dst factor bits 5-7, src factor bits 8-10).
    let cm = bp(0x41);
    let (blend, dst, src) = (cm & 1 != 0, (cm >> 5) & 7, (cm >> 8) & 7);
    // An enabled blend unit does not make an opaque, depth-writing GX draw
    // translucent. Putting these meshes in Bevy's transparent pass lets the
    // island's large water meshes draw over the plane and other solid objects.
    let alpha_mode = if !def.xlu && bp(0x40) & 16 != 0 {
        if (at >> 16) & 7 != 7 || (at >> 19) & 7 != 7 { AlphaMode::Mask(0.0) } else { AlphaMode::Opaque }
    } else if blend && dst == 1 {
        AlphaMode::Add
    } else if blend && src == 0 && dst == 2 {
        AlphaMode::Multiply
    } else if blend || def.xlu {
        AlphaMode::Blend
    } else if (at >> 16) & 7 != 7 || (at >> 19) & 7 != 7 {
        AlphaMode::Mask(0.0)
    } else {
        AlphaMode::Opaque
    };
    // Bit 0: fogged; bit 8: which of the scene's fogs (only indices 0 and 1 are defined).
    let mut flags = if def.fog >= 0 { 1 | (def.fog.min(1) as u32) << 8 } else { 0 };
    // Bevy's Add is premultiplied blending: the shader scales by the GX source
    // factor itself and outputs alpha 0 so the destination is kept, as GX ONE does.
    if alpha_mode == AlphaMode::Add {
        flags |= 4 | if src == 4 { 2 } else { 0 };
    }
    tev.chan.w = flags;
    let mut tex: [Option<Handle<Image>>; 8] = Default::default();
    for m in &def.maps {
        if let (Some(t), true) = (m.tex, m.slot < 8) {
            tex[m.slot] = Some(assets.load(GltfAssetLabel::Texture(t).from_asset(path.to_string())));
        }
    }
    let [t0, t1, t2, t3, t4, t5, t6, t7] = tex;
    let base = [tev.mat_color, Vec4::ONE, tev.amb_color, Vec4::ONE, tev.regs[1], tev.regs[2], tev.regs[3], tev.konst[0], tev.konst[1], tev.konst[2], tev.konst[3]];
    GxMaterial { tev, t0, t1, t2, t3, t4, t5, t6, t7, globals, alpha_mode, srt, base }
}

/// Swap each glTF StandardMaterial for its GX equivalent as scenes spawn.
#[allow(clippy::too_many_arguments)]
fn convert(
    mut commands: Commands,
    q: Query<(Entity, &MeshMaterial3d<StandardMaterial>, &GltfMaterialExtras), Added<GltfMaterialExtras>>,
    assets: Res<AssetServer>,
    globals: Res<GxGlobals>,
    mut cache: ResMut<GxMaterials>,
    mut mats: ResMut<Assets<GxMaterial>>,
) {
    for (e, std, extras) in &q {
        let Some(asset_path) = assets.get_path(&std.0) else { continue };
        let path = asset_path.path().to_string_lossy().replace('\\', "/");
        let label = asset_path.label().unwrap_or_default().to_string();
        let Ok(x) = serde_json::from_str::<Extras>(&extras.value) else { continue };
        // The blimp's panel is a 4x4 grey placeholder the original fills or hides at
        // runtime; drawn as-is it z-fights with the Wii logo underneath.
        if x.gx.name == "mt_airship_dummy" {
            commands.entity(e).insert(Visibility::Hidden);
        }
        let key = (path.clone(), x.gx.name.clone(), label);
        let handle = cache.0.entry(key).or_insert_with(|| mats.add(decode(&x.gx, &path, &assets, globals.0.clone()))).clone();
        commands.entity(e).remove::<MeshMaterial3d<StandardMaterial>>().insert(MeshMaterial3d(handle));
    }
}

/// GX works on raw (gamma-space) texels: read textures without sRGB decoding.
fn fix_image_formats(mut events: MessageReader<AssetEvent<Image>>, mut images: ResMut<Assets<Image>>) {
    for ev in events.read() {
        if let AssetEvent::LoadedWithDependencies { id } | AssetEvent::Added { id } = ev
            && let Some(img) = images.get(*id)
            && img.texture_descriptor.format == TextureFormat::Rgba8UnormSrgb
            && let Some(mut img) = images.get_mut(*id)
        {
            img.texture_descriptor.format = TextureFormat::Rgba8Unorm;
        }
    }
}

fn update_globals(
    env: Res<GxEnv>,
    globals: Res<GxGlobals>,
    cam: Query<&GlobalTransform, With<crate::world::MainCamera>>,
    mut buffers: ResMut<Assets<ShaderBuffer>>,
) {
    let Ok(cam) = cam.single() else { return };
    let view = cam.affine().inverse().matrix3;
    let rows = [view.row(0), view.row(1), view.row(2)];
    let data: Vec<[f32; 4]> = vec![
        env.sun_dir.extend(0.0).to_array(),
        env.sun_color.extend(0.0).to_array(),
        env.ambient.extend(0.0).to_array(),
        env.fogs[0].0.extend(0.0).to_array(),
        env.fogs[0].1.extend(0.0).to_array(),
        cam.translation().extend(0.0).to_array(),
        Vec3::from(rows[0]).extend(0.0).to_array(),
        Vec3::from(rows[1]).extend(0.0).to_array(),
        Vec3::from(rows[2]).extend(0.0).to_array(),
        env.fogs[1].0.extend(0.0).to_array(),
        env.fogs[1].1.extend(0.0).to_array(),
    ];
    if let Some(mut b) = buffers.get_mut(&globals.0) {
        b.clear();
        b.extend(data);
    }
}

fn tag_billboards(mut commands: Commands, q: Query<(Entity, &bevy::gltf::GltfExtras), Added<bevy::gltf::GltfExtras>>) {
    for (e, extras) in &q {
        if let Some(mode) = serde_json::from_str::<serde_json::Value>(&extras.value).ok().and_then(|v| v["billboard"].as_u64()) {
            commands.entity(e).insert(Billboard(mode as u32));
        }
    }
}

/// Turns billboard bones to face the camera after animation has posed them.
/// ponytail: modes 1-4 all face the view (ROT modes' own roll is dropped); 5-6 turn about Y only.
fn billboards(cam: Single<&GlobalTransform, With<crate::world::MainCamera>>, parents: Query<&GlobalTransform>, mut q: Query<(&Billboard, &ChildOf, &mut Transform)>) {
    let cam = cam.compute_transform();
    for (mode, parent, mut t) in &mut q {
        let Ok(p) = parents.get(parent.parent()) else { continue };
        let world = if mode.0 >= 5 {
            let back = cam.rotation * Vec3::Z;
            Quat::from_rotation_y(back.x.atan2(back.z))
        } else {
            cam.rotation
        };
        t.rotation = p.compute_transform().rotation.inverse() * world;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(m: [Vec4; 2], st: Vec2) -> Vec2 { Vec2::new(m[0].xyz().dot(st.extend(1.0)), m[1].xyz().dot(st.extend(1.0))) }

    #[test]
    fn texture_matrix_scales_about_the_centre() {
        let id = tex_mtx([1.0, 1.0, 0.0, 0.0, 0.0]);
        assert!(apply(id, Vec2::new(0.2, 0.7)).abs_diff_eq(Vec2::new(0.2, 0.7), 1e-6));
        // Firework: the quad centre lands on the mirrored quadrant's corner.
        let fw = tex_mtx([2.0, 2.0, 0.0, -0.25, 0.25]);
        assert!(apply(fw, Vec2::splat(0.5)).abs_diff_eq(Vec2::ONE, 1e-6));
        assert!(apply(fw, Vec2::ZERO).abs_diff_eq(Vec2::ZERO, 1e-6));
    }
}
