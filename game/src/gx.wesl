// GX (Wii GPU) fixed-function emulation: texcoord generation, colour channels,
// the TEV combiner stages, alpha test and fog, driven by registers exported
// from the original materials.

import bevy_pbr::render::forward_io::VertexOutput;

struct Tev {
    // x colour env, y alpha env, z order (texmap | coord<<3 | enable<<6 | chan<<7), w kcsel | kasel<<8
    stages: array<vec4<u32>, 16>,
    regs: array<vec4<f32>, 4>,
    konst: array<vec4<f32>, 4>,
    swap: array<vec4<u32>, 4>,
    // two rows per texcoord
    texmtx: array<vec4<f32>, 16>,
    // x source row, y map mode
    texgen: array<vec4<u32>, 8>,
    // x colour ctrl, y alpha ctrl, z stage count, w flags (1 fog, 2 premultiply)
    chan: vec4<u32>,
    mat_color: vec4<f32>,
    amb_color: vec4<f32>,
    // x comp0 | comp1<<4 | op<<8, y ref0, z ref1
    alpha_test: vec4<u32>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> tev: Tev;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var t0: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var s0: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(3) var t1: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(4) var s1: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(5) var t2: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(6) var s2: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(7) var t3: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(8) var s3: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(9) var t4: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(10) var s4: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(11) var t5: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(12) var s5: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(13) var t6: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(14) var s6: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(15) var t7: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(16) var s7: sampler;
// 0 sun dir, 1 sun colour, 2 ambient, 3 fog colour, 4 fog (start, end), 5 camera pos, 6-8 view rotation rows
@group(constants::MATERIAL_BIND_GROUP) @binding(17) var<storage, read> globals: array<vec4<f32>>;

fn sample_map(m: u32, uv: vec2<f32>) -> vec4<f32> {
    switch m {
        case 0u: { return textureSample(t0, s0, uv); }
        case 1u: { return textureSample(t1, s1, uv); }
        case 2u: { return textureSample(t2, s2, uv); }
        case 3u: { return textureSample(t3, s3, uv); }
        case 4u: { return textureSample(t4, s4, uv); }
        case 5u: { return textureSample(t5, s5, uv); }
        case 6u: { return textureSample(t6, s6, uv); }
        default: { return textureSample(t7, s7, uv); }
    }
}

fn swizzle(c: vec4<f32>, t: u32) -> vec4<f32> {
    let s = tev.swap[t];
    return vec4<f32>(c[s.x], c[s.y], c[s.z], c[s.w]);
}

fn konst_value(sel: u32, alpha: bool) -> vec4<f32> {
    if sel <= 7u {
        let v = 1.0 - f32(sel) / 8.0;
        return vec4<f32>(v);
    }
    if sel >= 12u && sel <= 15u && !alpha {
        return tev.konst[sel - 12u];
    }
    if sel >= 16u {
        let k = tev.konst[(sel - 16u) % 4u];
        let comp = (sel - 16u) / 4u;
        return vec4<f32>(k[comp]);
    }
    return vec4<f32>(1.0);
}

fn tev_bias_scale(v: f32, env: u32) -> f32 {
    let bias = (env >> 16u) & 3u;
    let scale = (env >> 20u) & 3u;
    var r = v;
    if bias == 1u { r += 0.5; } else if bias == 2u { r -= 0.5; }
    if scale == 1u { r *= 2.0; } else if scale == 2u { r *= 4.0; } else if scale == 3u { r *= 0.5; }
    return r;
}

fn alpha_cmp(comp: u32, a: f32, r: f32) -> bool {
    switch comp {
        case 0u: { return false; }
        case 1u: { return a < r; }
        case 2u: { return abs(a - r) < 0.002; }
        case 3u: { return a <= r; }
        case 4u: { return a > r; }
        case 5u: { return abs(a - r) >= 0.002; }
        case 6u: { return a >= r; }
        default: { return true; }
    }
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let n = normalize(in.world_normal);
    // View-space normal for sphere environment maps.
    let vn = vec3<f32>(dot(globals[6].xyz, n), dot(globals[7].xyz, n), dot(globals[8].xyz, n));
    var uv0 = vec2<f32>(0.0);
    @if(VERTEX_UVS_A) { uv0 = in.uv; }
    var uv1 = uv0;
    @if(VERTEX_UVS_B) { uv1 = in.uv_b; }
    var vcol = vec4<f32>(1.0);
    @if(VERTEX_COLORS) { vcol = in.color; }

    // Texture coordinates.
    var tc: array<vec2<f32>, 8>;
    for (var i = 0u; i < 8u; i++) {
        let src = tev.texgen[i].x;
        let mode = tev.texgen[i].y;
        var st = uv0;
        if src == 6u { st = uv1; }
        if mode == 1u || mode == 3u || mode == 4u || mode == 2u {
            st = vn.xy * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5);
        }
        let r0 = tev.texmtx[i * 2u];
        let r1 = tev.texmtx[i * 2u + 1u];
        tc[i] = vec2<f32>(dot(r0.xyz, vec3<f32>(st, 1.0)), dot(r1.xyz, vec3<f32>(st, 1.0)));
    }

