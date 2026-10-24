//! `flits reel`: raw footage to a vertical reel in one render.
//!
//! One transcript (reused when it exists), the chosen part(s) of the recording with silences and
//! filler words cut out, reframed to 9:16 (or any format), captions timed to the cut, an optional
//! hook title: all in a single scene, so the footage is decoded and encoded once and only for the
//! length of the reel (`flits cut` + `flits captions` rendered the whole recording twice and
//! transcribed it twice).

use super::{abs, canvas_for};
use crate::compose::captions::Word;
use crate::media;
use crate::media::reframe::{pan_for, Framing};
use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::path::Path;

pub struct ReelOpts<'a> {
    pub video: &'a Path,
    /// kept source ranges, in playback order
    pub ranges: &'a [(f64, f64)],
    /// the cut soundtrack (made with `cut::cut_audio` from the same ranges)
    pub audio: &'a Path,
    /// the remapped transcript (written by [`write_words`])
    pub words: &'a Path,
    pub format: String,
    /// horizontal point of interest, 0 (left) .. 1 (right), kept in frame when cropping
    pub focus: f64,
    /// per-shot points of interest (from `media::reframe`), overriding `focus`
    pub framings: Option<&'a [Framing]>,
    /// `crop` (fill the frame) or `fit` (whole picture, captions on the free space)
    pub fit: String,
    /// `fit`: a still of the footage, shown blurred behind the picture
    pub backdrop: Option<&'a Path>,
    /// frame rate (default: the footage's, halved above 30: 50 -> 25, 60 -> 30)
    pub fps: Option<f64>,
    /// punch in on every other jump cut, so cut-out pauses don't read as hiccups
    pub punch: bool,
    /// music under the voice (ducked while someone speaks, faded out at the end)
    pub music: Option<&'a Path>,
    /// logo in the top corner (png/svg)
    pub logo: Option<&'a Path>,
    /// a thin progress bar along the bottom
    pub progress: bool,
    /// accent colour (progress bar; captions highlight when none is given)
    pub accent: Option<String>,
    pub title: Option<String>,
    pub preset: Option<String>,
    pub highlight: Option<String>,
    /// captions position as a fraction of the height
    pub position: Option<f64>,
    /// a closing card after the last clip
    pub end: Option<EndCard<'a>>,
}

/// The end card: logo and/or a call to action on a blurred still of the last frame.
pub struct EndCard<'a> {
    /// call to action ("Volg ons", "Follow for more")
    pub text: Option<String>,
    pub logo: Option<&'a Path>,
    /// a still of the last frame (blurred behind the card); none: the accent colour
    pub still: Option<&'a Path>,
    /// seconds (1.5..2.5 reads well)
    pub duration: f64,
}

/// Map words from source time to reel time: a word plays in the range that holds its middle;
/// words in removed parts go.
pub fn remap_words(words: &[Word], ranges: &[(f64, f64)]) -> Vec<Word> {
    let mut out = Vec::new();
    let mut offset = 0.0;
    for &(a, b) in ranges {
        for w in words {
            let mid = (w.start + w.end) / 2.0;
            if mid >= a && mid < b {
                let mut w = w.clone();
                w.start = offset + (w.start.max(a) - a);
                w.end = offset + (w.end.min(b) - a);
                out.push(w);
            }
        }
        offset += b - a;
    }
    out
}

/// Write words as a flat transcript (`{"words": [...]}`), the format the captions layer reads.
pub fn write_words(words: &[Word], language: Option<&str>, path: &Path) -> Result<()> {
    let ws: Vec<Value> = words
        .iter()
        .map(|w| {
            let mut v = json!({"word": w.text, "start": w.start, "end": w.end});
            if let Some(s) = &w.speaker {
                v["speaker"] = json!(s);
            }
            v
        })
        .collect();
    std::fs::write(path, serde_json::to_string(&json!({"language": language, "words": ws}))?)?;
    Ok(())
}

/// Keep only the parts of `ranges` inside the windows (in window order).
pub fn within(ranges: &[(f64, f64)], windows: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let mut out = Vec::new();
    for &(wa, wb) in windows {
        for &(a, b) in ranges {
            let (s, e) = (a.max(wa), b.min(wb));
            if e - s > 0.05 {
                out.push((s, e));
            }
        }
    }
    out
}

