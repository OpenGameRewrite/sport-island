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
}

impl Material for GxMaterial {
    fn fragment_shader() -> ShaderRef { "embedded://sport_island/gx.wesl".into() }
    fn alpha_mode(&self) -> AlphaMode { self.alpha_mode }
}

impl GxMaterial {
    pub fn set_srt(&mut self, i: usize, srt: [f32; 5]) {
        self.srt[i] = srt;
        let m = tex_mtx(srt);
        self.tev.texmtx[i * 2] = m[0];
        self.tev.texmtx[i * 2 + 1] = m[1];
    }
}

/// NW4R's Maya-convention texture matrix.
pub fn tex_mtx([sx, sy, rot, tx, ty]: [f32; 5]) -> [Vec4; 2] {
    let (s, c) = rot.to_radians().sin_cos();
    [
        Vec4::new(sx * c, -sx * s, sx * ((-0.5 * c) - (0.5 * s - 0.5) - tx), 0.0),
        Vec4::new(sy * s, sy * c, sy * ((-0.5 * c) + (0.5 * s - 0.5) + ty) + 1.0, 0.0),
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
    pub fog_color: Vec3,
    pub fog: Vec2,
}

/// Named materials, so animations can find them: (model path, material name) -> handle.
#[derive(Resource, Default)]
pub struct GxMaterials(pub HashMap<(String, String), Handle<GxMaterial>>);

pub fn plugin(app: &mut App) {
    embedded_asset!(app, "gx.wesl");
    let globals = app.world_mut().resource_mut::<Assets<ShaderBuffer>>().add(ShaderBuffer::from(vec![[0f32; 4]; 9]));
    app.add_plugins(MaterialPlugin::<GxMaterial>::default())
        .insert_resource(GxGlobals(globals))
        .init_resource::<GxEnv>()
        .init_resource::<GxMaterials>()
        .add_systems(Update, (convert, fix_image_formats, update_globals));
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
    let alpha_mode = if blend && dst == 1 {
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
    let mut flags = if def.fog >= 0 { 1 } else { 0 };
    if alpha_mode == AlphaMode::Add && src == 4 {
        flags |= 2;
    }
    tev.chan.w = flags;
    let mut tex: [Option<Handle<Image>>; 8] = Default::default();
    for m in &def.maps {
        if let (Some(t), true) = (m.tex, m.slot < 8) {
            tex[m.slot] = Some(assets.load(GltfAssetLabel::Texture(t).from_asset(path.to_string())));
        }
    }
    let [t0, t1, t2, t3, t4, t5, t6, t7] = tex;
    GxMaterial { tev, t0, t1, t2, t3, t4, t5, t6, t7, globals, alpha_mode, srt }
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
        let Some(path) = assets.get_path(&std.0) else { continue };
        let path = path.path().to_string_lossy().replace('\\', "/");
        let Ok(x) = serde_json::from_str::<Extras>(&extras.value) else { continue };
        let key = (path.clone(), x.gx.name.clone());
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
        env.fog_color.extend(0.0).to_array(),
        [env.fog.x, env.fog.y, 0.0, 0.0],
        cam.translation().extend(0.0).to_array(),
        Vec3::from(rows[0]).extend(0.0).to_array(),
        Vec3::from(rows[1]).extend(0.0).to_array(),
        Vec3::from(rows[2]).extend(0.0).to_array(),
    ];
    if let Some(mut b) = buffers.get_mut(&globals.0) {
        b.clear();
        b.extend(data);
    }
}
