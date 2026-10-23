//! `flits slideshow`: a montage from a folder of photos and clips, optionally cut on
//! the beat of a music track.

use super::{abs, canvas_for};
use crate::media::{self, beats, image as images};
use anyhow::{bail, Result};
use rayon::prelude::*;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub struct SlideshowOpts {
    pub inputs: Vec<PathBuf>,
    pub music: Option<PathBuf>,
    pub format: String,
    /// seconds per photo
    pub per: f64,
    /// longest stretch of a video clip
    pub clip_max: f64,
    /// "kenburns" | "cards" | "mixed"
    pub style: String,
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub outro: Option<String>,
    /// cut on the music's beats
    pub beats: bool,
    /// stretch photo durations so the montage ends with the music
    pub fit_music: bool,
    pub keep_audio: bool,
    pub accent: String,
    /// "name" | "date" (file modification time)
    pub sort: String,
}

const IMAGE_EXT: &[&str] = &["jpg", "jpeg", "png", "webp", "gif", "bmp", "heic", "heif", "avif", "tif", "tiff"];
const VIDEO_EXT: &[&str] = &["mp4", "mov", "m4v", "mkv", "webm", "avi", "mts"];

fn ext(p: &Path) -> String {
    p.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase()
}

/// Expand folders into their media files.
pub fn collect(inputs: &[PathBuf], sort: &str) -> Result<Vec<PathBuf>> {
    let mut files = vec![];
    for i in inputs {
        if i.is_dir() {
            let mut v: Vec<PathBuf> = std::fs::read_dir(i)?.flatten().map(|e| e.path()).filter(|p| p.is_file()).collect();
            v.retain(|p| IMAGE_EXT.contains(&ext(p).as_str()) || VIDEO_EXT.contains(&ext(p).as_str()));
            files.extend(v);
        } else if i.is_file() {
            files.push(i.clone());
        } else {
            bail!("not found: {}", i.display());
        }
    }
    match sort {
        "date" => files.sort_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok()),
        _ => files.sort_by_key(|p| p.file_name().map(|n| n.to_ascii_lowercase())),
    }
    if files.is_empty() {
        bail!("no photos or videos found");
    }
    Ok(files)
}

struct Item {
    path: PathBuf,
    video: bool,
    w: u32,
    h: u32,
    /// natural length for clips
    len: f64,
    has_audio: bool,
}

fn inspect(files: &[PathBuf]) -> Vec<Item> {
    files
        .par_iter()
        .filter_map(|p| {
            if VIDEO_EXT.contains(&ext(p).as_str()) {
                let i = media::probe(p).ok().filter(|i| i.has_video)?;
                Some(Item { path: p.clone(), video: true, w: i.width, h: i.height, len: i.duration, has_audio: i.has_audio })
            } else {
                let (w, h) = images::image_size(p).ok()?;
                Some(Item { path: p.clone(), video: false, w, h, len: 0.0, has_audio: false })
            }
        })
        .collect()
}

