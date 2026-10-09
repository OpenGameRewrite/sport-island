//! Minimal binary glTF (GLB) writer.

use serde_json::{Value, json};

#[derive(Default)]
pub struct Gltf {
    pub bin: Vec<u8>,
    pub views: Vec<Value>,
    pub accessors: Vec<Value>,
    pub images: Vec<Value>,
    pub samplers: Vec<Value>,
    pub textures: Vec<Value>,
    pub materials: Vec<Value>,
    pub meshes: Vec<Value>,
    pub nodes: Vec<Value>,
    pub skins: Vec<Value>,
    pub animations: Vec<Value>,
    pub scene_roots: Vec<usize>,
}

impl Gltf {
    pub fn view(&mut self, bytes: &[u8], target: Option<u32>) -> usize {
        while self.bin.len() % 4 != 0 {
            self.bin.push(0);
        }
        let mut v = json!({ "buffer": 0, "byteOffset": self.bin.len(), "byteLength": bytes.len() });
        if let Some(t) = target {
            v["target"] = t.into();
        }
        self.bin.extend_from_slice(bytes);
        self.views.push(v);
        self.views.len() - 1
    }

    /// Float accessor of `n`-component vectors ("SCALAR", "VEC2", ...).
    pub fn floats(&mut self, data: &[f32], ty: &str, n: usize, bounds: bool, target: Option<u32>) -> usize {
        let bytes: Vec<u8> = data.iter().flat_map(|f| f.to_le_bytes()).collect();
        let view = self.view(&bytes, target);
        let mut a = json!({ "bufferView": view, "componentType": 5126, "count": data.len() / n, "type": ty });
        if bounds && !data.is_empty() {
            let (mut lo, mut hi) = (vec![f32::MAX; n], vec![f32::MIN; n]);
            for c in data.chunks(n) {
                for i in 0..n {
                    lo[i] = lo[i].min(c[i]);
                    hi[i] = hi[i].max(c[i]);
                }
            }
            a["min"] = json!(lo);
            a["max"] = json!(hi);
        }
        self.accessors.push(a);
        self.accessors.len() - 1
    }

    pub fn u32s(&mut self, data: &[u32]) -> usize {
        let bytes: Vec<u8> = data.iter().flat_map(|f| f.to_le_bytes()).collect();
        let view = self.view(&bytes, Some(34963));
        self.accessors.push(json!({ "bufferView": view, "componentType": 5125, "count": data.len(), "type": "SCALAR" }));
        self.accessors.len() - 1
    }

    pub fn u16x4(&mut self, data: &[u16]) -> usize {
        let bytes: Vec<u8> = data.iter().flat_map(|f| f.to_le_bytes()).collect();
        let view = self.view(&bytes, Some(34962));
        self.accessors.push(json!({ "bufferView": view, "componentType": 5123, "count": data.len() / 4, "type": "VEC4" }));
        self.accessors.len() - 1
    }

    pub fn image_png(&mut self, name: &str, png: &[u8]) -> usize {
        let view = self.view(png, None);
        self.images.push(json!({ "name": name, "bufferView": view, "mimeType": "image/png" }));
        self.images.len() - 1
    }

    pub fn glb(self) -> Vec<u8> {
        let mut root = json!({
            "asset": { "version": "2.0", "generator": "sport-island-extract" },
            "scene": 0,
            "scenes": [{ "nodes": self.scene_roots }],
            "nodes": self.nodes,
            "buffers": [{ "byteLength": self.bin.len() }],
            "bufferViews": self.views,
            "accessors": self.accessors,
        });
        let used = |k: &str, v: Vec<Value>, root: &mut Value| {
            if !v.is_empty() {
                root[k] = Value::Array(v);
            }
        };
        used("images", self.images, &mut root);
        used("samplers", self.samplers, &mut root);
        used("textures", self.textures, &mut root);
        used("materials", self.materials, &mut root);
        used("meshes", self.meshes, &mut root);
        used("skins", self.skins, &mut root);
        used("animations", self.animations, &mut root);
        root["extensionsUsed"] = json!(["KHR_materials_unlit", "KHR_materials_emissive_strength"]);
        let mut js = serde_json::to_vec(&root).unwrap();
        while js.len() % 4 != 0 {
            js.push(b' ');
        }
        let mut bin = self.bin;
        while bin.len() % 4 != 0 {
            bin.push(0);
        }
        let mut out = Vec::with_capacity(28 + js.len() + bin.len());
        out.extend_from_slice(b"glTF");
        out.extend_from_slice(&2u32.to_le_bytes());
        out.extend_from_slice(&((28 + js.len() + bin.len()) as u32).to_le_bytes());
        out.extend_from_slice(&(js.len() as u32).to_le_bytes());
        out.extend_from_slice(b"JSON");
        out.extend_from_slice(&js);
        out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
        out.extend_from_slice(b"BIN\0");
        out.extend_from_slice(&bin);
        out
    }
}
