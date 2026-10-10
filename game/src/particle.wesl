// NW4R-style particle draw: four GX TEV stages fed by each particle's colour
// layers, one or two texture layers, fog and the emitter's blend mode.

import bevy_pbr::render::mesh_functions::{get_world_from_local, mesh_position_local_to_world};
import bevy_pbr::render::view_transformations::position_world_to_clip;

struct Draw {
    // x colour stage (a | b<<4 | c<<8 | d<<12 | op<<16 | bias<<18 | scale<<20 | clamp<<22 | dest<<23)
    // y alpha stage (same packing), z texture layer (0, 1 or 255 for none), w konst colour | konst alpha << 8
    stages: array<vec4<u32>, 4>,
    // x stage count, y flags (1 fog, 2 premultiply, 4 zero alpha out), z/w unused
    info: vec4<u32>,
    // Register/konst colour sources (ColorInput): x colour regs, y colour konst, z alpha regs, w alpha konst (one byte each).
    inputs: vec4<u32>,
    // Texture wrap modes: tex1 S, tex1 T, tex2 S, tex2 T (0 clamp, 1 repeat, 2 mirror).
    wrap: vec4<u32>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> draw: Draw;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var t1: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var s1: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(3) var t2: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(4) var s2: sampler;
// Shared with the GX material: 3 fog colour, 4 fog (start, end, type), 5 camera, 8 view z row.
@group(constants::MATERIAL_BIND_GROUP) @binding(5) var<storage, read> globals: array<vec4<f32>>;

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) uv0: vec2<f32>,
    @location(2) uv1: vec2<f32>,
    // Colour layer 0 primary/secondary, layer 1 primary/secondary.
    @location(3) c0: vec4<f32>,
    @location(4) c1: vec4<f32>,
    @location(5) c2: vec4<f32>,
    @location(6) c3: vec4<f32>,
}

struct Out {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) uv0: vec2<f32>,
    @location(2) uv1: vec2<f32>,
    @location(3) c0: vec4<f32>,
    @location(4) c1: vec4<f32>,
    @location(5) c2: vec4<f32>,
    @location(6) c3: vec4<f32>,
}

@vertex
fn vertex(v: Vertex) -> Out {
    var out: Out;
    let world = mesh_position_local_to_world(get_world_from_local(v.instance_index), vec4<f32>(v.position, 1.0));
    out.clip = position_world_to_clip(world.xyz);
    out.world = world.xyz;
    out.uv0 = v.uv0;
    out.uv1 = v.uv1;
    out.c0 = v.c0;
    out.c1 = v.c1;
    out.c2 = v.c2;
    out.c3 = v.c3;
    return out;
}

fn wrap1(x: f32, mode: u32) -> f32 {
    if mode == 1u { return fract(x); }
    if mode == 2u { return 1.0 - abs(fract(x * 0.5) * 2.0 - 1.0); }
    return clamp(x, 0.0, 1.0);
}

fn source(sel: u32, inp: Out) -> vec4<f32> {
    switch sel {
        case 1u: { return inp.c0; }
        case 2u: { return inp.c1; }
        case 3u: { return inp.c2; }
        case 4u: { return inp.c3; }
        case 5u: { return inp.c0 * inp.c1; }
        case 6u: { return inp.c2 * inp.c3; }
        default: { return vec4<f32>(0.0); }
    }
}

fn konst_value(sel: u32, k: array<vec4<f32>, 4>) -> vec4<f32> {
    if sel <= 7u { return vec4<f32>(1.0 - f32(sel) / 8.0); }
    if sel >= 12u && sel <= 15u { return k[sel - 12u]; }
    if sel >= 16u {
        let c = k[(sel - 16u) % 4u];
        return vec4<f32>(c[(sel - 16u) / 4u]);
    }
    return vec4<f32>(1.0);
}

fn bias_scale(v: f32, env: u32) -> f32 {
    let bias = (env >> 18u) & 3u;
    let scale = (env >> 20u) & 3u;
    var r = v;
    if bias == 1u { r += 0.5; } else if bias == 2u { r -= 0.5; }
    if scale == 1u { r *= 2.0; } else if scale == 2u { r *= 4.0; } else if scale == 3u { r *= 0.5; }
    return r;
}

// GX adds in gamma space; the linear target would make glows far too faint
// over a bright sky. Add what gamma-space addition would over the scene's
// fog/sky colour, an estimate of the background.
fn additive(g: vec3<f32>) -> vec3<f32> {
    let d = globals[3].rgb;
    return max(pow(g + d, vec3<f32>(2.2)) - pow(d, vec3<f32>(2.2)), vec3<f32>(0.0));
}

