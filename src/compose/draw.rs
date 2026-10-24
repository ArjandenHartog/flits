//! Per-frame traversal: evaluates animations and emits paint ops.

use super::{Fill, Fit, Kind, Node, Prepared};
use crate::media::video::VideoReader;
use crate::paint::{cpu::CpuPainter, sprite_op, Clip, Op, Painter, Src};
use crate::scene::anim::Fx;
use crate::scene::color::Rgba;
use crate::text::Align;
use anyhow::Result;
use std::collections::HashMap;
use tiny_skia::{GradientStop, LinearGradient, Paint, Pixmap, Point, RadialGradient, Rect, SpreadMode, Transform};

/// Per-worker state: open video decoders.
#[derive(Default)]
pub struct VideoState {
    /// decoder per video node: (node end time, reader)
    readers: HashMap<usize, (f64, VideoReader)>,
    /// decoders of nodes that just ended, kept briefly: the next cut of the same clip
    /// (e.g. after a removed silence) continues with them instead of a fresh start
    spare: Vec<(f64, VideoReader)>,
    /// decode 10-bit sources at full precision (the GPU path supports 16-bit planes)
    pub high_bit: bool,
}

/// Start decoders this long before their layer appears (hides start-up and seeking).
const PREFETCH: f64 = 1.0;
/// A spare decoder is reused when the new cut starts at most this far ahead in the source.
const REUSE_GAP: f64 = 4.0;

pub struct FrameCtx<'a> {
    pub fps: f64,
    pub videos: &'a VideoState,
}

/// Software render of one frame into `canvas`.
pub fn render_frame(p: &Prepared, frame: i64, canvas: &mut Pixmap, vs: &mut VideoState) -> Result<()> {
    canvas.fill(p.background.skia(1.0));
    advance(p, frame, vs)?;
    let mut painter = CpuPainter { canvas };
    draw_frame(p, frame, &mut painter, vs);
    Ok(())
}

/// Decode the video frames needed for `frame`.
pub fn advance(p: &Prepared, frame: i64, vs: &mut VideoState) -> Result<()> {
    let t = frame as f64 / p.fps;
    // finished layers hand their decoder to the spare pool
    let ended: Vec<usize> = vs.readers.iter().filter(|(_, (end, _))| *end <= t).map(|(k, _)| *k).collect();
    for k in ended {
        let (_, r) = vs.readers.remove(&k).unwrap();
        vs.spare.push((t, r));
    }
    vs.spare.retain(|(since, _)| t - since < 2.0);
    vs.spare.truncate(4);
    for n in &p.nodes {
        advance_videos(n, t, frame, p.fps, vs)?;
    }
    Ok(())
}

/// Walk the tree for one frame and emit draw ops to any backend (call `advance` first).
pub fn draw_frame<'a>(p: &'a Prepared, frame: i64, painter: &mut dyn Painter<'a>, vs: &'a VideoState) {
    let t = frame as f64 / p.fps;
    let root = Transform::from_scale(p.root_scale, p.root_scale);
    let ctx = FrameCtx { fps: p.fps, videos: vs };
    for n in &p.nodes {
        draw_node(painter, n, t, root, 1.0, None, &ctx);
    }
}

