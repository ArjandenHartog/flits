//! Shared helpers for integration tests: synthetic assets in a temp dir.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

pub fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join("flits-tests").join(name);
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

pub fn has_ffmpeg() -> bool {
    Command::new("ffmpeg").arg("-version").output().map(|o| o.status.success()).unwrap_or(false)
}

/// A colourful test photo with detail (gradients + checker) so resampling differences show up.
pub fn photo(dir: &Path, name: &str, w: u32, h: u32) -> PathBuf {
    let img = image::RgbImage::from_fn(w, h, |x, y| {
        let c = ((x / 40 + y / 40) % 2) as u8 * 60;
        image::Rgb([(x * 255 / w) as u8 ^ c, (y * 255 / h) as u8, 128u8.wrapping_add(c)])
    });
    let p = dir.join(name);
    img.save(&p).unwrap();
    p
}

/// A short test clip with audio (needs ffmpeg).
pub fn clip(dir: &Path, name: &str, secs: f64) -> PathBuf {
    let p = dir.join(name);
    let ok = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i"])
        .arg(format!("testsrc2=size=640x360:rate=30:duration={secs}"))
        .args(["-f", "lavfi", "-i"])
        .arg(format!("sine=frequency=440:duration={secs}"))
        .args(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest"])
        .arg(&p)
        .status()
        .unwrap()
        .success();
    assert!(ok, "ffmpeg failed to make a test clip");
    p
}

pub fn transcript(dir: &Path) -> PathBuf {
    let p = dir.join("t.json");
    let words: Vec<serde_json::Value> = "dit is een korte test van de ondertitels in flits"
        .split(' ')
        .enumerate()
        .map(|(i, w)| serde_json::json!({"word": w, "start": i as f64 * 0.3, "end": i as f64 * 0.3 + 0.25}))
        .collect();
    std::fs::write(&p, serde_json::json!({"segments": [{"start": 0.0, "end": 3.0, "text": "", "words": words}]}).to_string()).unwrap();
    p
}

pub fn probe_frames(p: &Path) -> i64 {
    let o = Command::new("ffprobe")
        .args(["-v", "error", "-count_packets", "-select_streams", "v:0", "-show_entries", "stream=nb_read_packets", "-of", "csv=p=0"])
        .arg(p)
        .output()
        .unwrap();
    String::from_utf8_lossy(&o.stdout).trim().parse().unwrap()
}

pub fn has_audio(p: &Path) -> bool {
    let o = Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "a", "-show_entries", "stream=codec_name", "-of", "csv=p=0"])
        .arg(p)
        .output()
        .unwrap();
    !String::from_utf8_lossy(&o.stdout).trim().is_empty()
}