/// Social video plays at 30 fps at most: footage above that is halved (an exact frame step, no
/// judder), which also halves the work.
pub fn reel_fps(source: f64) -> f64 {
    let f = if source > 1.0 && source <= 240.0 { (source * 1000.0).round() / 1000.0 } else { 30.0 };
    if f > 31.0 && f / 2.0 >= 23.0 {
        f / 2.0
    } else {
        f.min(30.0)
    }
}

/// Whether clip `i` is punched in: jump cuts alternate between the wide and the punched framing,
/// counting from the last clip that isn't a jump cut.
fn punched(ranges: &[(f64, f64)], i: usize) -> bool {
    let mut n = 0;
    let mut k = i;
    while k > 0 && ranges[k].0 >= ranges[k - 1].1 && ranges[k].0 - ranges[k - 1].1 < 4.0 {
        n += 1;
        k -= 1;
    }
    n % 2 == 1
}

/// Fraction of the source width a cover crop to `canvas` shows (1 when nothing is cut sideways).
pub fn visible_fraction(canvas: (f64, f64), source: (f64, f64)) -> f64 {
    ((canvas.0 / canvas.1) / (source.0 / source.1.max(1.0))).min(1.0)
}

/// One layer of a clip: from `at` seconds into the clip (in the reel), source from `a + trim`.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Part {
    at: f64,
    trim: f64,
    pan: f64,
}

/// The framings of a clip playing source `[a, b)` from `start` in the reel: the framing of the
/// shot it starts in, then one part per later cut inside the clip; each part is its own layer with
/// a constant framing, which lets the decoder cut out just the part that shows.
///
/// The reel's frames are what matters: the clip starts wherever the clips before it end (rarely on
/// a frame), and its frame `n` shows source `a + n / fps` from the frame nearest its start. A part
/// starts just before the reel frame that first shows the new shot, with its source on that
/// frame's, so the parts play as the one clip would, each frame with its own shot's framing.
fn clip_parts(framings: &[Framing], (a, b): (f64, f64), start: f64, fps: f64, pan_of: impl Fn(f64) -> f64) -> Vec<Part> {
    let at = |t: f64| framings.iter().rev().find(|f| f.start <= t + 1e-6).or(framings.first()).map(|f| f.x).unwrap_or(0.5);
    let first = (start * fps).round();
    let mut parts = vec![Part { at: 0.0, trim: 0.0, pan: pan_of(at(a)) }];
    for f in framings.iter().filter(|f| f.start > a + 1e-6 && f.start < b) {
        // the clip's first frame showing source at or after the cut, and that frame in the reel
        let n = ((f.start - a) * fps - 1e-6).ceil();
        let reel_frame = first + n;
        // a hair before that frame: the layer starts on it, whatever the rounding of the sums
        let part = Part { at: (reel_frame - 1e-3) / fps - start, trim: n / fps, pan: pan_of(f.x) };
        let last = parts.last_mut().unwrap();
        if (last.pan - part.pan).abs() < 1e-9 || part.at >= b - a {
            continue;
        }
        if part.at <= last.at {
            last.pan = part.pan; // two cuts within one frame: the later framing wins
        } else {
            parts.push(part);
        }
    }
    parts
}

/// Reels: the app UI covers about the top 12% and the bottom 15% of the picture.
const SAFE_TOP: f64 = 0.12;
const SAFE_BOTTOM: f64 = 0.85;

/// Where the captions go (centre of the block, fraction of the height) when a face is in
/// `protect` (top, bottom; fractions): the first of a few fixed slots that keeps clear of it
/// (the default, as low as the app UI allows, near the top), else the one covering least.
/// `half` = half the height of a two-line caption block; `top_ok`: the top slot is free (no
/// hook title there).
fn caption_slot(default: f64, half: f64, protect: Option<(f64, f64)>, top_ok: bool) -> f64 {
    let Some((y0, y1)) = protect else { return default };
    // the face box ends about at the mouth: keep the chin clear too, plus a margin
    let (y0, y1) = (y0 - 0.02, y1 + (y1 - y0) * 0.3 + 0.02);
    let low = (SAFE_BOTTOM - half).max(default);
    let high = SAFE_TOP + half + 0.01;
    let mut slots = vec![default, low];
    if top_ok {
        slots.push(high);
    }
    let cover = |c: f64| ((c + half).min(y1) - (c - half).max(y0)).max(0.0);
    let mut best = default;
    for s in slots {
        if cover(s) < 1e-9 {
            return s;
        }
        if cover(s) < cover(best) - 1e-9 {
            best = s;
        }
    }
    best
}

