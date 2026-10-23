// RGBA -> YUV 4:2:0 (BT.709 limited)

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var<storage, read_write> outb: array<u32>;
@group(0) @binding(2) var<uniform> dims: vec4<u32>;

fn rgb(x: u32, y: u32) -> vec3<f32> {
    return textureLoad(src, vec2<i32>(i32(x), i32(y)), 0).rgb * 255.0;
}

fn luma(c: vec3<f32>) -> u32 {
    return u32(clamp(floor((47.0 * c.r + 157.0 * c.g + 16.0 * c.b + 128.0) / 256.0) + 16.0, 0.0, 255.0));
}

@compute @workgroup_size(16, 8)
fn yuv(@builtin(global_invocation_id) id: vec3<u32>) {
    let w = dims.x;
    let h = dims.y;
    let x0 = id.x * 8u;
    let y0 = id.y * 2u;
    if (x0 >= w || y0 >= h) {
        return;
    }
    for (var row = 0u; row < 2u; row = row + 1u) {
        var lo = 0u;
        var hi = 0u;
        for (var k = 0u; k < 4u; k = k + 1u) {
            lo = lo | (luma(rgb(x0 + k, y0 + row)) << (8u * k));
            hi = hi | (luma(rgb(x0 + 4u + k, y0 + row)) << (8u * k));
        }
        let base = ((y0 + row) * w + x0) / 4u;
        outb[base] = lo;
        outb[base + 1u] = hi;
    }
    var uw = 0u;
    var vw = 0u;
    for (var j = 0u; j < 4u; j = j + 1u) {
        let a = rgb(x0 + 2u * j, y0) + rgb(x0 + 2u * j + 1u, y0) + rgb(x0 + 2u * j, y0 + 1u) + rgb(x0 + 2u * j + 1u, y0 + 1u);
        let u = u32(clamp(floor((-26.0 * a.r - 87.0 * a.g + 113.0 * a.b + 512.0) / 1024.0) + 128.0, 0.0, 255.0));
        let v = u32(clamp(floor((112.0 * a.r - 102.0 * a.g - 10.0 * a.b + 512.0) / 1024.0) + 128.0, 0.0, 255.0));
        uw = uw | (u << (8u * j));
        vw = vw | (v << (8u * j));
    }
    let cw = w / 2u;
    let ch = h / 2u;
    let ci = (id.y * cw + x0 / 2u) / 4u;
    outb[(w * h) / 4u + ci] = uw;
    outb[(w * h + cw * ch) / 4u + ci] = vw;
}
