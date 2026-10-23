//! `flits captions`: burn animated word-by-word captions into an existing video.

use super::abs;
use crate::media;
use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::path::Path;

pub struct CaptionOpts<'a> {
    pub video: &'a Path,
    pub transcript: &'a Path,
    pub style: String,
    pub highlight: String,
    pub uppercase: bool,
    pub position: f64,
}

pub fn captioned_video(o: &CaptionOpts) -> Result<Value> {
    let info = media::probe(o.video)?;
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