/// `y` of the captions layer (px): per shot, a slot that keeps the caption block clear of the
/// faces in the picture; a jump (hold keyframe) exactly on the cut where the slot changes.
/// `zooms`: zoom of each clip; `vscale`: shown height of the cover-fitted video / canvas height.
#[allow(clippy::too_many_arguments)]
fn caption_y(
    framings: &[Framing],
    ranges: &[(f64, f64)],
    zooms: &[f64],
    vscale: f64,
    hf: f64,
    default: f64,
    half: f64,
    title: f64,
) -> Value {
    let at = |t: f64| framings.iter().rev().find(|f| f.start <= t + 1e-6).or(framings.first()).and_then(|f| f.band);
    let mut keys: Vec<(f64, f64)> = vec![];
    let mut offset = 0.0;
    for (i, &(a, b)) in ranges.iter().enumerate() {
        let k = vscale * zooms.get(i).copied().unwrap_or(1.0);
        let shown = |band: Option<(f64, f64)>| band.map(|(y0, y1)| (0.5 + (y0 - 0.5) * k, 0.5 + (y1 - 0.5) * k));
        let starts = std::iter::once(a).chain(framings.iter().map(|f| f.start).filter(|s| *s > a + 1e-6 && *s < b));
        for s in starts {
            let t = offset + s - a;
            let slot = caption_slot(default, half, shown(at(s)), t >= title);
            if keys.last().is_none_or(|l| (l.1 - slot).abs() > 1e-9) {
                keys.push((t, slot));
            }
        }
        offset += b - a;
    }
    match keys.as_slice() {
        [] | [_] => json!(hf * keys.first().map(|k| k.1).unwrap_or(default)),
        _ => Value::Array(
            keys.iter().enumerate().map(|(i, (t, y))| if i == 0 { json!([0.0, hf * y]) } else { json!([t, hf * y, "hold"]) }).collect(),
        ),
    }
}

/// Zoom of clip `i`: jump cuts alternate between the framing and a punch-in.
fn clip_zoom(o: &ReelOpts, crop: bool, i: usize) -> f64 {
    let r = o.ranges;
    // a jump cut: the next piece of the same take, a moment later
    let jump = i > 0 && (r[i].0 - r[i - 1].1) < 4.0 && r[i].0 >= r[i - 1].1;
    if o.punch && crop && jump && punched(r, i) {
        1.1
    } else {
        1.0
    }
}

/// Move the captions per shot so they don't cover faces (see [`caption_y`]).
fn place_captions(
    o: &ReelOpts,
    caps: &mut Value,
    framings: &[Framing],
    canvas: (f64, f64),
    source: (f64, f64),
    default: f64,
) -> Result<()> {
    let (wf, hf) = canvas;
    // the look decides how tall a caption block gets: two lines of the (popped) active size
    let look = match &o.preset {
        Some(p) => crate::scene::presets::caption_preset(p, canvas, None)?,
        None => caps.clone(),
    };
    let size = look["size"].as_f64().unwrap_or((wf.min(hf) * 0.085).round());
    let lines = look["maxLines"].as_f64().unwrap_or(2.0).clamp(1.0, 3.0);
    let line = look["lineHeight"].as_f64().unwrap_or(1.15);
    let half = (lines * size * line * 1.12 + 2.0 * look["stroke"]["width"].as_f64().unwrap_or(0.0)) / 2.0 / hf;
    // a cover fit scales the source to fill the canvas; vertically centred
    let vscale = (source.1 * (wf / source.0.max(1.0)).max(hf / source.1.max(1.0))) / hf;
    let zooms: Vec<f64> = (0..o.ranges.len()).map(|i| clip_zoom(o, true, i)).collect();
    let total: f64 = o.ranges.iter().map(|(a, b)| b - a).sum();
    let title = if o.title.as_deref().is_some_and(|t| !t.trim().is_empty()) { total.min(4.5) } else { 0.0 };
    let y = caption_y(framings, o.ranges, &zooms, vscale, hf, default, half, title);
    if y.is_array() {
        // a jump, not a smear, where the captions move
        caps["motionBlur"] = json!(false);
    }
    caps["y"] = y;
    Ok(())
}