/// One shot: full-bleed Ken Burns or a framed card on a blurred backdrop.
fn shot(it: &Item, i: usize, (cw, ch): (f64, f64), card: bool, volume: f64) -> Vec<Value> {
    let src = abs(&it.path);
    let dir = if i.is_multiple_of(2) { 1.0 } else { -1.0 };
    let media = |extra: Value| {
        let mut v = json!({"type": if it.video { "video" } else { "image" }, "src": src});
        if it.video {
            v["volume"] = json!(volume);
        }
        for (k, val) in extra.as_object().unwrap() {
            v[k] = val.clone();
        }
        v
    };
    if !card {
        let (z0, z1) = if i.is_multiple_of(2) { (1.0, 1.12) } else { (1.12, 1.0) };
        return vec![media(json!({"zoom": {"from": z0, "to": z1}, "panX": {"from": -0.35 * dir, "to": 0.35 * dir}}))];
    }
    let ar = it.w as f64 / it.h.max(1) as f64;
    let (mut h, mut w) = (ch * 0.78, ch * 0.78 * ar);
    if w > cw * 0.86 {
        w = cw * 0.86;
        h = w / ar;
    }
    let backdrop = if it.video {
        // a second decoder just for a backdrop isn't worth it: dark gradient
        json!({"type": "rect", "gradient": {"type": "radial", "colors": ["#2a2a33", "#0b0b0f"]}})
    } else {
        json!({"type": "image", "src": src, "blur": 50, "zoom": {"from": 1.15, "to": 1.25}})
    };
    vec![
        backdrop,
        json!({"type": "rect", "gradient": {"type": "radial", "stops": [[0, "rgba(0,0,0,0)"], [1, "rgba(0,0,0,0.55)"]]}}),
        media(json!({
            "width": w, "height": h, "radius": 10,
            "border": {"width": (ch * 0.012).round(), "color": "#fffaf3"},
            "shadow": {"blur": 40, "y": 24, "color": "rgba(0,0,0,0.5)"},
            "rotation": [[0, 4.0 * dir], [0.7, 1.2 * dir, "back-out"]],
            "scale": [[0, 0.9], [0.7, 1, "back-out"]],
            "zoom": {"from": 1.02, "to": 1.08},
            "in": {"type": "slide-up", "duration": 0.6, "distance": ch * 0.15}
        })),
    ]
}

fn title_card(text: &str, sub: Option<&str>, bg: Option<&Item>, (cw, ch): (f64, f64), accent: &str, dur: f64) -> Value {
    let mut kids = vec![];
    match bg {
        Some(it) if !it.video => kids.push(json!({"type": "image", "src": abs(&it.path), "blur": 40, "zoom": {"from": 1.1, "to": 1.2}})),
        _ => kids.push(json!({"type": "rect", "gradient": {"type": "radial", "colors": [accent, "#100c16"]}})),
    }
    kids.push(json!({"type": "rect", "color": "rgba(10,6,16,0.45)"}));
    let size = (cw.min(ch) * 0.1).round();
    kids.push(json!({"type": "text", "text": text, "y": ch * 0.47, "size": size, "font": "Segoe UI Black", "maxWidth": cw * 0.85,
        "shadow": {"blur": 24, "y": 6, "color": "rgba(0,0,0,0.5)"}, "in": {"type": "slide-up", "duration": 0.8}}));
    kids.push(json!({"type": "rect", "y": ch * 0.47 + size * 0.85, "width": cw * 0.12, "height": (size * 0.08).max(4.0), "radius": 4, "color": accent,
        "scaleX": [[0.3, 0], [1.0, 1, "out"]]}));
    if let Some(s) = sub {
        kids.push(json!({"type": "text", "text": s, "y": ch * 0.47 + size * 1.45, "size": (size * 0.42).round(), "color": "rgba(255,255,255,0.85)",
            "font": "Segoe UI Semibold", "in": {"type": "fade", "duration": 1.0}}));
    }
    json!({"type": "group", "duration": dur, "children": kids})
}

/// Pick cut points: uniform durations, optionally snapped to beats.
fn cut_times(durs: &[f64], beat_times: &[f64]) -> Vec<f64> {
    let mut cuts = vec![0.0];
    let mut t = 0.0;
    for d in durs {
        let target = t + d;
        let snapped = beat_times
            .iter()
            .copied()
            .filter(|b| *b > t + d * 0.6)
            .min_by(|a, b| (a - target).abs().partial_cmp(&(b - target).abs()).unwrap())
            .filter(|b| (b - target).abs() < d * 0.5)
            .unwrap_or(target);
        cuts.push(snapped);
        t = snapped;
    }
    cuts
}

