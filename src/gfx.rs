//! Bitmap helpers done once at prepare time: sprites, blur, shadows, rounded shapes.

use crate::scene::color::Rgba;
use rayon::prelude::*;
use tiny_skia::{FillRule, Paint, Path, PathBuilder, Pixmap, PixmapPaint, Rect, Transform};

/// Pre-rendered bitmap plus where its logical box sits inside it.
pub struct Sprite {
    pub pm: Pixmap,
    /// position of the logical box's top-left corner inside `pm`
    pub ox: f32,
    pub oy: f32,
    /// logical box size (what anchors/positions refer to)
    pub w: f32,
    pub h: f32,
    /// bitmap pixels per logical unit (1.0 normally; <1 for low-res blurry sprites)
    pub ppu: f32,
}

impl Sprite {
    /// Transform that maps the sprite bitmap so its box lands at local (0,0).
    pub fn place(&self, ts: tiny_skia::Transform) -> tiny_skia::Transform {
        ts.pre_scale(1.0 / self.ppu, 1.0 / self.ppu).pre_translate(-self.ox, -self.oy)
    }
}

pub fn rounded_rect(x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<Path> {
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    if r <= 0.01 {
        return Some(PathBuilder::from_rect(Rect::from_xywh(x, y, w.max(0.01), h.max(0.01))?));
    }
    let k = 0.552_284_8 * r;
    let mut pb = PathBuilder::new();
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.cubic_to(x + w - r + k, y, x + w, y + r - k, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.cubic_to(x + w, y + h - r + k, x + w - r + k, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.cubic_to(x + r - k, y + h, x, y + h - r + k, x, y + h - r);
    pb.line_to(x, y + r);
    pb.cubic_to(x, y + r - k, x + r - k, y, x + r, y);
    pb.close();
    pb.finish()
}

/// One horizontal box-blur pass over premultiplied RGBA rows.
fn blur_rows(data: &mut [u8], w: usize, h: usize, r: usize) {
    if r == 0 || w == 0 {
        return;
    }
    data.par_chunks_mut(w * 4).take(h).for_each(|row| {
        let src: Vec<u8> = row.to_vec();
        let div = (2 * r + 1) as u32;
        for c in 0..4 {
            let px = |i: isize| -> u32 { src[(i.clamp(0, w as isize - 1) as usize) * 4 + c] as u32 };
            let mut sum: u32 = 0;
            for i in -(r as isize)..=(r as isize) {
                sum += px(i);
            }
            for x in 0..w {
                row[x * 4 + c] = ((sum + div / 2) / div) as u8;
                sum += px(x as isize + r as isize + 1);
                sum -= px(x as isize - r as isize);
            }
        }
    });
}

fn transpose(data: &[u8], w: usize, h: usize) -> Vec<u8> {
    let mut out = vec![0u8; data.len()];
    out.par_chunks_mut(h * 4).enumerate().for_each(|(x, col)| {
        for y in 0..h {
            let s = (y * w + x) * 4;
            col[y * 4..y * 4 + 4].copy_from_slice(&data[s..s + 4]);
        }
    });
    out
}

/// Approximate gaussian blur (3 box passes) with standard deviation `sigma` px.
pub fn blur(pm: &mut Pixmap, sigma: f32) {
    if sigma < 0.5 {
        return;
    }
    let (w, h) = (pm.width() as usize, pm.height() as usize);
    // box radius for 3 passes ~ sigma
    let r = ((((4.0 * sigma * sigma + 1.0).sqrt()) - 1.0) / 2.0).round().max(1.0) as usize;
    let data = pm.data_mut();
    for _ in 0..3 {
        blur_rows(data, w, h, r);
    }
    let mut t = transpose(data, w, h);
    for _ in 0..3 {
        blur_rows(&mut t, h, w, r);
    }
    let back = transpose(&t, h, w);
    data.copy_from_slice(&back);
}

/// Replace color with `color`, keeping (scaled) alpha. Output is premultiplied.
pub fn tint(pm: &mut Pixmap, color: Rgba) {
    let (r, g, b, a) = (color.r, color.g, color.b, color.a);
    for px in pm.data_mut().as_chunks_mut::<4>().0 {
        let al = px[3] as f32 / 255.0 * a;
        px[0] = (r * al * 255.0 + 0.5) as u8;
        px[1] = (g * al * 255.0 + 0.5) as u8;
        px[2] = (b * al * 255.0 + 0.5) as u8;
        px[3] = (al * 255.0 + 0.5) as u8;
    }
}

/// Add a drop shadow under a sprite (grows the bitmap as needed).
pub fn with_shadow(s: Sprite, color: Rgba, sigma: f32, dx: f32, dy: f32) -> Sprite {
    let pad = (sigma * 3.0).ceil() + 2.0;
    let left = (pad - dx).max(0.0).ceil();
    let top = (pad - dy).max(0.0).ceil();
    let right = (pad + dx).max(0.0).ceil();
    let bottom = (pad + dy).max(0.0).ceil();
    let nw = s.pm.width() + (left + right) as u32;
    let nh = s.pm.height() + (top + bottom) as u32;
    let mut shadow = Pixmap::new(nw, nh).unwrap();
    shadow.draw_pixmap((left + dx) as i32, (top + dy) as i32, s.pm.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
    tint(&mut shadow, color);
    blur(&mut shadow, sigma);
    shadow.draw_pixmap(left as i32, top as i32, s.pm.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
    Sprite { pm: shadow, ox: s.ox + left, oy: s.oy + top, w: s.w, h: s.h, ppu: s.ppu }
}

/// Blurred silhouette of a rounded box, used as a drop shadow for media layers.
pub fn box_shadow(w: f32, h: f32, radius: f32, color: Rgba, sigma: f32, dx: f32, dy: f32) -> Option<Sprite> {
    // draw the silhouette at reduced resolution when the blur is big (it's blurry anyway)
    let pad = (sigma * 3.0).ceil() + 2.0;
    let down = (sigma / 6.0).clamp(1.0, 8.0);
    let pw = ((w + pad * 2.0) / down).ceil() as u32;
    let ph = ((h + pad * 2.0) / down).ceil() as u32;
    let mut pm = Pixmap::new(pw.max(1), ph.max(1))?;
    let path = rounded_rect(pad / down, pad / down, w / down, h / down, radius / down)?;
    let mut paint = Paint::default();
    paint.set_color(color.skia(1.0));
    paint.anti_alias = true;
    pm.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
    blur(&mut pm, sigma / down);
    Some(Sprite { pm, ox: (pad - dx) / down, oy: (pad - dy) / down, w, h, ppu: 1.0 / down })
}
