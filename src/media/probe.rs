//! ffprobe: size, duration, rotation, HDR, streams.

use super::colorspace::FrameFormat;
use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::path::Path;
use std::process::Command;

#[derive(Clone, Debug, Default)]
pub struct MediaInfo {
    pub width: u32,
    pub height: u32,
    pub duration: f64,
    pub fps: f64,
    pub has_video: bool,
    pub has_audio: bool,
    pub hdr: bool,
    /// colour description of the video stream (bit depth, transfer, matrix, range)
    pub color: FrameFormat,
}

pub fn probe(path: &Path) -> Result<MediaInfo> {
    if !path.exists() {
        bail!("file not found: {}", path.display());
    }
    let out = Command::new(std::env::var("FLITS_FFPROBE").unwrap_or_else(|_| "ffprobe".into()))
        .args(["-v", "error", "-show_streams", "-show_format", "-of", "json"])
        .arg(path)
        .output()
        .context("running ffprobe (is ffmpeg installed and on PATH?)")?;
    if !out.status.success() {
        bail!("ffprobe failed on {}: {}", path.display(), String::from_utf8_lossy(&out.stderr));
    }
    let v: Value = serde_json::from_slice(&out.stdout)?;
    let mut info = MediaInfo::default();
    if let Some(d) = v["format"]["duration"].as_str().and_then(|s| s.parse().ok()) {
        info.duration = d;
    }
    for s in v["streams"].as_array().cloned().unwrap_or_default() {
        match s["codec_type"].as_str() {
            Some("video") if !info.has_video => {
                info.has_video = true;
                let (mut w, mut h) = (s["width"].as_u64().unwrap_or(0) as u32, s["height"].as_u64().unwrap_or(0) as u32);
                let mut rot = s["tags"]["rotate"].as_str().and_then(|r| r.parse::<i64>().ok()).unwrap_or(0);
                if let Some(sd) = s["side_data_list"].as_array() {
                    for d in sd {
                        if let Some(r) = d["rotation"].as_i64() {
                            rot = r;
                        }
                    }
                }
                if rot.rem_euclid(180) == 90 {
                    std::mem::swap(&mut w, &mut h);
                }
                info.width = w;
                info.height = h;
                let fr = s["avg_frame_rate"].as_str().or(s["r_frame_rate"].as_str()).unwrap_or("0/1");
                let mut it = fr.split('/');
                let (n, d): (f64, f64) =
                    (it.next().and_then(|x| x.parse().ok()).unwrap_or(0.0), it.next().and_then(|x| x.parse().ok()).unwrap_or(1.0));
                info.fps = if d > 0.0 { n / d } else { 0.0 };
                let trc = s["color_transfer"].as_str().unwrap_or("");
                let mut color =
                    FrameFormat::from_probe(trc, s["color_space"].as_str().unwrap_or(""), s["color_range"].as_str().unwrap_or(""));
                let pix = s["pix_fmt"].as_str().unwrap_or("");
                color.high_bit = pix.contains("10") || pix.contains("12") || pix.contains("16");
                info.hdr = color.is_hdr();
                info.color = color;
                if info.duration == 0.0 {
                    info.duration = s["duration"].as_str().and_then(|x| x.parse().ok()).unwrap_or(0.0);
                }
            }
            Some("audio") => info.has_audio = true,
            _ => {}
        }
    }
    Ok(info)
}
