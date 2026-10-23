//! Turns a Scene into a tree of prepared nodes (all static pixels pre-rendered),
//! and draws one frame of that tree.

use crate::anim::{Fx, Prop, TKind, Transition};
use crate::assets;
use crate::captions::{self, CapKind, CapOptions, Captions};
use crate::color::Rgba;
use crate::effects::{box_shadow, Sprite};
use crate::ffmpeg::{self, MediaInfo, VideoReader};
use crate::scene::{Layer, Scene};
use crate::text::{self, Align, FontDb, TextStyle};
use crate::waveform::{self, Spectrum};
use anyhow::{anyhow, bail, Context, Result};
use rayon::prelude::*;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use crate::paint::{sprite_op, Clip, CpuPainter, Op, Painter, Src};
use tiny_skia::{GradientStop, LinearGradient, Paint, Pixmap, Point, RadialGradient, Rect, SpreadMode, Transform};

#[derive(Clone, Copy, PartialEq)]
pub enum Fit {
    Cover,
    Contain,
    Fill,
}

pub enum Fill {
    Solid(Rgba),
    /// gradient pre-rendered at prepare time
    Bitmap(Arc<Pixmap>),
    Linear(f64, Vec<(f32, Rgba)>),
    Radial(Vec<(f32, Rgba)>),
}

type ImgKey = (PathBuf, u32, u32, u32);

pub enum Kind {
    Group,
    Rect(Fill),
    Image { key: ImgKey, pm: Option<Arc<Pixmap>>, fit: Fit },
    Video { src: PathBuf, info: MediaInfo, dw: u32, dh: u32, trim: f64, looped: bool, fit: Fit },
    Text { sprite: Arc<Sprite>, pieces: Option<(Vec<(Arc<Sprite>, f32, f32)>, f64)> },
    Captions(Arc<Captions>),
    Waveform { spec: Arc<Spectrum>, color: Rgba, mirror: bool, gap: f32 },
}

pub struct Node {
    pub id: usize,
    pub kind: Kind,
    pub start: f64,
    pub end: f64,
    pub w: f32,
    pub h: f32,
    x: Prop,
    y: Prop,
    anchor: (f32, f32),
    scale: Prop,
    scale_x: Prop,
    scale_y: Prop,
    rotation: Prop,
    opacity: Prop,
    zoom: Prop,
    pan_x: Prop,
    pan_y: Prop,
    enter: Option<Transition>,
    exit: Option<Transition>,
    radius: f32,
    border: Option<(f32, Rgba)>,
    shadow: Option<Arc<Sprite>>,
    pub children: Vec<Node>,
}

#[derive(Clone, Debug)]
pub struct AudioInput {
    pub path: PathBuf,
    pub start: f64,
    pub trim: f64,
    pub dur: f64,
    pub volume: f64,
    pub fade_in: f64,
    pub fade_out: f64,
    pub looped: bool,
}

pub struct Prepared {
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub duration: f64,
    pub frames: i64,
    pub background: Rgba,
    pub root_scale: f32,
    pub nodes: Vec<Node>,
    pub audio: Vec<AudioInput>,
}

struct Ctx<'a> {
    scene: &'a Scene,
    fonts: FontDb,
    probes: HashMap<PathBuf, MediaInfo>,
    next_id: usize,
    audio: Vec<AudioInput>,
    render_scale: f64,
}

fn prop(p: &Option<Prop>, default: f64) -> Prop {
    p.clone().unwrap_or(Prop::Const(default))
}

fn parse_fit(s: Option<&str>) -> Result<Fit> {
    Ok(match s.unwrap_or("cover") {
        "cover" => Fit::Cover,
        "contain" => Fit::Contain,
        "fill" | "stretch" => Fit::Fill,
        o => bail!("unknown fit '{o}' (cover, contain, fill)"),
    })
}

fn collect_media(layers: &[Layer], scene: &Scene, out: &mut Vec<PathBuf>) {
    for l in layers {
        if l.kind == "video" {
            if let Some(s) = &l.src {
                out.push(scene.resolve(s));
            }
        }
        collect_media(&l.children, scene, out);
    }
}

fn transition_scale(t: &Option<Transition>) -> f64 {
    match t.as_ref().map(|t| t.kind) {
        Some(TKind::ZoomOut) | Some(TKind::ZoomIn) => 1.15,
        Some(TKind::Pop) => 1.1,
        _ => 1.0,
    }
}

