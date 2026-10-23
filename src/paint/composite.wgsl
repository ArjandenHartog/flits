// flits GPU compositor: every op is one instanced quad.
// Keep the colour math in sync with src/media/colorspace.rs (CPU reference).

struct Uni { size: vec4<f32> };
@group(0) @binding(0) var<uniform> uni: Uni;
// RGBA texture, or the Y plane of a video frame
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var samp: sampler;
// U and V planes of a video frame (same texture as binding 0 otherwise)
@group(1) @binding(2) var tex_u: texture_2d<f32>;
@group(1) @binding(3) var tex_v: texture_2d<f32>;

struct VIn {
    @builtin(vertex_index) vi: u32,
    @location(0) m: vec4<f32>,      // shape-local -> canvas: sx, kx, ky, sy
    @location(1) t: vec4<f32>,      // tx, ty, opacity, radius
    @location(2) shape: vec4<f32>,  // x0, y0, x1, y1 (local)
    @location(3) uvm: vec4<f32>,    // local -> uv: sx, kx, ky, sy
    @location(4) uvt: vec4<f32>,    // tx, ty, mode (0 texture, 1 solid, 2 yuv), px-per-local-unit
    @location(5) color: vec4<f32>,  // premultiplied solid color
    @location(6) cm: vec4<f32>,     // canvas -> clip-local: sx, kx, ky, sy
    @location(7) ct: vec4<f32>,     // tx, ty, has-clip, px-per-clip-unit
    @location(8) cr: vec4<f32>,     // clip rect
    @location(9) vid: vec4<f32>,    // video: transfer (0 sdr, 1 hlg, 2 pq), matrix (0 709, 1 2020, 2 601), full range, sample scale
};

struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) @interpolate(flat) shape: vec4<f32>,
    @location(2) @interpolate(flat) t: vec4<f32>,
    @location(3) @interpolate(flat) uvm: vec4<f32>,
    @location(4) @interpolate(flat) uvt: vec4<f32>,
    @location(5) @interpolate(flat) color: vec4<f32>,
    @location(6) @interpolate(flat) cm: vec4<f32>,
    @location(7) @interpolate(flat) ct: vec4<f32>,
    @location(8) @interpolate(flat) cr: vec4<f32>,
    @location(9) @interpolate(flat) vid: vec4<f32>,
};

@vertex
fn vs(v: VIn) -> VOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0));
    let c = corners[v.vi];
    // grow the quad by ~1.5 px so the anti-aliased edge fits
    let margin = 1.5 / max(v.uvt.w, 1e-4);
    let lo = v.shape.xy - vec2<f32>(margin, margin);
    let hi = v.shape.zw + vec2<f32>(margin, margin);
    let p = lo + (hi - lo) * c;
    let d = vec2<f32>(v.m.x * p.x + v.m.y * p.y + v.t.x, v.m.z * p.x + v.m.w * p.y + v.t.y);
    var o: VOut;
    o.pos = vec4<f32>(d.x / uni.size.x * 2.0 - 1.0, 1.0 - d.y / uni.size.y * 2.0, 0.0, 1.0);
    o.local = p;
    o.shape = v.shape;
    o.t = v.t;
    o.uvm = v.uvm;
    o.uvt = v.uvt;
    o.color = v.color;
    o.cm = v.cm;
    o.ct = v.ct;
    o.cr = v.cr;
    o.vid = v.vid;
    return o;
}

// ---------------------------------------------------------------- video colour

fn hlg_inverse_oetf(e: f32) -> f32 {
    if (e <= 0.5) {
        return e * e / 3.0;
    }
    return (exp((e - 0.55991073) / 0.17883277) + 0.28466892) / 12.0;
}

fn pq_eotf(e: f32) -> f32 {
    let p = pow(max(e, 0.0), 1.0 / 78.84375);
    return 10000.0 * pow(max(p - 0.8359375, 0.0) / (18.8515625 - 18.6875 * p), 1.0 / 0.1593017578125);
}

fn hable(x: f32) -> f32 {
    return (x * (0.15 * x + 0.05) + 0.004) / (x * (0.15 * x + 0.5) + 0.06) - 0.02 / 0.3;
}

fn bt709_oetf(l: f32) -> f32 {
    if (l < 0.018) {
        return 4.5 * l;
    }
    return 1.099 * pow(l, 0.45) - 0.099;
}

