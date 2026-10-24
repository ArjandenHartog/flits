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

/// The framings of a clip playing source `[a, b)`: `(offset into the clip, panX)` per part, the
/// framing of the shot it starts in, then one part per later cut inside the clip. Parts start on
/// whole frames from the clip start, so playing them one after the other is seamless; each is its
/// own layer with a constant framing, which lets the decoder cut out just the part that shows.
fn clip_parts(framings: &[Framing], a: f64, b: f64, fps: f64, pan_of: impl Fn(f64) -> f64) -> Vec<(f64, f64)> {
    let at = |t: f64| framings.iter().rev().find(|f| f.start <= t + 1e-6).or(framings.first()).map(|f| f.x).unwrap_or(0.5);
    let mut parts = vec![(0.0, pan_of(at(a)))];
    for f in framings.iter().filter(|f| f.start > a + 1e-6 && f.start < b) {
        // the first frame at or after the cut
        let off = ((f.start - a) * fps - 1e-6).ceil() / fps;
        let pan = pan_of(f.x);
        let last = parts.last_mut().unwrap();
        if (last.1 - pan).abs() < 1e-9 || off >= b - a {
            continue;
        }
        if off <= last.0 {
            last.1 = pan; // two cuts within one frame: the later framing wins
        } else {
            parts.push((off, pan));
        }
    }
    parts
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
    let clips: Vec<Value> = o
        .ranges
        .iter()
        .enumerate()
        .flat_map(|(i, &(a, b))| {
            let parts = match o.framings {
                Some(f) => clip_parts(f, a, b, fps, pan_of),
                None => vec![(0.0, pan_of(o.focus))],
            };
            // a jump cut: the next piece of the same take, a moment later
            let jump = i > 0 && (a - o.ranges[i - 1].1) < 4.0 && a >= o.ranges[i - 1].1;
            let zoom = if o.punch && crop && jump && punched(o.ranges, i) { 1.1 } else { 1.0 };
            let ends: Vec<f64> = parts.iter().skip(1).map(|p| p.0).chain([b - a]).collect();
            parts
                .iter()
                .zip(ends)
                .map(|(&(off, pan), end)| {
                    json!({"type": "video", "src": src, "trim": a + off, "duration": end - off, "volume": 0,
                           "fit": "cover", "panX": pan, "zoom": zoom, "width": wf, "height": vh})
                })
                .collect::<Vec<_>>()
        })
        .collect();
    let mut layers = vec![];
    if let (false, Some(b)) = (crop, o.backdrop) {
        // the same picture, blurred and dimmed, fills the frame behind the fitted video
        layers.push(json!({"type": "image", "src": abs(b), "blur": 40, "zoom": {"from": 1.15, "to": 1.3, "ease": "linear"}}));
        layers.push(json!({"type": "rect", "color": "rgba(0,0,0,0.45)"}));
    }
    layers.push(json!({"type": "group", "sequence": true, "y": vy, "height": vh, "children": clips}));
    if crop {
        // keeps white captions readable on any footage
        layers.push(json!({"type": "rect", "y": hf * 0.8, "height": hf * 0.4,
            "gradient": {"type": "linear", "angle": 180, "stops": [[0, "#00000000"], [1, "#0000008c"]]}}));
    }
    let position = o.position.unwrap_or(if crop { 0.72 } else { 0.8 });
    let mut caps = json!({"type": "captions", "src": abs(o.words), "y": hf * position, "width": wf * 0.86});
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
        layers.push(json!({"type": "rect", "anchor": [0.5, 0], "y": 0, "height": hf * 0.16, "motionBlur": false,
            "gradient": {"type": "linear", "angle": 180, "stops": [[0, "#00000073"], [1, "#00000000"]]}}));
        layers.push(json!({"type": "image", "src": abs(logo), "fit": "contain", "width": lw, "height": lh,
            "anchor": [1, 0], "x": wf - wf * 0.06, "y": hf * 0.06, "opacity": 0.9, "motionBlur": false}));
    }
    if o.progress {
        let accent = o.accent.as_deref().or(o.highlight.as_deref()).unwrap_or("#ffd60a");
        layers.push(json!({"type": "rect", "anchor": [0, 1], "x": 0, "y": hf, "width": wf, "height": (hf * 0.006).round().max(6.0),
            "color": accent, "scaleX": {"from": 0, "to": 1, "ease": "linear"}, "motionBlur": false}));
    }
    let mut audio = vec![json!({"src": abs(o.audio)})];
    if let Some(m) = o.music {
        audio.push(json!({"src": abs(m), "volume": 0.5, "duck": 0.18, "loop": true, "fadeIn": 0.5, "fadeOut": 2.0}));
    }
    Ok(json!({
        "width": w, "height": h, "fps": fps, "duration": total, "loudness": -14,
        "motionBlur": true,
        "audio": audio,
        "layers": layers
    }))
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
    fn clips_split_on_whole_frames_at_framing_changes() {
        let f = |start: f64, x: f64| Framing { start, x, face: true };
        let shots = [f(0.0, 0.3), f(12.33, 0.7), f(15.0, 0.7), f(18.0, 0.5), f(30.0, 0.2)];
        let pan = |x: f64| x * 2.0 - 1.0;
        // clip 10..20 at 25 fps: cuts at 12.33 (on the next whole frame, 2.36 s in) and 18; the
        // shot at 15 keeps the framing, the one at 30 lies past the clip
        let p = clip_parts(&shots, 10.0, 20.0, 25.0, pan);
        assert_eq!(p.len(), 3);
        assert!((p[0].0 - 0.0).abs() < 1e-9 && (p[0].1 - -0.4).abs() < 1e-9);
        assert!((p[1].0 - 2.36).abs() < 1e-9 && (p[1].1 - 0.4).abs() < 1e-9);
        assert!((p[2].0 - 8.0).abs() < 1e-9 && p[2].1.abs() < 1e-9);
        // every part starts on a whole frame from the clip start
        for (off, _) in &p {
            assert!(((off * 25.0) - (off * 25.0).round()).abs() < 1e-6);
        }
        // a clip inside one shot is one part
        assert_eq!(clip_parts(&shots, 19.0, 25.0, 25.0, pan).len(), 1);
    }

    #[test]
    fn ranges_are_clipped_to_the_windows() {
        let k = within(&[(0.0, 10.0), (12.0, 30.0)], &[(5.0, 15.0), (20.0, 22.0)]);
        assert_eq!(k, vec![(5.0, 10.0), (12.0, 15.0), (20.0, 22.0)]);
    }
}
