//! Colour glyphs (emoji): COLR v0/v1 layers painted with tiny-skia, and CBDT/sbix bitmap strikes.
//!
//! Text is rasterized once per layer (not per frame), so the emoji end up in the same sprite as the
//! rest of the text and draw identically on the GPU and CPU compositors.

use crate::scene::color::Rgba;
use tiny_skia::{
    BlendMode, Color, FillRule, FilterQuality, GradientStop, Mask, Paint, Path, PathBuilder, Pixmap, PixmapPaint, Point, Shader,
    SpreadMode, Transform,
};
use ttf_parser::colr::{self, CompositeMode, GradientExtend};
use ttf_parser::{GlyphId, RasterImageFormat, RgbaColor};

/// Whether `id` has a colour representation in `face` (COLR layers or a bitmap strike).
pub fn has_color(face: &ttf_parser::Face, id: u16) -> bool {
    let id = GlyphId(id);
    face.is_color_glyph(id) || face.glyph_raster_image(id, u16::MAX).is_some()
}

/// Draw colour glyph `id` with its origin at (`x`, `baseline`) and an em of `size` px.
/// Returns false when the glyph has no colour data (the caller draws the outline instead).
pub fn draw(pm: &mut Pixmap, face: &ttf_parser::Face, id: u16, x: f32, baseline: f32, size: f32, fg: Rgba) -> bool {
    let gid = GlyphId(id);
    if face.is_color_glyph(gid) {
        // paint into a glyph-sized scratch pixmap (layers and masks stay small), then copy it in
        let k = size / face.units_per_em() as f32;
        let adv = face.glyph_hor_advance(gid).unwrap_or(face.units_per_em()) as f32 * k;
        let (left, top) = ((x - 0.5 * size).floor(), (baseline - 1.5 * size).floor());
        let (w, h) = ((adv + size).ceil().max(1.0) as u32, (2.5 * size).ceil().max(1.0) as u32);
        // font units (y up) -> scratch pixels (y down)
        let base = Transform::from_row(k, 0.0, 0.0, -k, x - left, baseline - top);
        let mut p = ColrPainter::new(face, w, h, base);
        let c = fg.skia(1.0).to_color_u8();
        let fg = RgbaColor::new(c.red(), c.green(), c.blue(), c.alpha());
        if face.paint_color_glyph(gid, 0, fg, &mut p).is_some() {
            if let Some(layer) = p.layers.pop() {
                pm.draw_pixmap(left as i32, top as i32, layer.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
            }
            return true;
        }
    }
    draw_bitmap(pm, face, gid, x, baseline, size)
}

/// CBDT/sbix: decode the strike closest to `size` and scale it into place.
fn draw_bitmap(pm: &mut Pixmap, face: &ttf_parser::Face, id: GlyphId, x: f32, baseline: f32, size: f32) -> bool {
    let Some(img) = face.glyph_raster_image(id, size.ceil().clamp(1.0, u16::MAX as f32) as u16) else { return false };
    let Some(bitmap) = decode(&img) else { return false };
    let s = size / img.pixels_per_em.max(1) as f32;
    // (x, y) = bottom-left corner of the image relative to the glyph origin, y up, in strike pixels
    let top = baseline - (img.y as f32 + img.height as f32) * s;
    let left = x + img.x as f32 * s;
    let paint = PixmapPaint { quality: FilterQuality::Bicubic, ..Default::default() };
    pm.draw_pixmap(0, 0, bitmap.as_ref(), &paint, Transform::from_row(s, 0.0, 0.0, s, left, top), None);
    true
}

fn decode(img: &ttf_parser::RasterGlyphImage) -> Option<Pixmap> {
    match img.format {
        RasterImageFormat::PNG => {
            let rgba = image::load_from_memory_with_format(img.data, image::ImageFormat::Png).ok()?.to_rgba8();
            let mut pm = Pixmap::new(rgba.width(), rgba.height())?;
            for (dst, src) in pm.pixels_mut().iter_mut().zip(rgba.pixels()) {
                let [r, g, b, a] = src.0;
                *dst = tiny_skia::ColorU8::from_rgba(r, g, b, a).premultiply();
            }
            Some(pm)
        }
        RasterImageFormat::BitmapPremulBgra32 => {
            let mut pm = Pixmap::new(img.width as u32, img.height as u32)?;
            for (dst, &[b, g, r, a]) in pm.pixels_mut().iter_mut().zip(img.data.as_chunks::<4>().0) {
                *dst = tiny_skia::PremultipliedColorU8::from_rgba(r.min(a), g.min(a), b.min(a), a)?;
            }
            Some(pm)
        }
        // monochrome/grey strikes: the outline (drawn by the caller) looks better
        _ => None,
    }
}

/// Paints COLR glyphs: a stack of layers (for composite modes), clip masks and transforms.
/// Coordinates are kept in pixels: outlines are transformed when they are read, gradients
/// get the transform that is current when they are painted.
struct ColrPainter<'a, 'f> {
    face: &'f ttf_parser::Face<'a>,
    w: u32,
    h: u32,
    transforms: Vec<Transform>,
    outline: Option<Path>,
    clips: Vec<Mask>,
    layers: Vec<Pixmap>,
    modes: Vec<CompositeMode>,
}

impl<'a, 'f> ColrPainter<'a, 'f> {
    fn new(face: &'f ttf_parser::Face<'a>, w: u32, h: u32, base: Transform) -> Self {
        let root = Pixmap::new(w, h).expect("text pixmap has a size");
        ColrPainter { face, w, h, transforms: vec![base], outline: None, clips: vec![], layers: vec![root], modes: vec![] }
    }

    fn ts(&self) -> Transform {
        *self.transforms.last().unwrap()
    }

    fn fill(&mut self, shader: Shader) {
        let paint = Paint { shader, anti_alias: true, ..Default::default() };
        let mask = self.clips.last();
        let layer = self.layers.last_mut().unwrap();
        match &self.outline {
            Some(path) => layer.fill_path(path, &paint, FillRule::Winding, Transform::identity(), mask),
            None => {
                let full = tiny_skia::Rect::from_xywh(0.0, 0.0, self.w as f32, self.h as f32).unwrap();
                layer.fill_rect(full, &paint, Transform::identity(), mask);
            }
        }
    }
}

/// Gradient stops sorted by offset, with offsets mapped through `f`.
fn stops(raw: impl Iterator<Item = colr::ColorStop>, f: impl Fn(f32) -> f32) -> Vec<GradientStop> {
    let mut stops: Vec<_> = raw.collect();
    stops.sort_by(|a, b| a.stop_offset.total_cmp(&b.stop_offset));
    stops.iter().map(|s| GradientStop::new(f(s.stop_offset), color(s.color))).collect()
}

fn color(c: RgbaColor) -> Color {
    Color::from_rgba8(c.red, c.green, c.blue, c.alpha)
}

fn spread(e: GradientExtend) -> SpreadMode {
    match e {
        GradientExtend::Pad => SpreadMode::Pad,
        GradientExtend::Repeat => SpreadMode::Repeat,
        GradientExtend::Reflect => SpreadMode::Reflect,
    }
}

fn blend(m: CompositeMode) -> BlendMode {
    use CompositeMode as C;
    match m {
        C::Clear => BlendMode::Clear,
        C::Source => BlendMode::Source,
        C::Destination => BlendMode::Destination,
        C::SourceOver => BlendMode::SourceOver,
        C::DestinationOver => BlendMode::DestinationOver,
        C::SourceIn => BlendMode::SourceIn,
        C::DestinationIn => BlendMode::DestinationIn,
        C::SourceOut => BlendMode::SourceOut,
        C::DestinationOut => BlendMode::DestinationOut,
        C::SourceAtop => BlendMode::SourceAtop,
        C::DestinationAtop => BlendMode::DestinationAtop,
        C::Xor => BlendMode::Xor,
        C::Plus => BlendMode::Plus,
        C::Screen => BlendMode::Screen,
        C::Overlay => BlendMode::Overlay,
        C::Darken => BlendMode::Darken,
        C::Lighten => BlendMode::Lighten,
        C::ColorDodge => BlendMode::ColorDodge,
        C::ColorBurn => BlendMode::ColorBurn,
        C::HardLight => BlendMode::HardLight,
        C::SoftLight => BlendMode::SoftLight,
        C::Difference => BlendMode::Difference,
        C::Exclusion => BlendMode::Exclusion,
        C::Multiply => BlendMode::Multiply,
        C::Hue => BlendMode::Hue,
        C::Saturation => BlendMode::Saturation,
        C::Color => BlendMode::Color,
        C::Luminosity => BlendMode::Luminosity,
    }
}

/// Collects a glyph outline into a tiny-skia path.
struct Outline(PathBuilder);

impl ttf_parser::OutlineBuilder for Outline {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to(x, y)
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.line_to(x, y)
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.0.quad_to(x1, y1, x, y)
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.0.cubic_to(x1, y1, x2, y2, x, y)
    }
    fn close(&mut self) {
        self.0.close()
    }
}