fn advance_videos(n: &Node, t: f64, frame: i64, fps: f64, vs: &mut VideoState) -> Result<()> {
    if t + PREFETCH < n.start || t >= n.end {
        return Ok(());
    }
    if let Kind::Video { src, info, dw, dh, trim, looped, speed, .. } = &n.kind {
        let start_frame = (n.start * fps).round() as i64;
        let lf = frame - start_frame;
        let target = lf.max(0);
        let usable = |r: &VideoReader| target >= r.cur && target <= r.cur + (fps * 2.0) as i64 && target >= r.first();
        let want = trim + target as f64 * speed / fps;
        let reachable = |r: &VideoReader| {
            r.compatible(src, *dw, *dh, *speed, *looped)
                && want >= r.next_source_time() - 0.5 / fps
                && want - r.next_source_time() < REUSE_GAP
        };
        // not started yet and a running decoder of this clip will hand over: don't prefetch
        let handover = lf < 0 && vs.readers.iter().any(|(id, (_, r))| *id != n.id && reachable(r));
        if !handover && !vs.readers.get(&n.id).is_some_and(|(_, r)| usable(r)) {
            // continue a decoder of the same clip that is just behind this cut, if any
            let reuse = vs.spare.iter().position(|(_, r)| reachable(r));
            let r = match reuse {
                Some(k) => {
                    let (_, mut r) = vs.spare.swap_remove(k);
                    r.rebase(*trim);
                    r
                }
                None => VideoReader::spawn(src, info, *trim, target, fps, *dw, *dh, *looped, *speed, vs.high_bit)?,
            };
            vs.readers.insert(n.id, (n.end, r));
        }
        // before the layer starts the decoder just fills its read-ahead buffer
        if let (true, Some((_, r))) = (lf >= 0, vs.readers.get_mut(&n.id)) {
            r.seek_forward(lf)?;
        }
    }
    for c in &n.children {
        advance_videos(c, t, frame, fps, vs)?;
    }
    Ok(())
}

pub fn gradient_pixmap(fill: &Fill, w: f32, h: f32, q: f32) -> Option<Pixmap> {
    let (pw, ph) = ((w * q).ceil().max(1.0) as u32, (h * q).ceil().max(1.0) as u32);
    let mut pm = Pixmap::new(pw, ph)?;
    let (w, h) = (pw as f32, ph as f32);
    let stops = |s: &Vec<(f32, Rgba)>| s.iter().map(|(p, c)| GradientStop::new(*p, c.skia(1.0))).collect::<Vec<_>>();
    let shader = match fill {
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
            )?
        }
        Fill::Radial(s) => {
            let c = Point::from_xy(w / 2.0, h / 2.0);
            RadialGradient::new(c, c, (w * w + h * h).sqrt() / 2.0, stops(s), SpreadMode::Pad, Transform::identity())?
        }
        _ => return None,
    };
    let paint = Paint { shader, ..Paint::default() };
    pm.fill_rect(Rect::from_xywh(0.0, 0.0, w, h)?, &paint, Transform::identity(), None);
    Some(pm)
}

/// Op for an image/video drawn into a w*h box with fit/zoom/pan.
#[allow(clippy::too_many_arguments)]
fn media_op<'a>(
    src: Src<'a>,
    iw: f32,
    ih: f32,
    fit: Fit,
    w: f32,
    h: f32,
    zoom: f32,
    pan: (f32, f32),
    radius: f32,
    ts: Transform,
    opacity: f32,
    clip: Option<Clip>,
) -> Option<Op<'a>> {
    let (sx, sy) = match fit {
        Fit::Cover => {
            let s = (w / iw).max(h / ih);
            (s, s)
        }
        Fit::Contain => {
            let s = (w / iw).min(h / ih);
            (s, s)
        }
        Fit::Fill => (w / iw, h / ih),
    };
    let (sx, sy) = (sx * zoom, sy * zoom);
    let (dw, dh) = (iw * sx, ih * sy);
    let ox = (w - dw) / 2.0 * (1.0 + pan.0);
    let oy = (h - dh) / 2.0 * (1.0 + pan.1);
    let shape = [ox.max(0.0), oy.max(0.0), (ox + dw).min(w), (oy + dh).min(h)];
    if shape[2] <= shape[0] || shape[3] <= shape[1] {
        return None;
    }
    Some(Op { src, ts, shape, radius, uv: Transform::from_row(sx, 0.0, 0.0, sy, ox, oy), opacity, clip })
}

