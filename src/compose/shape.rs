//! `shape` layers: rectangles, ellipses, lines, arrows, polygons, stars and SVG paths, filled and/or
//! stroked, with an animatable `progress` that draws the outline on (like a trim path).
//!
//! Shapes are rasterized at prepare time like text. An animated `progress` is rendered once per
//! distinct value it takes during the layer (quantized, with a memory cap), so a frame only picks a
//! sprite and both backends draw the same pixels.

use super::Fill;
use crate::gfx::{rounded_rect, Sprite};
use crate::scene::anim::Prop;
use crate::scene::color::Rgba;
use crate::scene::Layer;
use anyhow::{anyhow, bail, Context, Result};
use std::sync::Arc;
use tiny_skia::{
    FillRule, GradientStop, LineCap, LineJoin, LinearGradient, Paint, Path, PathBuilder, PathSegment, Pixmap, Point, RadialGradient,
    Shader, SpreadMode, Stroke, StrokeDash, Transform,
};

pub const SHAPES: &[&str] = &["rect", "ellipse", "circle", "line", "arrow", "polygon", "star", "path"];
pub const LINE_CAPS: &[&str] = &["round", "butt", "square"];

/// Memory budget for the pre-rendered `progress` steps of one layer.
const PROGRESS_BUDGET: usize = 160 << 20;

pub struct Shape {
    /// (progress, sprite), sorted by progress; a single entry when progress doesn't change
    pub steps: Vec<(f32, Arc<Sprite>)>,
    pub progress: Option<Prop>,
}

impl Shape {
    /// The sprite for layer time `lt` of a layer lasting `dur`.
    pub fn sprite(&self, lt: f64, dur: f64) -> &Arc<Sprite> {
        let p = self.progress.as_ref().map(|p| p.eval(lt, dur).clamp(0.0, 1.0) as f32).unwrap_or(1.0);
        let i = self.steps.partition_point(|(v, _)| *v < p);
        let i = match (i.checked_sub(1), self.steps.get(i)) {
            (Some(lo), Some((hi_v, _))) if p - self.steps[lo].0 < hi_v - p => lo,
            (Some(lo), None) => lo,
            _ => i,
        };
        &self.steps[i].1
    }
}

/// What to draw, in layer-box units.
struct Geometry {
    path: Path,
    closed: bool,
    /// arrow head length (open shapes ending in an arrow)
    head: Option<f32>,
}

struct Style {
    fill: Option<Fill>,
    stroke: Option<(f32, Rgba)>,
    cap: LineCap,
    dash: Option<Vec<f32>>,
}

/// Build a shape layer of `w` x `h` units, rasterized at `q` pixels per unit. `times`: (layer time,
/// layer duration) of every frame, to find the `progress` values the layer shows.
pub fn build(l: &Layer, w: f32, h: f32, q: f32, fill: Option<Fill>, times: impl Iterator<Item = (f64, f64)>) -> Result<Shape> {
    let geo = geometry(l, w, h)?;
    let cap = match l.line_cap.as_deref().unwrap_or("round") {
        "round" => LineCap::Round,
        "butt" => LineCap::Butt,
        "square" => LineCap::Square,
        o => bail!("unknown lineCap '{o}' (round, butt, square)"),
    };
    let stroke = l.stroke.as_ref().map(|s| (s.width as f32, s.color));
    // closed shapes are filled (with `color`, default white, or `gradient`) unless only a stroke is given;
    // lines and arrows are stroked (default 8 px in `color`)
    let (fill, stroke) = if geo.closed {
        let fill = match (fill, l.color, &stroke) {
            (Some(f), _, _) => Some(f),
            (None, Some(c), _) => Some(Fill::Solid(c)),
            (None, None, None) => Some(Fill::Solid(Rgba::WHITE)),
            (None, None, Some(_)) => None,
        };
        (fill, stroke)
    } else {
        (None, Some(stroke.unwrap_or((8.0, l.color.unwrap_or(Rgba::WHITE)))))
    };
    if let Some(d) = &l.dash {
        if d.is_empty() || d.iter().any(|v| *v < 0.0) || d.iter().sum::<f64>() <= 0.0 {
            bail!("dash needs positive lengths, e.g. [20, 10]");
        }
    }
    let style = Style { fill, stroke, cap, dash: l.dash.as_ref().map(|d| d.iter().map(|v| *v as f32).collect()) };

    // the progress values this layer shows, quantized; fewer steps for big shapes
    let progress = l.progress.clone().filter(|p| !p.is_const());
    let mut values: Vec<f32> = match &progress {
        Some(p) => times.map(|(lt, dur)| p.eval(lt, dur).clamp(0.0, 1.0) as f32).collect(),
        None => vec![l.progress.as_ref().map(|p| p.eval(0.0, 1.0).clamp(0.0, 1.0) as f32).unwrap_or(1.0)],
    };
    let px = ((w * q).ceil() as usize + 64) * ((h * q).ceil() as usize + 64) * 4;
    let levels = (PROGRESS_BUDGET / px.max(1)).clamp(8, 1000) as f32;
    for v in &mut values {
        *v = (*v * levels).round() / levels;
    }
    values.sort_by(f32::total_cmp);
    values.dedup();
    let steps = values.iter().map(|&p| Ok((p, Arc::new(render(&geo, &style, w, h, q, p)?)))).collect::<Result<_>>()?;
    Ok(Shape { steps, progress })
}

