//! Turns a Scene into a tree of prepared nodes: every static pixel (images at their
//! on-screen size, blurred backdrops, text, shadows, gradients) is rendered once here.

pub mod captions;
pub mod draw;
pub mod waveform;

pub use draw::{advance, draw_frame, render_frame, VideoState};

use crate::gfx::{box_shadow, Sprite};
use crate::media::colorspace::{Adjust, NO_ADJUST};
use crate::media::{self, image as assets, MediaInfo};
use crate::scene::anim::{Prop, TKind, Transition};
use crate::scene::color::Rgba;
use crate::scene::{Layer, Scene};
use crate::text::{self, Align, FontDb, TextStyle};
use anyhow::{anyhow, bail, Context, Result};
use captions::{CapKind, CapOptions, Captions};
use draw::gradient_pixmap;
use rayon::prelude::*;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tiny_skia::Pixmap;
use waveform::Spectrum;

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

/// (file, width, height, blur*100, colour adjustment*1000)
type ImgKey = (PathBuf, u32, u32, u32, [i32; 4]);

pub enum Kind {
    Group,
    Rect(Fill),
    Image { key: ImgKey, pm: Option<Arc<Pixmap>>, fit: Fit },
    Video { src: PathBuf, info: MediaInfo, dw: u32, dh: u32, trim: f64, looped: bool, fit: Fit, speed: f64, adjust: Adjust },
    Text { sprite: Arc<Sprite>, reveal: Option<Reveal> },
    Counter(Arc<Counter>),
    Captions(Arc<Captions>),
    Waveform { spec: Arc<Spectrum>, color: Rgba, mirror: bool, gap: f32 },
}

/// Text that appears piece by piece (words, lines or characters).
pub struct Reveal {
    /// sprite + its position in the text box
    pub pieces: Vec<(Arc<Sprite>, f32, f32)>,
    /// seconds over which all pieces appear
    pub duration: f64,
    /// typewriter: pieces pop in without a fade/slide
    pub instant: bool,
}

/// An animated number ("1.234 bezoekers"), drawn from per-character sprites.
pub struct Counter {
    pub value: Prop,
    pub decimals: usize,
    pub prefix: Option<Arc<Sprite>>,
    pub suffix: Option<Arc<Sprite>>,
    pub separator: Option<char>,
    pub decimal_mark: char,
    /// sprites for digits, separators and minus sign
    pub glyphs: HashMap<char, Arc<Sprite>>,
    /// every digit gets the width of the widest one so the number doesn't jitter
    pub digit_w: f32,
    pub align: Align,
}

impl Counter {
    pub fn format(&self, v: f64) -> String {
        let s = format!("{:.*}", self.decimals, v.abs());
        let (int, frac) = s.split_once('.').map(|(a, b)| (a.to_string(), Some(b.to_string()))).unwrap_or((s.clone(), None));
        let mut out = String::new();
        if v < 0.0 && v.abs() >= 0.5 * 10f64.powi(-(self.decimals as i32)) {
            out.push('-');
        }
        for (i, ch) in int.chars().enumerate() {
            if i > 0 && (int.len() - i) % 3 == 0 {
                if let Some(sep) = self.separator {
                    out.push(sep);
                }
            }
            out.push(ch);
        }
        if let Some(f) = frac {
            out.push(self.decimal_mark);
            out += &f;
        }
        out
    }

    fn char_w(&self, c: char) -> f32 {
        if c.is_ascii_digit() {
            self.digit_w
        } else {
            self.glyphs.get(&c).map(|g| g.w).unwrap_or(0.0)
        }
    }

    pub fn width(&self, text: &str) -> f32 {
        self.prefix.as_ref().map(|s| s.w).unwrap_or(0.0)
            + text.chars().map(|c| self.char_w(c)).sum::<f32>()
            + self.suffix.as_ref().map(|s| s.w).unwrap_or(0.0)
    }

    /// (sprite, x) for every piece of the formatted value, starting at x = 0.
    pub fn layout(&self, text: &str) -> Vec<(&Arc<Sprite>, f32)> {
        let mut out = vec![];
        let mut x = 0.0;
        if let Some(p) = &self.prefix {
            out.push((p, x));
            x += p.w;
        }
        for c in text.chars() {
            let cw = self.char_w(c);
            if let Some(g) = self.glyphs.get(&c) {
                out.push((g, x + (cw - g.w) / 2.0));
            }
            x += cw;
        }
        if let Some(s) = &self.suffix {
            out.push((s, x));
        }
        out
    }
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
    /// clip children to the box (groups)
    clip: bool,
    pub children: Vec<Node>,
}