#[allow(clippy::too_many_arguments)]
fn draw_node<'a>(
    painter: &mut dyn Painter<'a>,
    n: &'a Node,
    t: f64,
    parent: Transform,
    parent_op: f32,
    pclip: Option<Clip>,
    ctx: &FrameCtx<'a>,
) {
    if t < n.start || t >= n.end {
        return;
    }
    let lt = t - n.start;
    let dur = n.end - n.start;
    let mut fx = Fx::default();
    let area = (n.area.0 as f64, n.area.1 as f64);
    if let Some(tr) = &n.enter {
        tr.apply_in(lt, area, &mut fx);
    }
    if let Some(tr) = &n.exit {
        tr.apply_out(dur - lt, area, &mut fx);
    }
    if fx.blur > 1e-3 {
        // zoom blur: the layer drawn GHOSTS times at growing scales; copy k at 1/(k+1) opacity
        // makes the result their average (exact for opaque layers)
        const GHOSTS: usize = 8;
        for k in 0..GHOSTS {
            let mut f = fx;
            f.blur = 0.0;
            f.scale *= 1.0 + fx.blur * k as f64 / (GHOSTS - 1) as f64;
            f.opacity /= (k + 1) as f32;
            draw_fx(painter, n, t, parent, parent_op, pclip, ctx, f);
        }
        return;
    }
    draw_fx(painter, n, t, parent, parent_op, pclip, ctx, fx);
}

