// flits GPU compositor: every op is one instanced quad.

struct Uni { size: vec4<f32> };
@group(0) @binding(0) var<uniform> uni: Uni;
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var samp: sampler;

struct VIn {
    @builtin(vertex_index) vi: u32,
    @location(0) m: vec4<f32>,      // shape-local -> canvas: sx, kx, ky, sy
    @location(1) t: vec4<f32>,      // tx, ty, opacity, radius
    @location(2) shape: vec4<f32>,  // x0, y0, x1, y1 (local)
    @location(3) uvm: vec4<f32>,    // local -> uv: sx, kx, ky, sy
    @location(4) uvt: vec4<f32>,    // tx, ty, solid?, px-per-local-unit
    @location(5) color: vec4<f32>,  // premultiplied solid color
    @location(6) cm: vec4<f32>,     // canvas -> clip-local: sx, kx, ky, sy
    @location(7) ct: vec4<f32>,     // tx, ty, has-clip, px-per-clip-unit
    @location(8) cr: vec4<f32>,     // clip rect
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
};

@vertex
fn vs(v: VIn) -> VOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0));
    let c = corners[v.vi];
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
    return o;
}

@fragment
fn fs(i: VOut) -> @location(0) vec4<f32> {
    let p = i.local;
    let aa = i.uvt.w;
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
    let texel = textureSampleLevel(tex, samp, uv, 0.0);
    let col = select(texel, i.color, i.uvt.z > 0.5);
    return col * (i.t.z * cov);
}