fn geometry(l: &Layer, w: f32, h: f32) -> Result<Geometry> {
    let kind = l.shape.as_deref().unwrap_or(if l.d.is_some() {
        "path"
    } else if l.points.is_some() {
        "line"
    } else {
        "rect"
    });
    let pts = |default: Vec<(f32, f32)>| -> Result<Vec<(f32, f32)>> {
        match &l.points {
            Some(p) if p.len() < 2 => bail!("points needs at least two [x, y] pairs"),
            Some(p) => Ok(p.iter().map(|&[x, y]| (x as f32, y as f32)).collect()),
            None => Ok(default),
        }
    };
    let (cx, cy) = (w / 2.0, h / 2.0);
    let r = w.min(h) / 2.0;
    let polyline = |p: &[(f32, f32)], close: bool| -> Option<Path> {
        let mut b = PathBuilder::new();
        b.move_to(p[0].0, p[0].1);
        for &(x, y) in &p[1..] {
            b.line_to(x, y);
        }
        if close {
            b.close();
        }
        b.finish()
    };
    let regular = |n: usize, inner: Option<f32>| -> Vec<(f32, f32)> {
        let k = if inner.is_some() { 2 * n } else { n };
        (0..k)
            .map(|i| {
                let a = std::f32::consts::TAU * i as f32 / k as f32 - std::f32::consts::FRAC_PI_2;
                let rr = if i % 2 == 1 { inner.map(|f| r * f).unwrap_or(r) } else { r };
                (cx + rr * a.cos(), cy + rr * a.sin())
            })
            .collect()
    };
    let (path, closed, head) = match kind {
        "rect" => (
            rounded_rect(0.0, 0.0, w, h, l.radius as f32).or_else(|| polyline(&[(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)], true)),
            true,
            None,
        ),
        "ellipse" | "circle" => {
            let (rx, ry) = if kind == "circle" { (r, r) } else { (cx, cy) };
            (ellipse(cx, cy, rx, ry), true, None)
        }
        "line" => (polyline(&pts(vec![(0.0, cy), (w, cy)])?, false), false, None),
        "arrow" => {
            let sw = l.stroke.as_ref().map(|s| s.width as f32).unwrap_or(8.0);
            (polyline(&pts(vec![(0.0, cy), (w, cy)])?, false), false, Some(l.head_size.map(|v| v as f32).unwrap_or(sw * 4.0)))
        }
        "polygon" => {
            let p = match &l.points {
                Some(_) => pts(vec![])?,
                None => regular(l.sides.unwrap_or(6).max(3), None),
            };
            (polyline(&p, true), true, None)
        }
        "star" => {
            let inner = l.inner_radius.unwrap_or(0.5).clamp(0.01, 1.0) as f32;
            (polyline(&regular(l.sides.unwrap_or(5).max(2), Some(inner)), true), true, None)
        }
        "path" => {
            let d = l.d.as_deref().ok_or_else(|| anyhow!("shape \"path\" needs \"d\" (SVG path data)"))?;
            (Some(svg_path(d)?), d.trim_end().ends_with(['z', 'Z']), None)
        }
        o => bail!("unknown shape '{o}' ({})", SHAPES.join(", ")),
    };
    Ok(Geometry { path: path.ok_or_else(|| anyhow!("empty shape"))?, closed, head })
}