impl Prepared {
    pub fn build(scene: &Scene, render_scale: f64) -> Result<Prepared> {
        let mut media = Vec::new();
        collect_media(&scene.layers, scene, &mut media);
        for a in &scene.audio {
            media.push(scene.resolve(&a.src));
        }
        media.sort();
        media.dedup();
        let probes: HashMap<PathBuf, MediaInfo> = media
            .par_iter()
            .map(|p| ffmpeg::probe(p).map(|i| (p.clone(), i)))
            .collect::<Result<_>>()?;
        let aliases = scene.fonts.iter().map(|(k, v)| (k.clone(), scene.resolve(v))).collect();
        let mut ctx = Ctx { scene, fonts: FontDb::new(aliases), probes, next_id: 0, audio: vec![], render_scale };

        let (cw, ch) = (scene.width as f32, scene.height as f32);
        let default_end = scene.duration.unwrap_or(f64::INFINITY);
        let mut nodes = build_list(&scene.layers, false, 0.0, 0.0, default_end, (cw, ch), 1.0, &mut ctx)?;
        let duration = match scene.duration {
            Some(d) => d,
            None => {
                let mut d = nodes.iter().map(|n| n.end).filter(|e| e.is_finite()).fold(0.0, f64::max);
                for a in &scene.audio {
                    if let Some(dur) = a.duration {
                        d = d.max(a.start + dur);
                    }
                }
                if d <= 0.0 {
                    bail!("cannot determine the video length: set \"duration\" on the scene or on the layers");
                }
                d
            }
        };
        for n in nodes.iter_mut() {
            clamp_end(n, duration);
        }
        for a in &scene.audio {
            let path = scene.resolve(&a.src);
            let nat = ctx.probes.get(&path).map(|i| i.duration).unwrap_or(0.0);
            let dur = a.duration.unwrap_or(if a.looped { duration - a.start } else { nat - a.trim }).min(duration - a.start);
            if dur > 0.0 {
                ctx.audio.push(AudioInput { path, start: a.start, trim: a.trim, dur, volume: a.volume, fade_in: a.fade_in, fade_out: a.fade_out, looped: a.looped });
            }
        }
        for a in ctx.audio.iter_mut() {
            a.dur = a.dur.min(duration - a.start);
        }
        ctx.audio.retain(|a| a.dur > 0.0);

        // load all images in parallel
        let mut keys = Vec::new();
        collect_keys(&nodes, &mut keys);
        keys.sort();
        keys.dedup();
        let loaded: HashMap<ImgKey, Arc<Pixmap>> = keys
            .par_iter()
            .map(|k| {
                assets::load_image(&k.0, k.1 as f64, k.2 as f64, k.3 as f64 / 100.0).map(|pm| (k.clone(), Arc::new(pm)))
            })
            .collect::<Result<_>>()?;
        fill_images(&mut nodes, &loaded);

        let even = |v: f64| ((v / 2.0).round() as u32 * 2).max(2);
        let width = even(scene.width as f64 * render_scale);
        let height = even(scene.height as f64 * render_scale);
        Ok(Prepared {
            width,
            height,
            fps: scene.fps,
            duration,
            frames: (duration * scene.fps).round() as i64,
            background: scene.background.unwrap_or(Rgba::BLACK),
            root_scale: width as f32 / scene.width as f32,
            nodes,
            audio: ctx.audio,
        })
    }
}

fn clamp_end(n: &mut Node, d: f64) {
    if !n.end.is_finite() || n.end > d {
        n.end = d;
    }
    for c in n.children.iter_mut() {
        clamp_end(c, d);
    }
}

fn collect_keys(nodes: &[Node], out: &mut Vec<ImgKey>) {
    for n in nodes {
        if let Kind::Image { key, .. } = &n.kind {
            out.push(key.clone());
        }
        collect_keys(&n.children, out);
    }
}

fn fill_images(nodes: &mut [Node], map: &HashMap<ImgKey, Arc<Pixmap>>) {
    for n in nodes {
        if let Kind::Image { key, pm, .. } = &mut n.kind {
            *pm = map.get(key).cloned();
        }
        fill_images(&mut n.children, map);
    }
}