@fragment
fn fragment(inp: Out) -> @location(0) vec4<f32> {
    let tex1 = textureSampleLevel(t1, s1, vec2<f32>(wrap1(inp.uv0.x, draw.wrap.x), wrap1(inp.uv0.y, draw.wrap.y)), 0.0);
    let tex2 = textureSampleLevel(t2, s2, vec2<f32>(wrap1(inp.uv1.x, draw.wrap.z), wrap1(inp.uv1.y, draw.wrap.w)), 0.0);

    // Registers: 0 PREV, 1-3 C0-C2; konst K0-K3.
    var r: array<vec4<f32>, 4>;
    var k: array<vec4<f32>, 4>;
    for (var i = 0u; i < 3u; i++) {
        r[i + 1u] = vec4<f32>(source((draw.inputs.x >> (8u * i)) & 255u, inp).rgb, source((draw.inputs.z >> (8u * i)) & 255u, inp).a);
    }
    for (var i = 0u; i < 4u; i++) {
        k[i] = vec4<f32>(source((draw.inputs.y >> (8u * i)) & 255u, inp).rgb, source((draw.inputs.w >> (8u * i)) & 255u, inp).a);
    }
    let ras = vec4<f32>(1.0);
    for (var s = 0u; s < max(draw.info.x, 1u); s++) {
        let st = draw.stages[s];
        var texc = vec4<f32>(1.0);
        if st.z == 0u { texc = tex1; } else if st.z == 1u { texc = tex2; }
        let kc = konst_value(st.w & 255u, k);
        let ka = konst_value((st.w >> 8u) & 255u, k);
        var cin: array<vec3<f32>, 16>;
        cin[0] = r[0].rgb; cin[1] = vec3<f32>(r[0].a);
        cin[2] = r[1].rgb; cin[3] = vec3<f32>(r[1].a);
        cin[4] = r[2].rgb; cin[5] = vec3<f32>(r[2].a);
        cin[6] = r[3].rgb; cin[7] = vec3<f32>(r[3].a);
        cin[8] = texc.rgb; cin[9] = vec3<f32>(texc.a);
        cin[10] = ras.rgb; cin[11] = vec3<f32>(ras.a);
        cin[12] = vec3<f32>(1.0); cin[13] = vec3<f32>(0.5);
        cin[14] = kc.rgb; cin[15] = vec3<f32>(0.0);
        var ain: array<f32, 8>;
        ain[0] = r[0].a; ain[1] = r[1].a; ain[2] = r[2].a; ain[3] = r[3].a;
        ain[4] = texc.a; ain[5] = ras.a; ain[6] = ka.a; ain[7] = 0.0;

        let ce = st.x;
        let ca = clamp(cin[ce & 15u], vec3<f32>(0.0), vec3<f32>(1.0));
        let cb = clamp(cin[(ce >> 4u) & 15u], vec3<f32>(0.0), vec3<f32>(1.0));
        let cc = clamp(cin[(ce >> 8u) & 15u], vec3<f32>(0.0), vec3<f32>(1.0));
        let cd = cin[(ce >> 12u) & 15u];
        let clerp = mix(ca, cb, cc);
        let cv = select(cd + clerp, cd - clerp, ((ce >> 16u) & 3u) == 1u);
        var cout = vec3<f32>(bias_scale(cv.r, ce), bias_scale(cv.g, ce), bias_scale(cv.b, ce));
        if ((ce >> 22u) & 1u) == 1u { cout = clamp(cout, vec3<f32>(0.0), vec3<f32>(1.0)); }

        let ae = st.y;
        let aa = clamp(ain[ae & 7u], 0.0, 1.0);
        let ab = clamp(ain[(ae >> 4u) & 7u], 0.0, 1.0);
        let ac = clamp(ain[(ae >> 8u) & 7u], 0.0, 1.0);
        let ad = ain[(ae >> 12u) & 7u];
        let alerp = mix(aa, ab, ac);
        var aout = bias_scale(select(ad + alerp, ad - alerp, ((ae >> 16u) & 3u) == 1u), ae);
        if ((ae >> 22u) & 1u) == 1u { aout = clamp(aout, 0.0, 1.0); }

        let cdst = (ce >> 23u) & 3u;
        let adst = (ae >> 23u) & 3u;
        r[cdst] = vec4<f32>(cout, r[cdst].a);
        r[adst].a = aout;
    }
    var out = clamp(r[0], vec4<f32>(0.0), vec4<f32>(1.0));

    if (draw.info.y & 1u) != 0u {
        let fog = globals[4];
        if fog.z > 0.5 && fog.y > fog.x {
            let d = -dot(globals[8].xyz, inp.world - globals[5].xyz);
            let f = clamp((d - fog.x) / (fog.y - fog.x), 0.0, 1.0);
            out = vec4<f32>(mix(out.rgb, globals[3].rgb, f), out.a);
        }
    }
    // GX colours are gamma-space; the render target is linear.
    if (draw.info.y & 4u) != 0u {
        var g = out.rgb;
        if (draw.info.y & 2u) != 0u { g *= out.a; }
        return vec4<f32>(additive(g), 0.0);
    }
    return vec4<f32>(pow(out.rgb, vec3<f32>(2.2)), out.a);
}