pub fn slideshow(o: &SlideshowOpts) -> Result<Value> {
    let files = collect(&o.inputs, &o.sort)?;
    let items = inspect(&files);
    if items.is_empty() {
        bail!("none of the files could be read");
    }
    let (w, h) = canvas_for(&o.format)?;
    let (cw, ch) = (w as f64, h as f64);
    let landscape = cw >= ch;
    let music_len = match &o.music {
        Some(m) => Some(media::probe(m)?.duration),
        None => None,
    };
    let fade = 0.6;
    let intro = if o.title.is_some() { 3.0 } else { 0.0 };
    let outro = if o.outro.is_some() { 3.0 } else { 0.0 };

    let mut durs: Vec<f64> = items.iter().map(|it| if it.video { it.len.clamp(1.0, o.clip_max) } else { o.per }).collect();
    if let (Some(ml), true) = (music_len, o.fit_music) {
        let fixed: f64 = items.iter().zip(&durs).filter(|(it, _)| it.video).map(|(_, d)| d).sum();
        let photos = items.iter().filter(|it| !it.video).count();
        if photos > 0 {
            // stretch to the music, but never into a slow crawl (the music fades out instead)
            let per = ((ml - intro - outro - fixed) / photos as f64).clamp(1.5, (o.per * 1.4).max(o.per));
            for (it, d) in items.iter().zip(durs.iter_mut()) {
                if !it.video {
                    *d = per;
                }
            }
        }
    }
    let beat_times = match (&o.music, o.beats) {
        (Some(m), true) => {
            let b = beats::beats(m, 0.0, music_len.unwrap_or(600.0))?;
            if let Some(bpm) = beats::bpm(&b) {
                eprintln!("flits: music at ~{bpm:.0} BPM, cutting on the beat");
            }
            b.into_iter().map(|t| t - intro).filter(|t| *t > 0.0).collect()
        }
        _ => vec![],
    };
    let cuts = cut_times(&durs, &beat_times);

    let has_music = o.music.is_some();
    let clip_volume = if has_music && !o.keep_audio { 0.0 } else { 1.0 };
    let mut layers = vec![];
    if let Some(t) = &o.title {
        layers.push(title_card(t, o.subtitle.as_deref(), items.first(), (cw, ch), &o.accent, intro + fade));
    }
    for (i, it) in items.iter().enumerate() {
        let start = intro + cuts[i];
        let end = intro + cuts[i + 1] + fade;
        let card = match o.style.as_str() {
            "cards" => true,
            "kenburns" => (it.w >= it.h) != landscape && !it.video, // keep portraits uncropped on landscape canvases
            _ => i % 3 == 1 || ((it.w >= it.h) != landscape),
        };
        let mut g = json!({"type": "group", "start": start, "duration": end - start, "children": shot(it, i, (cw, ch), card, clip_volume)});
        if i > 0 || o.title.is_some() {
            g["in"] = json!({"type": "fade", "duration": fade});
        }
        layers.push(g);
    }
    let total = intro + cuts.last().unwrap() + outro + if outro > 0.0 { fade } else { 0.0 };
    if let Some(t) = &o.outro {
        let mut card = title_card(t, None, items.last(), (cw, ch), &o.accent, outro + fade);
        card["start"] = json!(total - outro - fade);
        card["in"] = json!({"type": "fade", "duration": fade});
        layers.push(card);
    }
    let mut scene =
        json!({"width": w, "height": h, "fps": 30, "duration": total, "background": "#000000", "loudness": -14, "layers": layers});
    if let Some(m) = &o.music {
        let any_clip_audio = clip_volume > 0.0 && items.iter().any(|i| i.video && i.has_audio);
        let looped = music_len.is_some_and(|l| l < total);
        let mut track = json!({"src": abs(m), "volume": 0.9, "fadeIn": 0.5, "fadeOut": 2.5, "loop": looped});
        if any_clip_audio {
            track["duck"] = json!(0.3);
        }
        scene["audio"] = json!([track]);
    }
    Ok(scene)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cuts_snap_to_nearby_beats() {
        let beats: Vec<f64> = (0..40).map(|i| i as f64 * 0.5).collect();
        let cuts = cut_times(&[3.2, 3.2, 3.2], &beats);
        assert_eq!(cuts, vec![0.0, 3.0, 6.0, 9.0]);
        assert_eq!(cut_times(&[2.0, 2.0], &[]), vec![0.0, 2.0, 4.0]);
    }
}