#[allow(clippy::too_many_arguments)]
fn build_list(layers: &[Layer], sequence: bool, overlap: f64, base: f64, default_end: f64, pbox: (f32, f32), pscale: f64, ctx: &mut Ctx) -> Result<Vec<Node>> {
    let mut out = Vec::with_capacity(layers.len());
    let mut cursor = 0.0;
    for (i, l) in layers.iter().enumerate() {
        let label = l.id.clone().unwrap_or_else(|| format!("{} #{i}", l.kind));
        let node = if sequence {
            let mut l2 = l.clone();
            l2.start = cursor + l.start;
            if l.end.is_some() {
                bail!("{label}: layers inside a sequence use \"duration\", not \"end\"");
            }
            let n = build_layer(&l2, base, default_end, pbox, pscale, ctx).with_context(|| label.clone())?;
            if !n.end.is_finite() {
                bail!("{label}: layers inside a sequence need a \"duration\"");
            }
            cursor = n.end - base - overlap;
            n
        } else {
            build_layer(l, base, default_end, pbox, pscale, ctx).with_context(|| label.clone())?
        };
        out.push(node);
    }
    Ok(out)
}

fn text_style(l: &Layer, ctx: &Ctx, default_font: Option<&str>, default_size: f64, q: f32) -> Result<TextStyle> {
    let font = ctx.fonts.get(l.font.as_deref().or(default_font), l.weight)?;
    let size = l.size.unwrap_or(default_size) as f32;
    let bg = l.background.as_ref().map(|b| {
        let (px, py) = b.padding.as_ref().map(|p| p.xy()).unwrap_or((size as f64 * 0.4, size as f64 * 0.2));
        (b.color, px as f32 * q, py as f32 * q, b.radius.unwrap_or(size as f64 * 0.25) as f32 * q, b.mode.as_deref() == Some("lines"))
    });
    Ok(TextStyle {
        font,
        size: size * q,
        color: l.color.unwrap_or(Rgba::WHITE),
        line_height: l.line_height.unwrap_or(1.15) as f32,
        letter_spacing: l.letter_spacing as f32 * q,
        align: Align::parse(l.align.as_deref())?,
        max_width: l.max_width.map(|m| m as f32 * q),
        stroke: l.stroke.as_ref().map(|s| (s.width as f32 * q, s.color)),
        shadow: l.shadow.as_ref().map(|s| (s.color, s.blur as f32 * q, s.x as f32 * q, s.y as f32 * q)),
        bg,
    })
}

fn rescale(mut s: Sprite, q: f32) -> Sprite {
    s.w /= q;
    s.h /= q;
    s.ppu = q;
    s
}

