//! CPU colour conversion for the software path.

/// RGBA (opaque) -> planar YUV 4:2:0, BT.709 limited range.
pub fn rgba_to_yuv420(src: &[u8], w: usize, h: usize, out: &mut Vec<u8>) {
    let (cw, ch) = (w / 2, h / 2);
    out.resize(w * h + 2 * cw * ch, 0);
    let (yp, uv) = out.split_at_mut(w * h);
    let (up, vp) = uv.split_at_mut(cw * ch);
    for y in 0..h {
        let row = &src[y * w * 4..(y + 1) * w * 4];
        let yrow = &mut yp[y * w..(y + 1) * w];
        for x in 0..w {
            let (r, g, b) = (row[x * 4] as i32, row[x * 4 + 1] as i32, row[x * 4 + 2] as i32);
            yrow[x] = (((47 * r + 157 * g + 16 * b + 128) >> 8) + 16) as u8;
        }
    }
    for cy in 0..ch {
        let r0 = &src[(cy * 2) * w * 4..(cy * 2 + 1) * w * 4];
        let r1 = &src[(cy * 2 + 1) * w * 4..(cy * 2 + 2) * w * 4];
        for cx in 0..cw {
            let i = cx * 8;
            let r = r0[i] as i32 + r0[i + 4] as i32 + r1[i] as i32 + r1[i + 4] as i32;
            let g = r0[i + 1] as i32 + r0[i + 5] as i32 + r1[i + 1] as i32 + r1[i + 5] as i32;
            let b = r0[i + 2] as i32 + r0[i + 6] as i32 + r1[i + 2] as i32 + r1[i + 6] as i32;
            up[cy * cw + cx] = (((-26 * r - 87 * g + 113 * b + 512) >> 10) + 128).clamp(0, 255) as u8;
            vp[cy * cw + cx] = (((112 * r - 102 * g - 10 * b + 512) >> 10) + 128).clamp(0, 255) as u8;
        }
    }
}