    // Colour channel 0 (material/vertex colour times lighting).
    let cctrl = tev.chan.x;
    let actrl = tev.chan.y;
    var mat = tev.mat_color;
    if (cctrl & 1u) != 0u { mat = vec4<f32>(vcol.rgb, mat.a); }
    if (actrl & 1u) != 0u { mat.a = vcol.a; }
    var ras = mat;
    if (cctrl & 2u) != 0u {
        let diffuse = max(dot(n, -globals[0].xyz), 0.0);
        let light = clamp(globals[2].rgb + globals[1].rgb * diffuse, vec3<f32>(0.0), vec3<f32>(1.0));
        ras = vec4<f32>(mat.rgb * light, ras.a);
    }

    // TEV stages.
    var r: array<vec4<f32>, 4>;
    r[0] = tev.regs[0];
    r[1] = tev.regs[1];
    r[2] = tev.regs[2];
    r[3] = tev.regs[3];
    let count = max(tev.chan.z, 1u);
    for (var s = 0u; s < count; s++) {
        let st = tev.stages[s];
        let cenv = st.x;
        let aenv = st.y;
        let order = st.z;
        var texc = vec4<f32>(1.0);
        if ((order >> 6u) & 1u) != 0u {
            texc = swizzle(sample_map(order & 7u, tc[(order >> 3u) & 7u]), (aenv >> 2u) & 3u);
        }
        let chan = (order >> 7u) & 7u;
        var rasc = vec4<f32>(0.0);
        if chan != 7u { rasc = swizzle(ras, aenv & 3u); }
        let kc = konst_value(st.w & 31u, false);
        let ka = konst_value((st.w >> 8u) & 31u, true);

        var cin: array<vec3<f32>, 16>;
        cin[0] = r[0].rgb; cin[1] = vec3<f32>(r[0].a);
        cin[2] = r[1].rgb; cin[3] = vec3<f32>(r[1].a);
        cin[4] = r[2].rgb; cin[5] = vec3<f32>(r[2].a);
        cin[6] = r[3].rgb; cin[7] = vec3<f32>(r[3].a);
        cin[8] = texc.rgb; cin[9] = vec3<f32>(texc.a);
        cin[10] = rasc.rgb; cin[11] = vec3<f32>(rasc.a);
        cin[12] = vec3<f32>(1.0); cin[13] = vec3<f32>(0.5);
        cin[14] = kc.rgb; cin[15] = vec3<f32>(0.0);
        var ain: array<f32, 8>;
        ain[0] = r[0].a; ain[1] = r[1].a; ain[2] = r[2].a; ain[3] = r[3].a;
        ain[4] = texc.a; ain[5] = rasc.a; ain[6] = ka.a; ain[7] = 0.0;

        let ca = clamp(cin[(cenv >> 12u) & 15u], vec3<f32>(0.0), vec3<f32>(1.0));
        let cb = clamp(cin[(cenv >> 8u) & 15u], vec3<f32>(0.0), vec3<f32>(1.0));
        let cc = clamp(cin[(cenv >> 4u) & 15u], vec3<f32>(0.0), vec3<f32>(1.0));
        let cd = cin[cenv & 15u];
        var cout: vec3<f32>;
        if ((cenv >> 16u) & 3u) == 3u {
            // Compare mode (only the per-channel variants are approximated).
            let gt = ((cenv >> 18u) & 1u) == 0u;
            let hit = select(abs(ca - cb) < vec3<f32>(0.002), ca > cb, gt);
            cout = cd + select(vec3<f32>(0.0), cc, hit);
        } else {
            let lerp = mix(ca, cb, cc);
            var v = select(cd + lerp, cd - lerp, ((cenv >> 18u) & 1u) == 1u);
            cout = vec3<f32>(tev_bias_scale(v.r, cenv), tev_bias_scale(v.g, cenv), tev_bias_scale(v.b, cenv));
        }
        if ((cenv >> 19u) & 1u) == 1u { cout = clamp(cout, vec3<f32>(0.0), vec3<f32>(1.0)); }

        let aa = clamp(ain[(aenv >> 13u) & 7u], 0.0, 1.0);
        let ab = clamp(ain[(aenv >> 10u) & 7u], 0.0, 1.0);
        let ac = clamp(ain[(aenv >> 7u) & 7u], 0.0, 1.0);
        let ad = ain[(aenv >> 4u) & 7u];
        var aout: f32;
        if ((aenv >> 16u) & 3u) == 3u {
            let gt = ((aenv >> 18u) & 1u) == 0u;
            let hit = select(abs(aa - ab) < 0.002, aa > ab, gt);
            aout = ad + select(0.0, ac, hit);
        } else {
            let lerp = mix(aa, ab, ac);
            aout = tev_bias_scale(select(ad + lerp, ad - lerp, ((aenv >> 18u) & 1u) == 1u), aenv);
        }
        if ((aenv >> 19u) & 1u) == 1u { aout = clamp(aout, 0.0, 1.0); }

        let cdst = (cenv >> 22u) & 3u;
        let adst = (aenv >> 22u) & 3u;
        r[cdst] = vec4<f32>(cout, r[cdst].a);
        r[adst].a = aout;
    }
    var out = clamp(r[0], vec4<f32>(0.0), vec4<f32>(1.0));