pub fn reel(o: &ReelOpts) -> Result<Value> {
    let info = media::probe(o.video)?;
    if !info.has_video {
        bail!("{} has no video", o.video.display());
    }
    if o.ranges.is_empty() {
        bail!("nothing left to show: the chosen part is empty or all silence");
    }
    let (w, h) = canvas_for(&o.format)?;
    let (wf, hf) = (w as f64, h as f64);
    let fps = o.fps.unwrap_or_else(|| reel_fps(info.fps));
    let total: f64 = o.ranges.iter().map(|(a, b)| b - a).sum();
    let end_d = o.end.as_ref().map(|e| e.duration).unwrap_or(0.0);
    let src = abs(o.video);
    let crop = match o.fit.as_str() {
        "crop" | "cover" => true,
        "fit" | "contain" => false,
        f => bail!("unknown fit '{f}' (crop, fit)"),
    };
    let visible = visible_fraction((wf, hf), (info.width as f64, info.height as f64));
    let pan_of = |x: f64| if crop { pan_for(x, visible) } else { 0.0 };
    // `fit`: the whole picture across the width, in the upper part; captions underneath
    let (vy, vh) = if crop { (hf / 2.0, hf) } else { (hf * 0.42, wf * info.height as f64 / info.width.max(1) as f64) };
    // where each clip starts in the reel: they play one after the other
    let starts: Vec<f64> = o.ranges.iter().scan(0.0, |t, (a, b)| Some(std::mem::replace(t, *t + (b - a)))).collect();
    let clips: Vec<Value> = o
        .ranges
        .iter()
        .enumerate()
        .flat_map(|(i, &(a, b))| {
            let parts = match o.framings {
                Some(f) => clip_parts(f, (a, b), starts[i], fps, pan_of),
                None => vec![Part { at: 0.0, trim: 0.0, pan: pan_of(o.focus) }],
            };
            let zoom = clip_zoom(o, crop, i);
            let ends: Vec<f64> = parts.iter().skip(1).map(|p| p.at).chain([b - a]).collect();
            parts
                .iter()
                .zip(ends)
                .map(|(p, end)| {
                    json!({"type": "video", "src": src, "trim": a + p.trim, "duration": end - p.at, "volume": 0,
                           "fit": "cover", "panX": p.pan, "zoom": zoom, "width": wf, "height": vh})
                })
                .collect::<Vec<_>>()
        })
        .collect();
    // the end card's still is framed like the last clip, so the cut to the card reads as a focus pull
    let end_pan = clips.last().and_then(|c| c["panX"].as_f64()).unwrap_or(0.0);
    let mut layers = vec![];
    if let (false, Some(b)) = (crop, o.backdrop) {
        // the same picture, blurred and dimmed, fills the frame behind the fitted video
        layers.push(json!({"type": "image", "src": abs(b), "blur": 40, "zoom": {"from": 1.15, "to": 1.3, "ease": "linear"}}));
        layers.push(json!({"type": "rect", "color": "rgba(0,0,0,0.45)"}));
    }
    layers.push(json!({"type": "group", "sequence": true, "y": vy, "height": vh, "children": clips}));
    if crop {
        // keeps white captions readable on any footage
        layers.push(json!({"type": "rect", "y": hf * 0.8, "height": hf * 0.4, "duration": total,
            "gradient": {"type": "linear", "angle": 180, "stops": [[0, "#00000000"], [1, "#0000008c"]]}}));
    }
    let position = o.position.unwrap_or(if crop { 0.72 } else { 0.8 });
    // captions stop with the voice: they never run into the end card
    let mut caps = json!({"type": "captions", "src": abs(o.words), "y": hf * position, "width": wf * 0.86, "duration": total});
    match &o.preset {
        Some(p) => {
            crate::scene::presets::caption_preset(p, (wf, hf), None)?;
            caps["preset"] = json!(p);
        }
        None => {
            let size = (wf.min(hf) * 0.085).round();
            caps["style"] = json!("pop");
            caps["size"] = json!(size);
            caps["uppercase"] = json!(true);
            caps["stroke"] = json!({"width": (size * 0.09).round(), "color": "#000000"});
            caps["highlight"] = json!("#ffd60a");
        }
    }
    if let Some(c) = o.highlight.as_ref().or(o.accent.as_ref()) {
        caps["highlight"] = json!(c);
    }
    if let (true, None, Some(f)) = (crop, o.position, o.framings) {
        place_captions(o, &mut caps, f, (wf, hf), (info.width as f64, info.height as f64), position)?;
    }
    layers.push(caps);
    if let Some(t) = o.title.as_deref().filter(|t| !t.trim().is_empty()) {
        // the hook: big, at the top, in for the first seconds
        // black on a white box: readable on any footage, the look viewers know from reels
        let size = (wf * 0.068).round();
        layers.push(json!({"type": "text", "text": t, "y": hf * 0.15, "size": size, "maxWidth": wf * 0.8,
            "wrap": "balance", "font": "Segoe UI Black", "align": "center", "color": "#111111", "lineHeight": 1.1,
            "background": {"color": "#ffffff", "padding": [(size * 0.45).round(), (size * 0.28).round()], "radius": (size * 0.22).round()},
            "shadow": {"blur": 30, "y": 10, "color": "rgba(0,0,0,0.35)"},
            "duration": total.min(4.5), "in": {"type": "zoom-in", "duration": 0.45, "ease": "expo-out"}, "out": "fade 0.3"}));
    }
    if let Some(logo) = o.logo {
        // in the top right corner, inside the safe area the apps leave free: a fixed height, so
        // wide wordmarks and square marks look about as big (wide ones capped at a third of the width)
        let (iw, ih) = media::image::image_size(logo)?;
        let aspect = iw as f64 / ih.max(1) as f64;
        let lh = (hf * 0.034).min(wf * 0.34 / aspect).max(hf * 0.02);
        let lh = if aspect < 1.6 { lh * 1.8 } else { lh };
        let lw = lh * aspect;
        // a soft shade behind it: white logos stay visible against a bright sky
        layers.push(json!({"type": "rect", "anchor": [0.5, 0], "y": 0, "height": hf * 0.16, "motionBlur": false, "duration": total,
            "gradient": {"type": "linear", "angle": 180, "stops": [[0, "#00000073"], [1, "#00000000"]]}}));
        layers.push(json!({"type": "image", "src": abs(logo), "fit": "contain", "width": lw, "height": lh, "duration": total,
            "anchor": [1, 0], "x": wf - wf * 0.06, "y": hf * 0.06, "opacity": 0.9, "motionBlur": false}));
    }
    if o.progress {
        let accent = o.accent.as_deref().or(o.highlight.as_deref()).unwrap_or("#ffd60a");
        layers.push(json!({"type": "rect", "anchor": [0, 1], "x": 0, "y": hf, "width": wf, "height": (hf * 0.006).round().max(6.0),
            "color": accent, "scaleX": {"from": 0, "to": 1, "ease": "linear"}, "motionBlur": false, "duration": total}));
    }
    if let Some(e) = &o.end {
        layers.push(end_card(e, (wf, hf), total, end_pan, o.accent.as_deref().or(o.highlight.as_deref()))?);
    }
    let mut audio = vec![json!({"src": abs(o.audio)})];
    if let Some(m) = o.music {
        // over an end card the music carries on (no voice to duck for) and fades out with it
        let fade = if end_d > 0.0 { end_d } else { 2.0 };
        audio.push(json!({"src": abs(m), "volume": 0.5, "duck": 0.18, "loop": true, "fadeIn": 0.5, "fadeOut": fade}));
    }
    Ok(json!({
        "width": w, "height": h, "fps": fps, "duration": total + end_d, "loudness": -14,
        "motionBlur": true,
        "audio": audio,
        "layers": layers
    }))
}