impl<'a> colr::Painter<'a> for ColrPainter<'a, '_> {
    fn outline_glyph(&mut self, glyph_id: GlyphId) {
        let mut b = Outline(PathBuilder::new());
        self.outline = self.face.outline_glyph(glyph_id, &mut b).and_then(|_| b.0.finish()).and_then(|p| p.transform(self.ts()));
    }

    fn paint(&mut self, paint: colr::Paint<'a>) {
        let ts = self.ts();
        let coords = self.face.variation_coordinates();
        let shader = match paint {
            colr::Paint::Solid(c) => Shader::SolidColor(color(c)),
            colr::Paint::LinearGradient(g) => {
                // COLR: the gradient runs from p0 towards p1, its stripes parallel to p0->p2;
                // project p1 onto the normal of p0->p2 for the equivalent two-point gradient
                let (d1x, d1y, d2x, d2y) = (g.x1 - g.x0, g.y1 - g.y0, g.x2 - g.x0, g.y2 - g.y0);
                let (nx, ny) = (d2y, -d2x);
                let nn = nx * nx + ny * ny;
                let (ex, ey) = if nn > 1e-6 {
                    let t = (d1x * nx + d1y * ny) / nn;
                    (g.x0 + nx * t, g.y0 + ny * t)
                } else {
                    (g.x1, g.y1)
                };
                let stops = stops(g.stops(0, coords), |o| o);
                tiny_skia::LinearGradient::new(Point::from_xy(g.x0, g.y0), Point::from_xy(ex, ey), stops, spread(g.extend), ts)
                    .unwrap_or(Shader::SolidColor(Color::TRANSPARENT))
            }
            colr::Paint::RadialGradient(g) => {
                // tiny-skia draws focal gradients (start radius 0): map the stops from r0..r1 onto 0..r1
                let f = if g.r1 > 0.0 { (g.r0 / g.r1).clamp(0.0, 1.0) } else { 0.0 };
                let stops = stops(g.stops(0, coords), |o| f + o * (1.0 - f));
                tiny_skia::RadialGradient::new(
                    Point::from_xy(g.x0, g.y0),
                    Point::from_xy(g.x1, g.y1),
                    g.r1.max(0.01),
                    stops,
                    spread(g.extend),
                    ts,
                )
                .unwrap_or(Shader::SolidColor(Color::TRANSPARENT))
            }
            colr::Paint::SweepGradient(g) => {
                // no sweep gradients in tiny-skia: the average colour is a close stand-in at emoji sizes
                let stops: Vec<_> = g.stops(0, coords).collect();
                let n = stops.len().max(1) as f32;
                let avg = |f: fn(&RgbaColor) -> u8| (stops.iter().map(|s| f(&s.color) as f32).sum::<f32>() / n).round() as u8;
                Shader::SolidColor(Color::from_rgba8(avg(|c| c.red), avg(|c| c.green), avg(|c| c.blue), avg(|c| c.alpha)))
            }
        };
        self.fill(shader);
    }