// HDR (BT.2020, HLG/PQ) -> SDR BT.709, hable curve, reference white 203 nits
fn hdr_to_sdr(rgb: vec3<f32>, transfer: f32) -> vec3<f32> {
    var nits: vec3<f32>;
    if (transfer < 1.5) {
        let e = vec3<f32>(hlg_inverse_oetf(rgb.r), hlg_inverse_oetf(rgb.g), hlg_inverse_oetf(rgb.b));
        let ys = 0.2627 * e.r + 0.6780 * e.g + 0.0593 * e.b;
        nits = e * (1000.0 * pow(max(ys, 1e-6), 0.2));
    } else {
        nits = vec3<f32>(pq_eotf(rgb.r), pq_eotf(rgb.g), pq_eotf(rgb.b));
    }
    let x = nits / 203.0;
    let l = max(vec3<f32>(
        1.6605 * x.r - 0.5876 * x.g - 0.0728 * x.b,
        -0.1246 * x.r + 1.1329 * x.g - 0.0083 * x.b,
        -0.0182 * x.r - 0.1006 * x.g + 1.1187 * x.b), vec3<f32>(0.0));
    let sig = max(max(max(l.r, l.g), l.b), 1e-6);
    let k = hable(sig) / hable(1000.0 / 203.0) / sig;
    let c = clamp(l * k, vec3<f32>(0.0), vec3<f32>(1.0));
    return vec3<f32>(bt709_oetf(c.r), bt709_oetf(c.g), bt709_oetf(c.b));
}

fn video_rgb(uv: vec2<f32>, vid: vec4<f32>) -> vec3<f32> {
    let s = vid.w;
    var y = textureSampleLevel(tex, samp, uv, 0.0).r * s;
    var cb = textureSampleLevel(tex_u, samp, uv, 0.0).r * s;
    var cr = textureSampleLevel(tex_v, samp, uv, 0.0).r * s;
    if (vid.z > 0.5) {
        cb = cb - 0.5;
        cr = cr - 0.5;
    } else {
        y = (y - 16.0 / 255.0) * (255.0 / 219.0);
        cb = (cb - 128.0 / 255.0) * (255.0 / 224.0);
        cr = (cr - 128.0 / 255.0) * (255.0 / 224.0);
    }
    var kr = 0.2126;
    var kb = 0.0722;
    if (vid.y > 1.5) {
        kr = 0.299;
        kb = 0.114;
    } else if (vid.y > 0.5) {
        kr = 0.2627;
        kb = 0.0593;
    }
    let r = y + 2.0 * (1.0 - kr) * cr;
    let b = y + 2.0 * (1.0 - kb) * cb;
    let g = (y - kr * r - kb * b) / (1.0 - kr - kb);
    let rgb = clamp(vec3<f32>(r, g, b), vec3<f32>(0.0), vec3<f32>(1.0));
    if (vid.x > 0.5) {
        return hdr_to_sdr(rgb, vid.x);
    }
    return rgb;
}

// ---------------------------------------------------------------- fragment

@fragment
fn fs(i: VOut) -> @location(0) vec4<f32> {
    let p = i.local;
    let aa = i.uvt.w;
    // rounded-rect coverage from a signed distance (local units -> pixels)
    let c = (i.shape.xy + i.shape.zw) * 0.5;
    let half = (i.shape.zw - i.shape.xy) * 0.5;
    let r = min(i.t.w, min(half.x, half.y));
    let q = abs(p - c) - half + vec2<f32>(r, r);
    let d = length(max(q, vec2<f32>(0.0, 0.0))) + min(max(q.x, q.y), 0.0) - r;
    var cov = clamp(0.5 - d * aa, 0.0, 1.0);
    if (i.ct.z > 0.5) {
        let s = i.pos.xy;
        let cp = vec2<f32>(i.cm.x * s.x + i.cm.y * s.y + i.ct.x, i.cm.z * s.x + i.cm.w * s.y + i.ct.y);
        let e = min(min(cp.x - i.cr.x, i.cr.z - cp.x), min(cp.y - i.cr.y, i.cr.w - cp.y));
        cov = cov * clamp(e * i.ct.w + 0.5, 0.0, 1.0);
    }
    let uv = vec2<f32>(i.uvm.x * p.x + i.uvm.y * p.y + i.uvt.x, i.uvm.z * p.x + i.uvm.w * p.y + i.uvt.y);
    var col: vec4<f32>;
    if (i.uvt.z > 1.5) {
        col = vec4<f32>(video_rgb(uv, i.vid), 1.0);
    } else if (i.uvt.z > 0.5) {
        col = i.color;
    } else {
        col = textureSampleLevel(tex, samp, uv, 0.0);
    }
    return col * (i.t.z * cov);
}