/// The end card, from `start` (the end of the last clip) to the end: the last frame goes soft
/// (a blurred still cross-fades in while the camera keeps drifting in), then the logo zooms in and
/// the call to action rises out of a mask, word by word, over an accent rule that draws itself.
pub fn end_card(e: &EndCard, (wf, hf): (f64, f64), start: f64, pan: f64, accent: Option<&str>) -> Result<Value> {
    let accent = accent.unwrap_or("#ffd60a");
    let d = e.duration;
    let mut kids = vec![];
    match e.still {
        Some(s) => {
            kids.push(json!({"type": "image", "src": abs(s), "blur": 36, "panX": pan,
                "in": {"type": "fade", "duration": 0.35, "ease": "out"}, "zoom": [[0, 1.0], [d, 1.12, "expo-out"]]}));
            // dimmed, darkest at the bottom where the apps put their buttons
            kids.push(json!({"type": "rect", "in": {"type": "fade", "duration": 0.35},
                "gradient": {"type": "linear", "angle": 180, "stops": [[0, "#00000073"], [0.5, "#000000a6"], [1, "#000000e0"]]}}));
        }
        None => {
            kids.push(json!({"type": "rect", "color": accent, "in": {"type": "iris", "duration": 0.55, "ease": "expo-in-out"}}));
            kids.push(json!({"type": "rect", "gradient": {"type": "radial", "colors": ["#00000000", "#00000059"]}}));
        }
    }
    let text = e.text.as_deref().map(str::trim).filter(|t| !t.is_empty());
    // the logo sits a little above the middle with text under it, in the middle without
    let mut y = if text.is_some() { hf * 0.44 } else { hf * 0.5 };
    if let Some(logo) = e.logo {
        let (iw, ih) = crate::media::image::image_size(logo)?;
        let aspect = iw as f64 / ih.max(1) as f64;
        // wordmarks by width, marks by height
        let (lw, lh) = if aspect >= 1.6 { (wf * 0.62, wf * 0.62 / aspect) } else { (hf * 0.15 * aspect, hf * 0.15) };
        let (lw, lh) = if lh > hf * 0.2 { (hf * 0.2 * aspect, hf * 0.2) } else { (lw, lh) };
        kids.push(json!({"type": "image", "src": abs(logo), "fit": "contain", "width": lw, "height": lh, "x": wf / 2.0,
            "y": y - lh / 2.0, "start": 0.12, "shadow": {"blur": 40, "y": 10, "color": "rgba(0,0,0,0.35)"},
            "scale": [[0, 0.82], [0.7, 1, "expo-out"], ["end", 1.03, "linear"]],
            "opacity": [[0, 0], [0.35, 1, "out"]]}));
        y += hf * 0.035;
    }
    if let Some(t) = text {
        let size = (wf * if e.logo.is_some() { 0.088 } else { 0.11 }).round();
        // rough line count for the layout below (uppercase black type, balanced at 84% of the width)
        let lines = (t.chars().count() as f64 * size * 0.72 / (wf * 0.84)).ceil().max(1.0);
        let th = lines * size;
        let ty = if e.logo.is_some() { y + th / 2.0 } else { hf * 0.48 };
        kids.push(json!({"type": "text", "text": t, "y": ty, "size": size, "font": "Segoe UI Black", "color": "#ffffff",
            "uppercase": true, "letterSpacing": (size * 0.02).round(),
            "align": "center", "maxWidth": wf * 0.84, "wrap": "balance", "lineHeight": 1.0,
            "start": 0.3, "reveal": "words", "revealStyle": "mask", "revealDuration": 0.45,
            "shadow": {"blur": 30, "y": 8, "color": "rgba(0,0,0,0.45)"}}));
        // an accent rule under it that draws out from the middle
        kids.push(json!({"type": "rect", "color": accent, "width": wf * 0.18, "height": (hf * 0.0065).round().max(6.0),
            "radius": 8, "y": ty + th / 2.0 + size * 0.5, "start": 0.5, "scaleX": [[0, 0], [0.6, 1, "expo-out"]]}));
    }
    Ok(json!({"type": "group", "start": start, "duration": d, "children": kids}))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(text: &str, start: f64, end: f64) -> Word {
        Word { text: text.into(), start, end, speaker: None }
    }

    #[test]
    fn words_follow_the_cut() {
        let words = vec![w("a", 1.0, 1.4), w("uh", 2.0, 2.3), w("b", 5.1, 5.5), w("c", 9.0, 9.2)];
        // keep 0.5..2.0 and 5.0..6.0 (the "uh" and "c" are cut)
        let r = remap_words(&words, &[(0.5, 2.0), (5.0, 6.0)]);
        assert_eq!(r.iter().map(|w| w.text.as_str()).collect::<Vec<_>>(), ["a", "b"]);
        assert!((r[0].start - 0.5).abs() < 1e-9 && (r[0].end - 0.9).abs() < 1e-9);
        // "b" starts 0.1 s into the second range, which starts at 1.5 s in the reel
        assert!((r[1].start - 1.6).abs() < 1e-9 && (r[1].end - 2.0).abs() < 1e-9);
    }

    #[test]
    fn jump_cuts_alternate_punch_ins() {
        // three pieces of one take (jump cuts), then a part from much later, then its jump cut
        let r = [(0.0, 5.0), (5.6, 9.0), (9.5, 12.0), (60.0, 70.0), (70.8, 75.0)];
        let p: Vec<bool> = (0..r.len()).map(|i| punched(&r, i)).collect();
        assert_eq!(p, [false, true, false, false, true]);
    }

    #[test]
    fn reels_play_at_most_30_fps() {
        assert_eq!(reel_fps(25.0), 25.0);
        assert_eq!(reel_fps(29.97), 29.97);
        assert_eq!(reel_fps(50.0), 25.0);
        assert_eq!(reel_fps(59.94), 29.97);
        assert_eq!(reel_fps(120.0), 60.0);
        assert_eq!(reel_fps(0.0), 30.0);
    }

    #[test]
    fn end_card_follows_the_last_clip() {
        let e = EndCard { text: Some("Volg ons".into()), logo: None, still: None, duration: 2.0 };
        let g = end_card(&e, (1080.0, 1920.0), 31.5, 0.2, Some("#f39200")).unwrap();
        assert_eq!(g["start"], 31.5);
        assert_eq!(g["duration"], 2.0);
        let kids = g["children"].as_array().unwrap();
        // no still: the accent colour, opened with an iris
        assert_eq!(kids[0]["color"], "#f39200");
        let text = kids.iter().find(|k| k["type"] == "text").unwrap();
        assert_eq!(text["text"], "Volg ons");
        assert_eq!(text["revealStyle"], "mask");
        // the whole card validates as a scene
        let scene = json!({"width": 1080, "height": 1920, "duration": 33.5, "layers": [g]});
        let (_, warnings) = crate::Scene::from_value(scene, std::path::PathBuf::new()).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    #[test]
    fn captions_keep_clear_of_faces() {
        let half = 0.06;
        // no face, or a face high in the picture (a wide or medium shot): the default
        assert_eq!(caption_slot(0.72, half, None, true), 0.72);
        assert_eq!(caption_slot(0.72, half, Some((0.2, 0.45)), true), 0.72);
        // a close-up whose chin reaches the captions: as low as the app UI allows
        let low = caption_slot(0.72, half, Some((0.35, 0.6)), true);
        assert!((low - (SAFE_BOTTOM - half)).abs() < 1e-9);
        // a face filling the lower half: up to the top slot, unless the hook title is there
        let high = caption_slot(0.72, half, Some((0.45, 0.75)), true);
        assert!((high - (SAFE_TOP + half + 0.01)).abs() < 1e-9);
        assert!(caption_slot(0.72, half, Some((0.45, 0.75)), false) > 0.7);
        // every slot stays inside the safe area
        for b in [None, Some((0.35, 0.6)), Some((0.45, 0.75)), Some((0.1, 0.95))] {
            let s = caption_slot(0.72, half, b, true);
            assert!(s - half >= SAFE_TOP - 1e-9 && s + half <= SAFE_BOTTOM + 1e-9, "{b:?} -> {s}");
        }
    }

    fn framing(start: f64, band: Option<(f64, f64)>) -> Framing {
        Framing { start, x: 0.5, face: band.is_some(), band }
    }

    #[test]
    fn captions_jump_on_the_cut() {
        // a wide shot, a close-up from 3 s (source), back to a wide shot at 6 s
        let f = [framing(0.0, Some((0.2, 0.35))), framing(3.0, Some((0.35, 0.6))), framing(6.0, None)];
        // the reel plays 1..4 and 5..8 of the source
        let y = caption_y(&f, &[(1.0, 4.0), (5.0, 8.0)], &[1.0, 1.0], 1.0, 1000.0, 0.72, 0.06, 0.0);
        let keys = y.as_array().unwrap();
        assert_eq!(keys.len(), 3, "{y}");
        assert_eq!(keys[0], json!([0.0, 720.0]));
        // the close-up starts 2 s into the reel; the wide shot at source 6 s is reel 4 s
        assert_eq!(keys[1][0], json!(2.0));
        assert!((keys[1][1].as_f64().unwrap() - 790.0).abs() < 1e-6);
        assert_eq!(keys[1][2], "hold");
        assert_eq!(keys[2][0], json!(4.0));
        assert_eq!(keys[2][1], json!(720.0));
        // nothing to avoid: one fixed position
        let y = caption_y(&f[..1], &[(1.0, 4.0)], &[1.0], 1.0, 1000.0, 0.72, 0.06, 0.0);
        assert_eq!(y, json!(720.0));
        // a punch-in (zoom 1.1) pushes a face just clear of the captions into them
        let f = [framing(0.0, Some((0.47, 0.6)))];
        assert_eq!(caption_y(&f, &[(0.0, 2.0)], &[1.0], 1.0, 1000.0, 0.72, 0.06, 0.0), json!(720.0));
        assert_ne!(caption_y(&f, &[(0.0, 2.0)], &[1.1], 1.0, 1000.0, 0.72, 0.06, 0.0), json!(720.0));
    }

    /// Play a clip's parts as a render does (a part draws frame `f` from its start on; it shows
    /// source `a + trim + (f - round(part start * fps)) / fps`): (source, panX) per frame.
    fn play(parts: &[Part], (a, b): (f64, f64), start: f64, fps: f64) -> Vec<(f64, f64)> {
        let ends: Vec<f64> = parts.iter().skip(1).map(|p| p.at).chain([b - a]).collect();
        // part starts summed the way a sequence group does
        let mut t = start;
        let starts: Vec<f64> = parts
            .iter()
            .zip(&ends)
            .map(|(p, e)| {
                let s = t;
                t += e - p.at;
                s
            })
            .collect();
        let mut out = vec![];
        let mut f = (start * fps).floor() as i64;
        while (f as f64 / fps) < start + (b - a) - 1e-9 {
            let ft = f as f64 / fps;
            if let Some(k) = (0..parts.len()).rev().find(|&k| ft >= starts[k]) {
                let lf = f - (starts[k] * fps).round() as i64;
                out.push((a + parts[k].trim + lf as f64 / fps, parts[k].pan));
            }
            f += 1;
        }
        out
    }

    #[test]
    fn clips_split_where_the_shots_change_on_the_reel_frames() {
        let f = |start: f64, x: f64| Framing { start, x, face: true, band: None };
        let shots = [f(0.0, 0.3), f(10.05, 0.9), f(12.33, 0.7), f(15.0, 0.7), f(18.0, 0.5), f(30.0, 0.2)];
        let pan = |x: f64| x * 2.0 - 1.0;
        let shot_pan = |t: f64| pan(shots.iter().rev().find(|s| s.start <= t + 1e-6).unwrap().x);
        let fps = 25.0;
        // clips starting on a frame and between frames (the clips before end anywhere)
        for start in [0.0, 0.01, 0.013, 0.02, 0.025, 0.031, 3.217, 41.99] {
            let parts = clip_parts(&shots, (10.0, 20.0), start, fps, pan);
            // the shot at 15 keeps the framing, the one at 30 lies past the clip
            assert_eq!(parts.len(), 4, "{start}: {parts:?}");
            let frames = play(&parts, (10.0, 20.0), start, fps);
            for w in frames.windows(2) {
                // seamless: every frame the next source frame
                assert!((w[1].0 - w[0].0 - 1.0 / fps).abs() < 1e-6, "{start}: {w:?}");
            }
            for &(src, p) in &frames {
                // and every frame framed for the shot it shows
                assert!((p - shot_pan(src)).abs() < 1e-9, "clip at {start}: source {src:.3} framed {p}, its shot {}", shot_pan(src));
            }
        }
        // a clip inside one shot is one part
        assert_eq!(clip_parts(&shots, (19.0, 25.0), 1.0, fps, pan).len(), 1);
    }

    #[test]
    fn ranges_are_clipped_to_the_windows() {
        let k = within(&[(0.0, 10.0), (12.0, 30.0)], &[(5.0, 15.0), (20.0, 22.0)]);
        assert_eq!(k, vec![(5.0, 10.0), (12.0, 15.0), (20.0, 22.0)]);
    }
}