#[derive(Clone, Debug)]
pub struct AudioInput {
    pub path: PathBuf,
    /// scene time where it starts
    pub start: f64,
    /// seconds skipped in the source
    pub trim: f64,
    /// duration in the scene (source plays `dur * speed` seconds)
    pub dur: f64,
    pub volume: Prop,
    pub fade_in: f64,
    pub fade_out: f64,
    pub looped: bool,
    pub speed: f64,
    /// duck under the other audio to this level
    pub duck: Option<f64>,
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
    /// master loudness target (LUFS)
    pub loudness: Option<f64>,
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
        let probes: HashMap<PathBuf, MediaInfo> =
            media.par_iter().map(|p| media::probe(p).map(|i| (p.clone(), i))).collect::<Result<_>>()?;
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
                ctx.audio.push(AudioInput {
                    path,
                    start: a.start,
                    trim: a.trim,
                    dur,
                    volume: a.volume.clone().unwrap_or(Prop::Const(1.0)),
                    fade_in: a.fade_in,
                    fade_out: a.fade_out,
                    looped: a.looped,
                    speed: 1.0,
                    duck: a.duck.as_ref().and_then(|d| d.level()),
                });
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
                assets::load_image(&k.0, k.1 as f64, k.2 as f64, k.3 as f64 / 100.0, &k.4.map(|v| v as f32 / 1000.0))
                    .map(|pm| (k.clone(), Arc::new(pm)))
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
            loudness: scene.loudness,
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
fn build_list(
    layers: &[Layer],
    sequence: bool,
    overlap: f64,
    base: f64,
    default_end: f64,
    pbox: (f32, f32),
    pscale: f64,
    ctx: &mut Ctx,
) -> Result<Vec<Node>> {
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
            let adj = l.filter.map(|f| f.adjust()).unwrap_or(NO_ADJUST).map(|v| (v * 1000.0).round() as i32);
            Kind::Image { key: (src, tw.max(1), th.max(1), (blur_px * 100.0) as u32, adj), pm: None, fit }
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
            let speed = l.speed.unwrap_or(1.0);
            if !(0.05..=20.0).contains(&speed) {
                bail!("video speed must be between 0.05 and 20");
            }
            if !l.looped {
                natural = Some(((info.duration - l.trim) / speed).max(0.0));
            }
            let vol = l.volume.clone().unwrap_or(Prop::Const(1.0));
            if vol.max_abs() > 0.0 && info.has_audio {
                // duration filled in after the node end is known (see below)
                ctx.audio.push(AudioInput {
                    path: src.clone(),
                    start,
                    trim: l.trim,
                    dur: f64::NAN,
                    volume: vol,
                    fade_in: l.fade_in,
                    fade_out: l.fade_out,
                    looped: l.looped,
                    speed,
                    duck: None,
                });
            }
            Kind::Video {
                src,
                dw: even(iw * s),
                dh: even(ih * s),
                info,
                trim: l.trim,
                looped: l.looped,
                fit,
                speed,
                adjust: l.filter.map(|f| f.adjust()).unwrap_or(NO_ADJUST),
            }
        }
        "text" => {
            let raw = l.text.clone().ok_or_else(|| anyhow!("text needs \"text\""))?;
            let raw = if l.uppercase { raw.to_uppercase() } else { raw };
            let style = text_style(l, ctx, None, 64.0, q)?;
            let lay = text::layout(&style, &text::tokenize(&raw));
            let sprite = rescale(text::render(&style, &lay, None, None)?, q);
            let reveal = match l.reveal.as_deref() {
                None | Some("none") => None,
                Some(mode @ ("words" | "lines" | "chars" | "typewriter")) => {
                    let mut st = style.clone();
                    st.bg = None;
                    let (bx, by) = style.bg.map(|b| (b.1, b.2)).unwrap_or((0.0, 0.0));
                    let mut v = Vec::new();
                    match mode {
                        "words" => {
                            for pw in &lay.words {
                                let s = rescale(text::render_word(&st, &pw.text, st.color, None)?, q);
                                v.push((Arc::new(s), (bx + pw.x) / q, (by + pw.y) / q));
                            }
                        }
                        "lines" => {
                            for (li, (lx, ly, _)) in lay.lines.iter().enumerate() {
                                let t: Vec<&str> = lay.words.iter().filter(|w| w.line == li).map(|w| w.text.as_str()).collect();
                                if t.is_empty() {
                                    continue;
                                }
                                let s = rescale(text::render_word(&st, &t.join(" "), st.color, None)?, q);
                                v.push((Arc::new(s), (bx + lx) / q, (by + ly) / q));
                            }
                        }
                        _ => {
                            // characters: one sprite per distinct char, placed at its prefix width
                            let mut cache: HashMap<char, Arc<Sprite>> = HashMap::new();
                            for pw in &lay.words {
                                for (k, ch) in pw.text.char_indices() {
                                    let sp = match cache.get(&ch) {
                                        Some(s) => s.clone(),
                                        None => {
                                            let s = Arc::new(rescale(text::render_word(&st, &ch.to_string(), st.color, None)?, q));
                                            cache.insert(ch, s.clone());
                                            s
                                        }
                                    };
                                    let x = pw.x + st.measure(&pw.text[..k]) + if k > 0 { st.letter_spacing } else { 0.0 };
                                    v.push((sp, (bx + x) / q, (by + pw.y) / q));
                                }
                            }
                        }
                    }
                    let per = if mode == "typewriter" { 0.06 } else { 0.0 };
                    Some(Reveal {
                        duration: l.reveal_duration.unwrap_or(if per > 0.0 { v.len() as f64 * per } else { 0.8 }),
                        pieces: v,
                        instant: mode == "typewriter",
                    })
                }
                Some(o) => bail!("unknown reveal '{o}' (words, lines, chars, typewriter)"),
            };
            w = Some(sprite.w);
            h = Some(sprite.h);
            Kind::Text { sprite: Arc::new(sprite), reveal }
        }
        "counter" => {
            let value = l.value.clone().ok_or_else(|| anyhow!("counter needs \"value\", e.g. {{\"from\": 0, \"to\": 1500}}"))?;
            let decimals = l.decimals.unwrap_or(0).min(6);
            let separator = match l.separator.as_deref() {
                None => Some('.'),
                Some("") | Some("none") => None,
                Some(s) => s.chars().next(),
            };
            let decimal_mark = if separator == Some(',') { '.' } else { ',' };
            let style = text_style(l, ctx, None, 96.0, q)?;
            let glyph = |t: &str| -> Result<Arc<Sprite>> { Ok(Arc::new(rescale(text::render_word(&style, t, style.color, None)?, q))) };
            let mut glyphs = HashMap::new();
            for c in "0123456789-".chars().chain(separator).chain(std::iter::once(decimal_mark)) {
                glyphs.insert(c, glyph(&c.to_string())?);
            }
            let digit_w = "0123456789".chars().map(|c| glyphs[&c].w).fold(0.0, f32::max);
            let counter = Counter {
                prefix: l.prefix.as_deref().filter(|p| !p.is_empty()).map(glyph).transpose()?,
                suffix: l.suffix.as_deref().filter(|p| !p.is_empty()).map(glyph).transpose()?,
                value,
                decimals,
                separator,
                decimal_mark,
                glyphs,
                digit_w,
                align: Align::parse(l.align.as_deref())?,
            };
            // the box fits the widest value the animation reaches
            let widest = [counter.value.eval(0.0, 1.0), counter.value.eval(1e9, 1.0), counter.value.max_abs()]
                .iter()
                .map(|v| counter.width(&counter.format(*v)))
                .fold(0.0, f32::max);
            w = Some(w.unwrap_or(widest));
            h = Some(h.unwrap_or(counter.glyphs[&'0'].h));
            Kind::Counter(Arc::new(counter))
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
                _ => {
                    if default_end.is_finite() {
                        default_end - start
                    } else {
                        words.last().map(|w| w.end + 1.0).unwrap_or(0.0)
                    }
                }
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
                    let d = media::probe(&src)?.duration - l.trim;
                    natural = Some(d);
                    d
                }
            };
            let spec = waveform::analyse(
                &src,
                l.trim,
                dur.max(0.04),
                scene.fps,
                l.bars.unwrap_or(48).clamp(2, 512),
                l.gain.unwrap_or(1.0) as f32,
            )?;
            Kind::Waveform {
                spec: Arc::new(spec),
                color: l.color.unwrap_or(Rgba::WHITE),
                mirror: l.mirror,
                gap: l.gap.unwrap_or(0.35).clamp(0.0, 0.95) as f32,
            }
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
        clip: l.clip,
        children,
    })
}

// ---------------------------------------------------------------- drawing