fn build_layer(l: &Layer, base: f64, default_end: f64, pbox: (f32, f32), pscale: f64, ctx: &mut Ctx) -> Result<Node> {
    let id = ctx.next_id;
    ctx.next_id += 1;
    let scene = ctx.scene;
    let start = base + l.start;
    let mut w = l.width.map(|v| v as f32);
    let mut h = l.height.map(|v| v as f32);
    let max_scale = prop(&l.scale, 1.0).max_abs()
        * prop(&l.scale_x, 1.0).max_abs().max(prop(&l.scale_y, 1.0).max_abs())
        * transition_scale(&l.enter).max(transition_scale(&l.exit))
        * pscale;
    let zoom = prop(&l.zoom, 1.0);
    let rs = ctx.render_scale;
    // oversampling for text so scaled-up text stays sharp
    let q = ((max_scale * rs).clamp(0.25, 3.0)) as f32;

    let mut natural: Option<f64> = None;
    let mut children = Vec::new();
    let kind = match l.kind.as_str() {
        "group" => Kind::Group,
        "rect" => Kind::Rect(match &l.gradient {
            Some(g) => {
                let mut stops = g.stops.iter().map(|(p, c)| (*p as f32, *c)).collect::<Vec<_>>();
                if stops.is_empty() {
                    let n = g.colors.len().max(2) - 1;
                    stops = g.colors.iter().enumerate().map(|(i, c)| (i as f32 / n as f32, *c)).collect();
                }
                if stops.len() < 2 {
                    bail!("gradient needs at least two stops or colors");
                }
                if g.kind.as_deref() == Some("radial") {
                    Fill::Radial(stops)
                } else {
                    Fill::Linear(g.angle.unwrap_or(180.0), stops)
                }
            }
            None => Fill::Solid(l.color.unwrap_or(Rgba::WHITE)),
        }),
        "image" => {
            let src = scene.resolve(l.src.as_deref().ok_or_else(|| anyhow!("image needs \"src\""))?);
            let (iw, ih) = assets::image_size(&src).with_context(|| format!("reading {}", src.display()))?;
            let fit = parse_fit(l.fit.as_deref())?;
            match (w, h) {
                (Some(_), None) => h = Some(w.unwrap() * ih as f32 / iw as f32),
                (None, Some(_)) => w = Some(h.unwrap() * iw as f32 / ih as f32),
                _ => {}
            }
            let (bw, bh) = (w.unwrap_or(pbox.0) as f64, h.unwrap_or(pbox.1) as f64);
            let s0 = match fit {
                Fit::Cover => (bw / iw as f64).max(bh / ih as f64),
                Fit::Contain => (bw / iw as f64).min(bh / ih as f64),
                Fit::Fill => (bw / iw as f64).max(bh / ih as f64),
            };
            let s = s0 * zoom.max_abs().max(1.0) * max_scale * rs;
            let tw = (iw as f64 * s).ceil().min(iw as f64) as u32;
            let th = (ih as f64 * s).ceil().min(ih as f64) as u32;
            // blur is specified in screen px; convert to the loaded image's px
            let blur_px = if l.blur > 0.0 { l.blur * (tw as f64 / (iw as f64 * s0 * rs)).min(1.0) * rs } else { 0.0 };
            Kind::Image { key: (src, tw.max(1), th.max(1), (blur_px * 100.0) as u32), pm: None, fit }
        }
        "video" => {
            let src = scene.resolve(l.src.as_deref().ok_or_else(|| anyhow!("video needs \"src\""))?);
            let info = ctx.probes.get(&src).cloned().ok_or_else(|| anyhow!("could not probe {}", src.display()))?;
            if !info.has_video {
                bail!("{} has no video stream", src.display());
            }
            let fit = parse_fit(l.fit.as_deref())?;
            let (iw, ih) = (info.width as f64, info.height as f64);
            match (w, h) {
                (Some(_), None) => h = Some(w.unwrap() * (ih / iw) as f32),
                (None, Some(_)) => w = Some(h.unwrap() * (iw / ih) as f32),
                _ => {}
            }
            let (bw, bh) = (w.unwrap_or(pbox.0) as f64, h.unwrap_or(pbox.1) as f64);
            let s0 = match fit {
                Fit::Contain => (bw / iw).min(bh / ih),
                _ => (bw / iw).max(bh / ih),
            };
            let s = (s0 * zoom.max_abs().max(1.0) * max_scale * rs).min(1.0);
            let even = |v: f64| ((v / 2.0).ceil() as u32 * 2).max(2);
            if !l.looped {
                natural = Some((info.duration - l.trim).max(0.0));
            }
            let vol = l.volume.unwrap_or(1.0);
            if vol > 0.0 && info.has_audio {
                // duration filled in after the node end is known (see below)
                ctx.audio.push(AudioInput { path: src.clone(), start, trim: l.trim, dur: f64::NAN, volume: vol, fade_in: l.fade_in, fade_out: l.fade_out, looped: l.looped });
            }
            Kind::Video { src, dw: even(iw * s), dh: even(ih * s), info, trim: l.trim, looped: l.looped, fit }
        }
        "text" => {
            let raw = l.text.clone().ok_or_else(|| anyhow!("text needs \"text\""))?;
            let raw = if l.uppercase { raw.to_uppercase() } else { raw };
            let style = text_style(l, ctx, None, 64.0, q)?;
            let lay = text::layout(&style, &text::tokenize(&raw));
            let sprite = rescale(text::render(&style, &lay, None, None)?, q);
            let pieces = match l.reveal.as_deref() {
                None | Some("none") => None,
                Some(mode @ ("words" | "lines")) => {
                    let mut st = style.clone();
                    st.bg = None;
                    let (bx, by) = style.bg.map(|b| (b.1, b.2)).unwrap_or((0.0, 0.0));
                    let mut v = Vec::new();
                    if mode == "words" {
                        for pw in &lay.words {
                            let s = rescale(text::render_word(&st, &pw.text, st.color, None)?, q);
                            v.push((Arc::new(s), (bx + pw.x) / q, (by + pw.y) / q));
                        }
                    } else {
                        for (li, (lx, ly, _)) in lay.lines.iter().enumerate() {
                            let t: Vec<&str> = lay.words.iter().filter(|w| w.line == li).map(|w| w.text.as_str()).collect();
                            if t.is_empty() {
                                continue;
                            }
                            let s = rescale(text::render_word(&st, &t.join(" "), st.color, None)?, q);
                            v.push((Arc::new(s), (bx + lx) / q, (by + ly) / q));
                        }
                    }
                    Some((v, l.reveal_duration.unwrap_or(0.8)))
                }
                Some(o) => bail!("unknown reveal '{o}' (words, lines)"),
            };
            w = Some(sprite.w);
            h = Some(sprite.h);
            Kind::Text { sprite: Arc::new(sprite), pieces }
        }
        "captions" => {
            let src = scene.resolve(l.src.as_deref().ok_or_else(|| anyhow!("captions need \"src\" (a transcript .json/.srt/.vtt)"))?);
            let kind = CapKind::parse(l.style.as_deref())?;
            let mut l2 = l.clone();
            if l2.shadow.is_none() {
                l2.shadow = Some(crate::scene::Shadow { color: Rgba::rgba8(0, 0, 0, 0.65), blur: 10.0, x: 0.0, y: 4.0 });
            }
            let size = l.size.unwrap_or((scene.width.min(scene.height) as f64 * 0.065).round());
            let bw = w.unwrap_or_else(|| l.max_width.map(|m| m as f32).unwrap_or(pbox.0 * 0.86));
            l2.max_width = Some(bw as f64);
            let q = rs as f32;
            let style = text_style(&l2, ctx, Some("Segoe UI Black"), size, q)?;
            let bh = h.unwrap_or((size * 1.15 * 2.6) as f32);
            w = Some(bw);
            h = Some(bh);
            let words: Vec<captions::Word> = captions::load_transcript(&src)?
                .into_iter()
                .map(|wd| captions::Word { start: wd.start - l.trim, end: wd.end - l.trim, ..wd })
                .collect();
            let dur_guess = match (l.end, l.duration) {
                (Some(e), _) => base + e - start,
                (_, Some(d)) => d,
                _ => if default_end.is_finite() { default_end - start } else { words.last().map(|w| w.end + 1.0).unwrap_or(0.0) },
            };
            if !(l.end.is_some() || l.duration.is_some() || default_end.is_finite()) {
                natural = Some(dur_guess);
            }
            let opts = CapOptions {
                kind,
                style,
                highlight: l.highlight.unwrap_or(Rgba::rgba8(255, 214, 10, 1.0)),
                highlight_bg: l.highlight_bg,
                max_words: l.max_words.unwrap_or(match kind {
                    CapKind::Pop => 3,
                    CapKind::Karaoke => 5,
                    CapKind::Word => 1,
                    CapKind::Line => 9,
                }),
                uppercase: l.uppercase,
                box_w: bw * q,
                box_h: bh * q,
                q,
            };
            let caps = captions::build(&words, dur_guess, &opts)?;
            Kind::Captions(Arc::new(caps))
        }
        "waveform" => {
            let src = scene.resolve(l.src.as_deref().ok_or_else(|| anyhow!("waveform needs \"src\" (audio file)"))?);
            let dur = match (l.end, l.duration) {
                (Some(e), _) => base + e - start,
                (_, Some(d)) => d,
                _ if default_end.is_finite() => default_end - start,
                _ => {
                    let d = ffmpeg::probe(&src)?.duration - l.trim;
                    natural = Some(d);
                    d
                }
            };
            let spec = waveform::analyse(&src, l.trim, dur.max(0.04), scene.fps, l.bars.unwrap_or(48).clamp(2, 512), l.gain.unwrap_or(1.0) as f32)?;
            Kind::Waveform { spec: Arc::new(spec), color: l.color.unwrap_or(Rgba::WHITE), mirror: l.mirror, gap: l.gap.unwrap_or(0.35).clamp(0.0, 0.95) as f32 }
        }
        "" => bail!("layer without \"type\""),
        o => bail!("unknown layer type '{o}'"),
    };

    let w = w.unwrap_or(pbox.0);
    let h = h.unwrap_or(pbox.1);
    let kind = match kind {
        Kind::Rect(f @ (Fill::Linear(..) | Fill::Radial(..))) => {
            // gradients are smooth: half resolution is plenty
            let q = ((max_scale * rs) as f32 * 0.5).clamp(0.05, 1.0);
            Kind::Rect(gradient_pixmap(&f, w, h, q).map(|pm| Fill::Bitmap(Arc::new(pm))).unwrap_or(f))
        }
        k => k,
    };
    let mut end = match (l.end, l.duration, natural) {
        (Some(e), _, _) => base + e,
        (None, Some(d), _) => start + d,
        (None, None, Some(n)) => start + n,
        _ => default_end,
    };
    // a child never outlives its parent
    end = end.min(default_end);
    if l.kind == "group" {
        children = build_list(&l.children, l.sequence, l.overlap, start, end, (w, h), max_scale, ctx)?;
        if !end.is_finite() {
            end = children.iter().map(|c| c.end).filter(|e| e.is_finite()).fold(start, f64::max);
        }
    }
    // patch duration of this video's audio now that the end is known
    if l.kind == "video" {
        for a in ctx.audio.iter_mut().filter(|a| a.dur.is_nan()) {
            a.dur = end - start;
        }
    }

    let border = l.border.as_ref().map(|b| (b.width as f32, b.color));
    let radius = l.radius as f32;
    let shadow = match (&l.shadow, l.kind.as_str()) {
        (Some(s), "image" | "video" | "rect" | "group") => {
            let bw = border.map(|b| b.0).unwrap_or(0.0);
            box_shadow(w + bw * 2.0, h + bw * 2.0, radius + bw, s.color, s.blur as f32, s.x as f32, s.y as f32).map(Arc::new)
        }
        _ => None,
    };

    Ok(Node {
        id,
        kind,
        start,
        end,
        w,
        h,
        x: prop(&l.x, pbox.0 as f64 / 2.0),
        y: prop(&l.y, pbox.1 as f64 / 2.0),
        anchor: l.anchor.map(|a| (a[0] as f32, a[1] as f32)).unwrap_or((0.5, 0.5)),
        scale: prop(&l.scale, 1.0),
        scale_x: prop(&l.scale_x, 1.0),
        scale_y: prop(&l.scale_y, 1.0),
        rotation: prop(&l.rotation, 0.0),
        opacity: prop(&l.opacity, 1.0),
        zoom,
        pan_x: prop(&l.pan_x, 0.0),
        pan_y: prop(&l.pan_y, 0.0),
        enter: l.enter.clone(),
        exit: l.exit.clone(),
        radius,
        border,
        shadow,
        children,
    })
}

