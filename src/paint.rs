//! The drawing interface between the scene tree and a backend (CPU or GPU).
//! Every visual is reduced to one primitive: a (rounded) rectangle filled with a
//! solid color or an affinely mapped bitmap.

use crate::color::Rgba;
use crate::effects::{rounded_rect, Sprite};
use tiny_skia::{FillRule, FilterQuality, Mask, Paint, PathBuilder, Pattern, Pixmap, PixmapRef, Rect, SpreadMode, Transform};

pub enum Src<'a> {
    Solid(Rgba),
    /// static bitmap; its address is used as a cache key by the GPU backend
    Image(&'a Pixmap),
    /// a decoded video frame (opaque RGBA), `ver` changes whenever the content does
    Frame { key: usize, ver: i64, data: &'a [u8], w: u32, h: u32 },
}

#[derive(Clone, Copy)]
pub struct Clip {
    /// clip-local -> canvas pixels
    pub ts: Transform,
    pub rect: [f32; 4],
}

pub struct Op<'a> {
    pub src: Src<'a>,
    /// shape-local -> canvas pixels
    pub ts: Transform,
    /// x0, y0, x1, y1 in shape-local units
    pub shape: [f32; 4],
    pub radius: f32,
    /// source pixels -> shape-local
    pub uv: Transform,
    pub opacity: f32,
    pub clip: Option<Clip>,
}

pub trait Painter<'a> {
    fn draw(&mut self, op: Op<'a>);
}

/// Draw a pre-rendered sprite so its logical box lands at local (0,0).
pub fn sprite_op<'a>(s: &'a Sprite, ts: Transform, opacity: f32, clip: Option<Clip>) -> Op<'a> {
    let (pw, ph) = (s.pm.width() as f32, s.pm.height() as f32);
    let p = s.ppu;
    Op {
        src: Src::Image(&s.pm),
        ts,
        shape: [-s.ox / p, -s.oy / p, (pw - s.ox) / p, (ph - s.oy) / p],
        radius: 0.0,
        uv: Transform::from_row(1.0 / p, 0.0, 0.0, 1.0 / p, -s.ox / p, -s.oy / p),
        opacity,
        clip,
    }
}

/// tiny-skia software backend.
pub struct CpuPainter<'a> {
    pub canvas: &'a mut Pixmap,
}

impl<'a> Painter<'a> for CpuPainter<'_> {
    fn draw(&mut self, op: Op<'a>) {
        let [x0, y0, x1, y1] = op.shape;
        if x1 <= x0 || y1 <= y0 || op.opacity <= 0.0 {
            return;
        }
        let mask = op.clip.and_then(|c| {
            let r = Rect::from_ltrb(c.rect[0], c.rect[1], c.rect[2].max(c.rect[0] + 0.01), c.rect[3].max(c.rect[1] + 0.01))?;
            let mut m = Mask::new(self.canvas.width(), self.canvas.height())?;
            m.fill_path(&PathBuilder::from_rect(r), FillRule::Winding, true, c.ts);
            Some(m)
        });
        let mut paint = Paint { anti_alias: true, ..Paint::default() };
        match &op.src {
            Src::Solid(c) => paint.set_color(c.skia(op.opacity)),
            Src::Image(pm) => paint.shader = pattern(pm.as_ref(), op.ts, op.uv, op.opacity),
            Src::Frame { data, w, h, .. } => match PixmapRef::from_bytes(data, *w, *h) {
                Some(r) => paint.shader = pattern(r, op.ts, op.uv, op.opacity),
                None => return,
            },
        }
        if op.radius > 0.0 {
            if let Some(path) = rounded_rect(x0, y0, x1 - x0, y1 - y0, op.radius) {
                self.canvas.fill_path(&path, &paint, FillRule::Winding, op.ts, mask.as_ref());
            }
        } else if let Some(r) = Rect::from_ltrb(x0, y0, x1, y1) {
            self.canvas.fill_rect(r, &paint, op.ts, mask.as_ref());
        }
    }
}

fn pattern<'a>(img: PixmapRef<'a>, ts: Transform, uv: Transform, opacity: f32) -> tiny_skia::Shader<'a> {
    let total = ts.pre_concat(uv);
    let one_to_one = (total.sx - 1.0).abs() < 1e-3 && (total.sy - 1.0).abs() < 1e-3 && total.kx.abs() < 1e-6 && total.ky.abs() < 1e-6;
    let q = if one_to_one { FilterQuality::Nearest } else { FilterQuality::Bilinear };
    Pattern::new(img, SpreadMode::Pad, q, opacity, uv)
}