/// An ellipse starting at the top, clockwise (so `progress` grows like a clock hand).
fn ellipse(cx: f32, cy: f32, rx: f32, ry: f32) -> Option<Path> {
    const K: f32 = 0.552_284_8;
    let mut b = PathBuilder::new();
    b.move_to(cx, cy - ry);
    b.cubic_to(cx + K * rx, cy - ry, cx + rx, cy - K * ry, cx + rx, cy);
    b.cubic_to(cx + rx, cy + K * ry, cx + K * rx, cy + ry, cx, cy + ry);
    b.cubic_to(cx - K * rx, cy + ry, cx - rx, cy + K * ry, cx - rx, cy);
    b.cubic_to(cx - rx, cy - K * ry, cx - K * rx, cy - ry, cx, cy - ry);
    b.close();
    b.finish()
}

/// SVG path data (`M 0 0 L 100 50 A ...`) parsed by usvg.
fn svg_path(d: &str) -> Result<Path> {
    if d.contains(['"', '<', '&']) {
        bail!("d: not valid SVG path data");
    }
    let svg = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><path d="{d}"/></svg>"#);
    let tree = resvg::usvg::Tree::from_str(&svg, &Default::default()).context("d: not valid SVG path data")?;
    fn first(g: &resvg::usvg::Group) -> Option<Path> {
        g.children().iter().find_map(|n| match n {
            resvg::usvg::Node::Path(p) => Some(p.data().clone()),
            resvg::usvg::Node::Group(g) => first(g),
            _ => None,
        })
    }
    first(tree.root()).ok_or_else(|| anyhow!("d: no drawable path in '{d}'"))
}

/// The path as polylines (one per contour), curves flattened to `tol` units.
fn flatten(path: &Path, tol: f32) -> Vec<(Vec<Point>, bool)> {
    let mut out: Vec<(Vec<Point>, bool)> = Vec::new();
    let mut last = Point::zero();
    let steps = |len: f32| ((len / tol).ceil() as usize).clamp(2, 256);
    for seg in path.segments() {
        match seg {
            PathSegment::MoveTo(p) => {
                out.push((vec![p], false));
                last = p;
            }
            PathSegment::LineTo(p) => {
                if let Some(c) = out.last_mut() {
                    c.0.push(p);
                }
                last = p;
            }
            PathSegment::QuadTo(c, p) => {
                let n = steps(last.distance(c) + c.distance(p));
                let (a, cur) = (last, out.last_mut());
                if let Some(cur) = cur {
                    for i in 1..=n {
                        let t = i as f32 / n as f32;
                        let u = 1.0 - t;
                        cur.0.push(Point::from_xy(
                            u * u * a.x + 2.0 * u * t * c.x + t * t * p.x,
                            u * u * a.y + 2.0 * u * t * c.y + t * t * p.y,
                        ));
                    }
                }
                last = p;
            }
            PathSegment::CubicTo(c1, c2, p) => {
                let n = steps(last.distance(c1) + c1.distance(c2) + c2.distance(p));
                let (a, cur) = (last, out.last_mut());
                if let Some(cur) = cur {
                    for i in 1..=n {
                        let t = i as f32 / n as f32;
                        let u = 1.0 - t;
                        let (b0, b1, b2, b3) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
                        cur.0
                            .push(Point::from_xy(b0 * a.x + b1 * c1.x + b2 * c2.x + b3 * p.x, b0 * a.y + b1 * c1.y + b2 * c2.y + b3 * p.y));
                    }
                }
                last = p;
            }
            PathSegment::Close => {
                if let Some(cur) = out.last_mut() {
                    cur.1 = true;
                    if let Some(&first) = cur.0.first() {
                        cur.0.push(first);
                        last = first;
                    }
                }
            }
        }
    }
    out.retain(|(p, _)| p.len() > 1);
    out
}

fn length(p: &[Point]) -> f32 {
    p.windows(2).map(|w| w[0].distance(w[1])).sum()
}

/// The first `frac` of the contours (by length), plus the end point and direction there.
fn trim(contours: &[(Vec<Point>, bool)], frac: f32) -> (Vec<Vec<Point>>, Option<(Point, Point)>) {
    let total: f32 = contours.iter().map(|(p, _)| length(p)).sum();
    let mut left = total * frac.clamp(0.0, 1.0);
    let mut out = Vec::new();
    let mut end = None;
    for (pts, _) in contours {
        if left <= 0.0 {
            break;
        }
        let mut kept = vec![pts[0]];
        for w in pts.windows(2) {
            let d = w[0].distance(w[1]);
            if d >= left {
                let t = if d > 0.0 { left / d } else { 0.0 };
                let p = Point::from_xy(w[0].x + (w[1].x - w[0].x) * t, w[0].y + (w[1].y - w[0].y) * t);
                kept.push(p);
                end = Some((p, Point::from_xy(w[1].x - w[0].x, w[1].y - w[0].y)));
                left = 0.0;
                break;
            }
            left -= d;
            kept.push(w[1]);
            end = Some((w[1], Point::from_xy(w[1].x - w[0].x, w[1].y - w[0].y)));
        }
        out.push(kept);
    }
    (out, end)
}