    fn push_clip(&mut self) {
        let mut mask = match self.clips.last() {
            Some(m) => m.clone(),
            None => {
                let mut m = Mask::new(self.w, self.h).expect("mask size");
                m.data_mut().fill(255);
                m
            }
        };
        match &self.outline {
            Some(p) => mask.intersect_path(p, FillRule::Winding, true, Transform::identity()),
            None => mask.data_mut().fill(0),
        }
        self.clips.push(mask);
    }

    fn push_clip_box(&mut self, b: colr::ClipBox) {
        let rect = tiny_skia::Rect::from_ltrb(b.x_min, b.y_min, b.x_max, b.y_max);
        self.outline = rect.map(PathBuilder::from_rect).and_then(|p| p.transform(self.ts()));
        self.push_clip();
    }

    fn pop_clip(&mut self) {
        self.clips.pop();
    }

    fn push_layer(&mut self, mode: CompositeMode) {
        self.layers.push(Pixmap::new(self.w, self.h).expect("layer size"));
        self.modes.push(mode);
    }

    fn pop_layer(&mut self) {
        if self.layers.len() < 2 {
            return;
        }
        let top = self.layers.pop().unwrap();
        let mode = self.modes.pop().unwrap_or(CompositeMode::SourceOver);
        let paint = PixmapPaint { blend_mode: blend(mode), ..Default::default() };
        self.layers.last_mut().unwrap().draw_pixmap(0, 0, top.as_ref(), &paint, Transform::identity(), None);
    }

    fn push_transform(&mut self, t: ttf_parser::Transform) {
        let next = self.ts().pre_concat(Transform::from_row(t.a, t.b, t.c, t.d, t.e, t.f));
        self.transforms.push(next);
    }

    fn pop_transform(&mut self) {
        if self.transforms.len() > 1 {
            self.transforms.pop();
        }
    }
}
