//! Ready-made scenes: podcast audiograms and captioned videos.

use crate::ffmpeg;
use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::path::Path;

fn abs(p: &Path) -> String {
    std::fs::canonicalize(p)
        .map(|p| p.to_string_lossy().trim_start_matches(r"\\?\").to_string())
        .unwrap_or_else(|_| p.to_string_lossy().to_string())
}

pub struct PodcastOpts<'a> {
    pub audio: &'a Path,
    pub transcript: &'a Path,
    pub cover: Option<&'a Path>,
    pub title: String,
    pub subtitle: Option<String>,
    pub format: String,
    pub from: f64,
    pub to: Option<f64>,
    pub accent: String,
    pub style: Option<String>,
}

pub fn podcast(o: &PodcastOpts) -> Result<Value> {
    let info = ffmpeg::probe(o.audio)?;
    let end = o.to.unwrap_or(info.duration).min(info.duration);
    let dur = end - o.from;
    if dur <= 0.0 {
        bail!("empty time range ({} .. {end})", o.from);
    }
    let (w, h) = match o.format.as_str() {
        "16:9" | "landscape" | "youtube" => (1920, 1080),
        "9:16" | "portrait" | "reel" | "story" | "tiktok" => (1080, 1920),
        "1:1" | "square" => (1080, 1080),
        f => bail!("unknown format '{f}' (16:9, 9:16, 1:1)"),
    };
    let audio = abs(o.audio);
    let tr = abs(o.transcript);
    let accent = o.accent.as_str();
    let mut layers: Vec<Value> = vec![];

    // background
    match o.cover {
        Some(c) => {
            layers.push(json!({"type": "image", "src": abs(c), "blur": 60, "zoom": {"from": 1.15, "to": 1.35}}));
            layers.push(json!({"type": "rect", "color": "rgba(12,8,22,0.55)"}));
        }
        None => layers.push(json!({"type": "rect", "gradient": {"type": "radial", "colors": [accent, "#120a1c"]}})),
    }
    // geometry per format: (cover x,y,size) (title y, size) (captions y,size,width,style) (wave y,w,h)
    let (cx, cy, cs, tx, ty, tsz, capy, capsz, capw, wavey, wavew, waveh, align) = match (w, h) {
        (1920, 1080) => (470.0, 470.0, 560.0, 1290.0, 240.0, 64.0, 640.0, 62.0, 1040.0, 900.0, 560.0, 120.0, "center"),
        (1080, 1920) => (540.0, 560.0, 620.0, 540.0, 980.0, 58.0, 1390.0, 84.0, 960.0, 1730.0, 760.0, 110.0, "center"),
        _ => (540.0, 330.0, 420.0, 540.0, 620.0, 46.0, 830.0, 58.0, 960.0, 1000.0, 520.0, 70.0, "center"),
    };
    let style = o.style.clone().unwrap_or_else(|| if h > w { "pop".into() } else { "karaoke".into() });

    match o.cover {
        Some(c) => layers.push(json!({
            "type": "image", "src": abs(c), "x": cx, "y": cy, "width": cs, "height": cs, "radius": 32,
            "shadow": {"blur": 40, "y": 24, "color": "rgba(0,0,0,0.55)"},
            "zoom": {"from": 1.0, "to": 1.08}, "in": "pop"
        })),
        None => {
            layers.push(json!({
                "type": "rect", "x": cx, "y": cy, "width": cs, "height": cs, "radius": 32,
                "gradient": {"angle": 135, "colors": [accent, "#2a0d3e"]},
                "shadow": {"blur": 40, "y": 24, "color": "rgba(0,0,0,0.55)"}, "in": "pop"
            }));
            layers.push(json!({"type": "text", "text": o.title, "x": cx, "y": cy, "size": cs * 0.11, "maxWidth": cs * 0.8, "weight": 900, "in": "pop"}));
        }
    }
    layers.push(json!({
        "type": "text", "text": o.title, "x": tx, "y": ty, "size": tsz, "maxWidth": if w > h { 1050.0 } else { 940.0 },
        "align": align, "font": "Segoe UI Black", "in": {"type": "slide-up", "duration": 0.6}
    }));
    if let Some(sub) = &o.subtitle {
        layers.push(json!({
            "type": "text", "text": sub, "x": tx, "y": ty + tsz * 1.25, "size": tsz * 0.55, "color": "rgba(255,255,255,0.72)",
            "maxWidth": 1000, "align": align, "font": "Segoe UI Semibold", "in": {"type": "slide-up", "duration": 0.7}
        }));
    }
    layers.push(json!({
        "type": "waveform", "src": audio, "trim": o.from, "x": cx, "y": wavey,
        "width": wavew, "height": waveh, "bars": if h > w { 40 } else { 32 }, "color": accent, "mirror": true, "opacity": 0.95
    }));
    layers.push(json!({
        "type": "captions", "src": tr, "trim": o.from, "x": tx, "y": capy, "width": capw, "size": capsz,
        "style": style, "highlight": accent, "font": "Segoe UI Black"
    }));
    layers.push(json!({
        "type": "rect", "x": 0, "y": h as f64 - 5.0, "anchor": [0, 0.5], "width": w, "height": 10,
        "color": accent, "scaleX": {"from": 0.0, "to": 1.0}
    }));

    Ok(json!({
        "width": w, "height": h, "fps": 30, "duration": dur, "background": "#0d0a14",
        "audio": [{"src": audio, "trim": o.from, "duration": dur, "fadeIn": if o.from > 0.0 { 0.3 } else { 0.0 }, "fadeOut": if o.to.is_some() { 0.6 } else { 0.0 }}],
        "layers": layers
    }))
}

pub struct CaptionOpts<'a> {
    pub video: &'a Path,
    pub transcript: &'a Path,
    pub style: String,
    pub highlight: String,
    pub uppercase: bool,
    pub position: f64,
}

pub fn captioned_video(o: &CaptionOpts) -> Result<Value> {
    let info = ffmpeg::probe(o.video)?;
    if !info.has_video {
        bail!("{} has no video", o.video.display());
    }
    let (mut w, mut h) = (info.width as f64, info.height as f64);
    let long = w.max(h);
    if long > 1920.0 {
        w = w * 1920.0 / long;
        h = h * 1920.0 / long;
    }
    let even = |v: f64| ((v / 2.0).round() * 2.0) as u32;
    let (w, h) = (even(w), even(h));
    let vertical = h > w;
    let size = (w.min(h) as f64 * if vertical { 0.085 } else { 0.06 }).round();
    let fps = if info.fps > 1.0 && info.fps <= 120.0 { (info.fps * 1000.0).round() / 1000.0 } else { 30.0 };
    Ok(json!({
        "width": w, "height": h, "fps": fps, "duration": info.duration,
        "layers": [
            {"type": "video", "src": abs(o.video)},
            {"type": "captions", "src": abs(o.transcript), "style": o.style, "y": h as f64 * o.position,
             "width": w as f64 * 0.88, "size": size, "highlight": o.highlight, "uppercase": o.uppercase,
             "stroke": {"width": (size * 0.09).round(), "color": "#000000"}}
        ]
    }))
}