fn shader(fill: &Fill, w: f32, h: f32) -> Option<Shader<'static>> {
    let stops = |s: &Vec<(f32, Rgba)>| s.iter().map(|(p, c)| GradientStop::new(*p, c.skia(1.0))).collect::<Vec<_>>();
    match fill {
        Fill::Solid(c) => Some(Shader::SolidColor(c.skia(1.0))),
        Fill::Linear(angle, s) => {
            let a = angle.to_radians() as f32;
            let (dx, dy) = (a.sin(), -a.cos());
            let len = (w * dx).abs() + (h * dy).abs();
            let (cx, cy) = (w / 2.0, h / 2.0);
            LinearGradient::new(
                Point::from_xy(cx - dx * len / 2.0, cy - dy * len / 2.0),
                Point::from_xy(cx + dx * len / 2.0, cy + dy * len / 2.0),
                stops(s),
                SpreadMode::Pad,
                Transform::identity(),
            )
        }
        Fill::Radial(s) => {
            let c = Point::from_xy(w / 2.0, h / 2.0);
            RadialGradient::new(c, c, (w * w + h * h).sqrt() / 2.0, stops(s), SpreadMode::Pad, Transform::identity())
        }
        Fill::Bitmap(_) => None,
    }
}

/// Rasterize at `q` px per unit with the outline drawn up to `progress`.
fn render(geo: &Geometry, st: &Style, w: f32, h: f32, q: f32, progress: f32) -> Result<Sprite> {
    let sw = st.stroke.map(|s| s.0).unwrap_or(0.0);
    let head = geo.head.unwrap_or(0.0);
    // room for strokes, caps and arrow heads that stick out of the box (and for points outside it)
    let b = geo.path.bounds();
    let pad = sw + head + 2.0;
    let (x0, y0) = (b.left().min(0.0) - pad, b.top().min(0.0) - pad);
    let (x1, y1) = (b.right().max(w) + pad, b.bottom().max(h) + pad);
    let (pw, ph) = (((x1 - x0) * q).ceil().max(1.0) as u32, ((y1 - y0) * q).ceil().max(1.0) as u32);
    let mut pm = Pixmap::new(pw, ph).ok_or_else(|| anyhow!("shape too large ({pw}x{ph} px)"))?;
    let ts = Transform::from_row(q, 0.0, 0.0, q, -x0 * q, -y0 * q);

    if let Some(f) = &st.fill {
        if let Some(sh) = shader(f, w, h) {
            let paint = Paint { shader: sh, anti_alias: true, ..Default::default() };
            pm.fill_path(&geo.path, &paint, FillRule::Winding, ts, None);
        }
    }
    if let Some((sw, color)) = st.stroke {
        let mut paint = Paint::default();
        paint.set_color(color.skia(1.0));
        paint.anti_alias = true;
        let mut stroke = Stroke { width: sw, line_cap: st.cap, line_join: LineJoin::Round, ..Default::default() };
        if let Some(d) = &st.dash {
            let mut d = d.clone();
            if d.len() % 2 == 1 {
                d.extend_from_within(..);
            }
            stroke.dash = StrokeDash::new(d, 0.0);
        }
        // whole outline: the exact path; partial: the flattened outline up to `progress`
        let (outline, end, drawn) = if progress >= 1.0 && geo.head.is_none() {
            (Some(geo.path.clone()), None, 0.0)
        } else {
            let contours = flatten(&geo.path, 0.5 / q);
            let (parts, end) = trim(&contours, progress);
            let drawn: f32 = parts.iter().map(|p| length(p)).sum();
            let mut pb = PathBuilder::new();
            for p in parts.iter().filter(|p| p.len() > 1) {
                pb.move_to(p[0].x, p[0].y);
                for pt in &p[1..] {
                    pb.line_to(pt.x, pt.y);
                }
            }
            (pb.finish(), end, drawn)
        };
        if let Some(path) = outline {
            pm.stroke_path(&path, &paint, &stroke, ts, None);
        }
        // arrow head: a chevron at the end of the drawn part, in the stroke's style
        if let (Some(len), Some((tip, dir))) = (geo.head, end) {
            // a short line gets a proportionally small head
            let len = len.min(drawn * 0.8);
            let n = (dir.x * dir.x + dir.y * dir.y).sqrt();
            if n > 0.0 && len > 0.0 {
                let (ux, uy) = (dir.x / n, dir.y / n);
                let (cos, sin) = (0.5f32.cos(), 0.5f32.sin()); // ~29 degrees each side
                let wing = |s: f32| Point::from_xy(tip.x - len * (ux * cos - s * uy * sin), tip.y - len * (uy * cos + s * ux * sin));
                let mut pb = PathBuilder::new();
                let (a, c) = (wing(1.0), wing(-1.0));
                pb.move_to(a.x, a.y);
                pb.line_to(tip.x, tip.y);
                pb.line_to(c.x, c.y);
                if let Some(p) = pb.finish() {
                    let stroke = Stroke { width: sw, line_cap: LineCap::Round, line_join: LineJoin::Round, ..Default::default() };
                    pm.stroke_path(&p, &paint, &stroke, ts, None);
                }
            }
        }
    }
    Ok(Sprite { pm, ox: -x0 * q, oy: -y0 * q, w, h, ppu: q })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn layer(v: serde_json::Value) -> Layer {
        serde_json::from_value(v).unwrap()
    }

    fn coverage(s: &Sprite) -> usize {
        s.pm.pixels().iter().filter(|p| p.alpha() > 128).count()
    }

    #[test]
    fn every_shape_draws() {
        for (kind, extra) in [
            ("rect", json!({"radius": 12})),
            ("ellipse", json!({})),
            ("circle", json!({"stroke": {"width": 6, "color": "#fff"}})),
            ("line", json!({"points": [[0, 0], [50, 80], [100, 20]]})),
            ("arrow", json!({})),
            ("polygon", json!({"sides": 3})),
            ("star", json!({"innerRadius": 0.4})),
            ("path", json!({"d": "M 0 0 C 50 100 50 -50 100 50 Z"})),
        ] {
            let mut v = json!({"type": "shape", "shape": kind});
            v.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            let s = build(&layer(v), 100.0, 100.0, 1.0, None, std::iter::empty()).unwrap();
            assert_eq!(s.steps.len(), 1);
            assert!(coverage(&s.steps[0].1) > 100, "{kind} draws something");
        }
        assert!(build(&layer(json!({"type": "shape", "shape": "blob"})), 10.0, 10.0, 1.0, None, std::iter::empty()).is_err());
        assert!(
            build(&layer(json!({"type": "shape", "shape": "path", "d": "nonsense"})), 10.0, 10.0, 1.0, None, std::iter::empty()).is_err()
        );
    }

    #[test]
    fn progress_draws_the_outline_on() {
        let l = layer(json!({"type": "shape", "shape": "circle", "stroke": {"width": 10, "color": "#fff"},
                             "progress": [[0, 0], [1, 1, "linear"]]}));
        let times = (0..=30).map(|f| (f as f64 / 30.0, 1.0));
        let s = build(&l, 200.0, 200.0, 1.0, None, times).unwrap();
        assert_eq!(s.steps.len(), 31, "one sprite per distinct value");
        let (half, full) = (coverage(s.sprite(0.5, 1.0)), coverage(s.sprite(1.0, 1.0)));
        assert!(coverage(s.sprite(0.0, 1.0)) < 50 && half > full * 4 / 10 && half < full * 6 / 10, "{half} of {full}");
        // starts at the top (12 o'clock) and runs clockwise: a quarter covers the top-right only
        let q = s.sprite(0.25, 1.0);
        let px = |x: f32, y: f32| q.pm.pixel((q.ox + x) as u32, (q.oy + y) as u32).unwrap().alpha();
        assert!(px(170.0, 30.0) > 128 && px(30.0, 170.0) == 0);
    }

    #[test]
    fn trim_finds_the_end_and_direction() {
        let c = vec![(vec![Point::from_xy(0.0, 0.0), Point::from_xy(10.0, 0.0), Point::from_xy(10.0, 10.0)], false)];
        let (parts, end) = trim(&c, 0.75);
        assert_eq!(parts[0].len(), 3);
        let (p, d) = end.unwrap();
        assert!((p.y - 5.0).abs() < 1e-4 && d.y > 0.0 && d.x == 0.0);
    }
}