// ---------------------------------------------------------------- drawing

/// Per-worker state: open video decoders.
#[derive(Default)]
pub struct VideoState {
    readers: HashMap<usize, (f64, VideoReader)>,
}

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
    for n in &p.nodes {
        advance_videos(n, t, frame, p.fps, vs)?;
    }
    vs.readers.retain(|_, (end, _)| *end > t);
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
    if t < n.start || t >= n.end {
        return Ok(());
    }
    if let Kind::Video { src, info, dw, dh, trim, looped, .. } = &n.kind {
        let lf = frame - (n.start * fps).round() as i64;
        let need_new = match vs.readers.get(&n.id) {
            None => true,
            Some((_, r)) => lf < r.cur || lf > r.cur + (fps * 2.0) as i64 || lf < r.first(),
        };
        if need_new {
            let r = VideoReader::spawn(src, info, *trim, lf.max(0), fps, *dw, *dh, *looped)?;
            vs.readers.insert(n.id, (n.end, r));
        }
        vs.readers.get_mut(&n.id).unwrap().1.seek_forward(lf)?;
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
fn media_op<'a>(src: Src<'a>, iw: f32, ih: f32, fit: Fit, w: f32, h: f32, zoom: f32, pan: (f32, f32), radius: f32, ts: Transform, opacity: f32, clip: Option<Clip>) -> Option<Op<'a>> {
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
fn draw_node<'a>(painter: &mut dyn Painter<'a>, n: &'a Node, t: f64, parent: Transform, parent_op: f32, pclip: Option<Clip>, ctx: &FrameCtx<'a>) {
    if t < n.start || t >= n.end {
        return;
    }
    let lt = t - n.start;
    let dur = n.end - n.start;
    let mut fx = Fx::default();
    if let Some(tr) = &n.enter {
        tr.apply_in(lt, &mut fx);
    }
    if let Some(tr) = &n.exit {
        tr.apply_out(dur - lt, &mut fx);
    }
    let op = parent_op * n.opacity.eval(lt, dur).clamp(0.0, 1.0) as f32 * fx.opacity;
    if op <= 0.002 {
        return;
    }
    let x = n.x.eval(lt, dur) + fx.dx;
    let y = n.y.eval(lt, dur) + fx.dy;
    let s = n.scale.eval(lt, dur) * fx.scale;
    let (sx, sy) = ((s * n.scale_x.eval(lt, dur)) as f32, (s * n.scale_y.eval(lt, dur)) as f32);
    if sx.abs() < 1e-4 || sy.abs() < 1e-4 {
        return;
    }
    let rot = n.rotation.eval(lt, dur) as f32;
    let (w, h) = (n.w, n.h);
    let mut ts = parent.pre_translate(x as f32, y as f32);
    if rot != 0.0 {
        ts = ts.pre_concat(Transform::from_rotate(rot));
    }
    let ts = ts.pre_scale(sx, sy).pre_translate(-n.anchor.0 * w, -n.anchor.1 * h);
    let clip = match fx.clip {
        Some(c) => {
            let (cx, cy, cw, ch) = c.rect(w as f64, h as f64);
            Some(Clip { ts, rect: [cx as f32, cy as f32, (cx + cw) as f32, (cy + ch) as f32] })
        }
        None => pclip,
    };

    // shadow + border behind box-like layers
    let bw = n.border.map(|b| b.0).unwrap_or(0.0);
    if let Some(sh) = &n.shadow {
        painter.draw(sprite_op(sh, ts.pre_translate(-bw, -bw), op, clip));
    }
    if let Some((bw, bc)) = n.border {
        painter.draw(Op { src: Src::Solid(bc), ts, shape: [-bw, -bw, w + bw, h + bw], radius: n.radius + bw, uv: Transform::identity(), opacity: op, clip });
    }

    match &n.kind {
        Kind::Group => {
            for c in &n.children {
                draw_node(painter, c, t, ts, op, clip, ctx);
            }
        }
        Kind::Rect(fill) => match fill {
            Fill::Solid(c) => painter.draw(Op { src: Src::Solid(*c), ts, shape: [0.0, 0.0, w, h], radius: n.radius, uv: Transform::identity(), opacity: op, clip }),
            Fill::Bitmap(pm) => {
                let (kx, ky) = (w / pm.width() as f32, h / pm.height() as f32);
                painter.draw(Op { src: Src::Image(pm), ts, shape: [0.0, 0.0, w, h], radius: n.radius, uv: Transform::from_scale(kx, ky), opacity: op, clip });
            }
            _ => {}
        },
        Kind::Image { pm, fit, .. } => {
            if let Some(pm) = pm {
                let zoom = n.zoom.eval(lt, dur) as f32;
                let pan = (n.pan_x.eval(lt, dur) as f32, n.pan_y.eval(lt, dur) as f32);
                if let Some(o) = media_op(Src::Image(pm), pm.width() as f32, pm.height() as f32, *fit, w, h, zoom, pan, n.radius, ts, op, clip) {
                    painter.draw(o);
                }
            }
        }
        Kind::Video { fit, .. } => {
            if let Some((_, r)) = ctx.videos.readers.get(&n.id) {
                let zoom = n.zoom.eval(lt, dur) as f32;
                let pan = (n.pan_x.eval(lt, dur) as f32, n.pan_y.eval(lt, dur) as f32);
                let src = Src::Frame { key: n.id, ver: r.cur, data: &r.frame, w: r.w, h: r.h };
                if let Some(o) = media_op(src, r.w as f32, r.h as f32, *fit, w, h, zoom, pan, n.radius, ts, op, clip) {
                    painter.draw(o);
                }
            }
        }
        Kind::Text { sprite, pieces } => match pieces {
            None => painter.draw(sprite_op(sprite, ts, op, clip)),
            Some((pieces, rd)) => {
                let step = rd / pieces.len().max(1) as f64;
                for (i, (s, px, py)) in pieces.iter().enumerate() {
                    let p = ((lt - i as f64 * step) / 0.35).clamp(0.0, 1.0);
                    if p <= 0.0 {
                        continue;
                    }
                    let e = crate::anim::Ease::Out.apply(p) as f32;
                    painter.draw(sprite_op(s, ts.pre_translate(*px, *py + (1.0 - e) * s.h * 0.35), op * e, clip));
                }
            }
        },
        Kind::Captions(c) => c.draw(painter, lt, ts, op, clip),
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
                    painter.draw(Op { src: Src::Solid(*color), ts, shape: [x, y, x + bwid, y + bh], radius: r, uv: Transform::identity(), opacity: op, clip });
                }
            }
        }
    }
}