    // Alpha test.
    let at = tev.alpha_test;
    let p0 = alpha_cmp(at.x & 7u, out.a, f32(at.y) / 255.0);
    let p1 = alpha_cmp((at.x >> 4u) & 7u, out.a, f32(at.z) / 255.0);
    let op = (at.x >> 8u) & 3u;
    var keep = p0 && p1;
    if op == 1u { keep = p0 || p1; } else if op == 2u { keep = p0 != p1; } else if op == 3u { keep = p0 == p1; }
    if !keep { discard; }

    // Fog selected by the material (GX fog index).
    // ponytail: every non-zero GX fog type is treated as linear (the island only uses linear).
    let second = ((tev.chan.w >> 8u) & 1u) == 1u;
    let fog_color = select(globals[3], globals[9], second);
    let fog = select(globals[4], globals[10], second);
    if (tev.chan.w & 1u) != 0u && fog.z > 0.5 && fog.y > fog.x {
        // GX perspective fog uses eye-space depth, not radial distance.
        let d = -dot(globals[8].xyz, in.world_position.xyz - globals[5].xyz);
        let f = clamp((d - fog.x) / (fog.y - fog.x), 0.0, 1.0);
        out = vec4<f32>(mix(out.rgb, fog_color.rgb, f), out.a);
    }
    // GX colours are gamma-space; the render target is linear. GX also adds in
    // gamma space, which the linear target would make far too faint over a
    // bright sky: add what gamma-space addition would over the fog/sky colour.
    if (tev.chan.w & 4u) != 0u {
        var g = out.rgb;
        if (tev.chan.w & 2u) != 0u { g *= out.a; }
        let d = globals[3].rgb;
        return vec4<f32>(max(pow(g + d, vec3<f32>(2.2)) - pow(d, vec3<f32>(2.2)), vec3<f32>(0.0)), 0.0);
    }
    out = vec4<f32>(pow(out.rgb, vec3<f32>(2.2)), out.a);
    if (tev.chan.w & 2u) != 0u { out = vec4<f32>(out.rgb * out.a, out.a); }
    return out;
}
