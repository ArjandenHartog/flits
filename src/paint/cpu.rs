//! tiny-skia software backend.

use super::{Op, Painter, Src};
use crate::gfx::rounded_rect;
use crate::media::colorspace::frame_to_rgba;
use tiny_skia::{FillRule, FilterQuality, Mask, Paint, PathBuilder, Pattern, Pixmap, PixmapRef, Rect, SpreadMode, Transform};

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
        let rgba: Vec<u8>;
        let mut paint = Paint { anti_alias: true, ..Paint::default() };
        match &op.src {
            Src::Solid(c) => paint.set_color(c.skia(op.opacity)),
            Src::Image(pm) => paint.shader = pattern(pm.as_ref(), op.ts, op.uv, op.opacity),
            Src::Frame { data, w, h, fmt, .. } => {
                rgba = frame_to_rgba(data, *w, *h, fmt);
                match PixmapRef::from_bytes(&rgba, *w, *h) {
                    Some(r) => paint.shader = pattern(r, op.ts, op.uv, op.opacity),
                    None => return,
                }
            }
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