/// Draw a node with the transition effect `fx` already worked out.
#[allow(clippy::too_many_arguments)]
fn draw_fx<'a>(
    painter: &mut dyn Painter<'a>,
    n: &'a Node,
    t: f64,
    parent: Transform,
    parent_op: f32,
    pclip: Option<Clip>,
    ctx: &FrameCtx<'a>,
    fx: Fx,
) {
    let lt = t - n.start;
    let dur = n.end - n.start;
    let op = parent_op * n.opacity.eval(lt, dur).clamp(0.0, 1.0) as f32 * fx.opacity;
    if op <= 0.002 {
        return;
    }
    let x = n.x.eval(lt, dur) + fx.dx;
    let y = n.y.eval(lt, dur) + fx.dy;
    let s = n.scale.eval(lt, dur) * fx.scale;
    let (sx, sy) = ((s * fx.sx * n.scale_x.eval(lt, dur)) as f32, (s * fx.sy * n.scale_y.eval(lt, dur)) as f32);
    if sx.abs() < 1e-4 || sy.abs() < 1e-4 {
        return;
    }
    let rot = (n.rotation.eval(lt, dur) + fx.rot) as f32;
    let (w, h) = (n.w, n.h);
    let mut ts = parent.pre_translate(x as f32, y as f32);
    if rot != 0.0 {
        ts = ts.pre_concat(Transform::from_rotate(rot));
    }
    let ts = ts.pre_scale(sx, sy).pre_translate(-n.anchor.0 * w, -n.anchor.1 * h);
    let clip = match fx.clip {
        Some(c) => {
            let (cx, cy, cw, ch) = c.rect(w as f64, h as f64);
            let radius = c.radius(w as f64, h as f64) as f32;
            Some(Clip { ts, rect: [cx as f32, cy as f32, (cx + cw) as f32, (cy + ch) as f32], radius })
        }
        None => pclip,
    };

    // shadow + border behind box-like layers
    let bw = n.border.map(|b| b.0).unwrap_or(0.0);
    if let Some(sh) = &n.shadow {
        painter.draw(sprite_op(sh, ts.pre_translate(-bw, -bw), op, clip));
    }
    if let Some((bw, bc)) = n.border {
        painter.draw(Op {
            src: Src::Solid(bc),
            ts,
            shape: [-bw, -bw, w + bw, h + bw],
            radius: n.radius + bw,
            uv: Transform::identity(),
            opacity: op,
            clip,
        });
    }

    match &n.kind {
        Kind::Group => {
            let clip = if n.clip { Some(Clip { ts, rect: [0.0, 0.0, w, h], radius: 0.0 }) } else { clip };
            for c in &n.children {
                draw_node(painter, c, t, ts, op, clip, ctx);
            }
        }
        Kind::Rect(fill) => match fill {
            Fill::Solid(c) => painter.draw(Op {
                src: Src::Solid(*c),
                ts,
                shape: [0.0, 0.0, w, h],
                radius: n.radius,
                uv: Transform::identity(),
                opacity: op,
                clip,
            }),
            Fill::Bitmap(pm) => {
                let (kx, ky) = (w / pm.width() as f32, h / pm.height() as f32);
                painter.draw(Op {
                    src: Src::Image(pm),
                    ts,
                    shape: [0.0, 0.0, w, h],
                    radius: n.radius,
                    uv: Transform::from_scale(kx, ky),
                    opacity: op,
                    clip,
                });
            }
            _ => {}
        },
        Kind::Image { pm, fit, .. } => {
            if let Some(pm) = pm {
                let zoom = n.zoom.eval(lt, dur) as f32;
                let pan = (n.pan_x.eval(lt, dur) as f32, n.pan_y.eval(lt, dur) as f32);
                if let Some(o) =
                    media_op(Src::Image(pm), pm.width() as f32, pm.height() as f32, *fit, w, h, zoom, pan, n.radius, ts, op, clip)
                {
                    painter.draw(o);
                }
            }
        }
        Kind::Video { fit, adjust, .. } => {
            if let Some((_, r)) = ctx.videos.readers.get(&n.id) {
                let zoom = n.zoom.eval(lt, dur) as f32;
                let pan = (n.pan_x.eval(lt, dur) as f32, n.pan_y.eval(lt, dur) as f32);
                let src = Src::Frame { key: n.id, ver: r.cur, data: &r.frame, w: r.w, h: r.h, fmt: r.fmt, adjust: *adjust };
                if let Some(o) = media_op(src, r.w as f32, r.h as f32, *fit, w, h, zoom, pan, n.radius, ts, op, clip) {
                    painter.draw(o);
                }
            }
        }
        Kind::Text { sprite, reveal } => match reveal {
            None => painter.draw(sprite_op(sprite, ts, op, clip)),
            Some(r) => {
                let step = r.duration / r.pieces.len().max(1) as f64;
                for (i, (s, px, py)) in r.pieces.iter().enumerate() {
                    let since = lt - i as f64 * step;
                    if since < 0.0 {
                        continue;
                    }
                    if r.instant {
                        painter.draw(sprite_op(s, ts.pre_translate(*px, *py), op, clip));
                    } else {
                        let e = crate::scene::anim::Ease::Out.apply((since / 0.35).clamp(0.0, 1.0)) as f32;
                        painter.draw(sprite_op(s, ts.pre_translate(*px, *py + (1.0 - e) * s.h * 0.35), op * e, clip));
                    }
                }
            }
        },
        Kind::Counter(c) => {
            let text = c.format(c.value.eval(lt, dur));
            let tw = c.width(&text);
            let x0 = match c.align {
                Align::Left => 0.0,
                Align::Center => (w - tw) / 2.0,
                Align::Right => w - tw,
            };
            for (s, x) in c.layout(&text) {
                painter.draw(sprite_op(s, ts.pre_translate(x0 + x, 0.0), op, clip));
            }
        }
        Kind::Captions(c) => c.draw(painter, lt, ts, op, clip),
        Kind::Shape(sh) => painter.draw(sprite_op(sh.sprite(lt, dur), ts, op, clip)),
        Kind::Waveform { spec, color, mirror, gap } => {
            let fi = (lt * ctx.fps).round() as i64;
            if let Some(vals) = spec.at(fi) {
                let nb = vals.len() as f32;
                let slot = w / nb;
                let bwid = slot * (1.0 - gap);
                let r = if n.radius > 0.0 { n.radius } else { bwid / 2.0 };
                for (i, v) in vals.iter().enumerate() {
                    let bh = (v * h).max(bwid.min(h));
                    let x = i as f32 * slot + (slot - bwid) / 2.0;
                    let y = if *mirror { (h - bh) / 2.0 } else { h - bh };
                    painter.draw(Op {
                        src: Src::Solid(*color),
                        ts,
                        shape: [x, y, x + bwid, y + bh],
                        radius: r,
                        uv: Transform::identity(),
                        opacity: op,
                        clip,
                    });
                }
            }
        }
    }
}
