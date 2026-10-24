//! `flits reel` end to end on a synthetic clip with a synthetic transcript (no whisper needed):
//! a face on the right, a cut, the face on the left, and a pause in the speech. The reel must cut
//! the pause, follow the face across the cut, time the captions to the cut and keep the face in
//! the picture; footage without a face must be shown whole instead of cropped.

mod common;

use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;

fn flits() -> Command {
    Command::new(env!("CARGO_BIN_EXE_flits"))
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
}

/// 6 s at 1280x720: 0-3 s the face right of centre, 3-6 s (another shot) left of centre; a tone
/// that pauses from 2.0 to 3.2 s. `face: false` leaves the face out.
fn footage(dir: &Path, face: bool) -> PathBuf {
    let p = dir.join(if face { "clip.mp4" } else { "noface.mp4" });
    let mut c = Command::new("ffmpeg");
    c.args(["-hide_banner", "-loglevel", "error", "-y"]);
    c.args(["-f", "lavfi", "-i", "color=c=0x404040:s=1280x720:r=30:d=3"]);
    c.args(["-f", "lavfi", "-i", "color=c=0x705030:s=1280x720:r=30:d=3"]);
    c.arg("-i").arg(fixture("face.jpg"));
    c.args(["-f", "lavfi", "-i", r"aevalsrc=if(between(t\,2\,3.2)\,0\,0.3*sin(2*PI*220*t)):s=48000:d=6"]);
    let graph = if face {
        "[2]scale=320:320,split[f1][f2];[0][f1]overlay=860:200[a];[1][f2]overlay=100:200[b];[a][b]concat=n=2:v=1:a=0[v]"
    } else {
        "[0][1]concat=n=2:v=1:a=0[v]"
    };
    c.args(["-filter_complex", graph, "-map", "[v]", "-map", "3", "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest"]);
    assert!(c.arg(&p).status().unwrap().success(), "ffmpeg failed to make the test footage");
    p
}

fn words(dir: &Path) -> PathBuf {
    let w = |word: &str, start: f64, end: f64| json!({"word": word, "start": start, "end": end});
    let v = json!({"language": "en", "segments": [
        {"start": 0.2, "end": 1.8, "text": "This is a test.", "words": [w("This", 0.2, 0.5), w("is", 0.6, 0.8), w("a", 0.9, 1.0), w("test.", 1.1, 1.8)]},
        {"start": 3.4, "end": 5.6, "text": "Of the whole reel.", "words": [w("Of", 3.4, 3.7), w("the", 3.8, 4.0), w("whole", 4.1, 4.6), w("reel.", 4.7, 5.6)]}
    ]});
    let p = dir.join("words.json");
    std::fs::write(&p, v.to_string()).unwrap();
    p
}

fn reel(input: &Path, transcript: &Path, out: &Path, extra: &[&str]) -> Value {
    let o = flits()
        .arg("reel")
        .arg(input)
        .arg("--transcript")
        .arg(transcript)
        .arg("-o")
        .arg(out)
        .args(["--scale", "0.25", "--gpu", "cpu", "--encoder", "cpu"])
        .args(extra)
        .output()
        .unwrap();
    assert!(o.status.success(), "flits reel failed: {}", String::from_utf8_lossy(&o.stderr));
    serde_json::from_str(&std::fs::read_to_string(out.with_extension("scene.json")).unwrap()).unwrap()
}

fn duration(p: &Path) -> f64 {
    let o = Command::new("ffprobe").args(["-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0"]).arg(p).output().unwrap();
    String::from_utf8_lossy(&o.stdout).trim().parse().unwrap()
}

fn clips(scene: &Value) -> Vec<Value> {
    scene["layers"].as_array().unwrap().iter().find(|l| l["type"] == "group").unwrap()["children"].as_array().unwrap().clone()
}

#[test]
fn reel_cuts_the_pause_follows_the_face_and_times_the_captions() {
    if !common::has_ffmpeg() {
        eprintln!("no ffmpeg: skipping");
        return;
    }
    let dir = common::tmp("reel-face");
    let (clip, tr) = (footage(&dir, true), words(&dir));
    let out = dir.join("reel.mp4");
    let scene = reel(&clip, &tr, &out, &[]);

    // the 1.2 s pause becomes 0.2 s: 6 s of footage, 5 s of reel
    assert!((duration(&out) - 5.0).abs() < 0.1, "reel is {} s", duration(&out));
    // two clips around the pause; the crop sits right before the cut, left after it
    let c = clips(&scene);
    assert_eq!(c.len(), 2, "{c:?}");
    assert!(c[0]["panX"].as_f64().unwrap() > 0.5, "first shot should pan right: {}", c[0]["panX"]);
    assert!(c[1]["panX"].as_f64().unwrap() < -0.5, "second shot should pan left: {}", c[1]["panX"]);
    // captions follow the cut: "Of" was spoken at 3.4 s, 1.0 s of pause is gone
    let w: Value = serde_json::from_str(&std::fs::read_to_string(out.with_extension("words.json")).unwrap()).unwrap();
    let of = w["words"].as_array().unwrap().iter().find(|w| w["word"] == "Of").unwrap();
    assert!((of["start"].as_f64().unwrap() - 2.4).abs() < 0.05, "{of}");

    // the face is in the picture: the middle of the frame is not flat background
    let still = dir.join("s.png");
    let o = flits()
        .arg("still")
        .arg(out.with_extension("scene.json"))
        .args(["--at", "1.0", "--scale", "0.5", "-o"])
        .arg(&still)
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    // `flits still` prints the file(s) it wrote
    let written = String::from_utf8_lossy(&o.stdout).lines().next().unwrap_or_default().trim().to_string();
    let img = image::open(&written).unwrap_or_else(|e| panic!("reading still {written:?}: {e}")).to_luma8();
    let (w, h) = img.dimensions();
    let px: Vec<f64> =
        (h * 3 / 10..h / 2).flat_map(|y| (w / 4..w * 3 / 4).map(move |x| (x, y))).map(|(x, y)| img.get_pixel(x, y).0[0] as f64).collect();
    let mean = px.iter().sum::<f64>() / px.len() as f64;
    let sd = (px.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / px.len() as f64).sqrt();
    assert!(sd > 20.0, "the middle of the frame looks empty (sd {sd:.1}): the face is not in the crop");
}

#[test]
fn footage_without_faces_is_shown_whole() {
    if !common::has_ffmpeg() {
        eprintln!("no ffmpeg: skipping");
        return;
    }
    let dir = common::tmp("reel-noface");
    let (clip, tr) = (footage(&dir, false), words(&dir));
    let scene = reel(&clip, &tr, &dir.join("reel.mp4"), &[]);
    // fit, not crop: the picture spans the width at its own aspect ratio, on a blurred backdrop
    let group = scene["layers"].as_array().unwrap().iter().find(|l| l["type"] == "group").unwrap();
    assert!((group["height"].as_f64().unwrap() - 1080.0 * 720.0 / 1280.0).abs() < 1.0, "{group}");
    assert!(scene["layers"].as_array().unwrap().iter().any(|l| l["type"] == "image" && l["blur"].as_f64().unwrap_or(0.0) > 0.0));
    // a fixed focus crops anyway
    let scene = reel(&clip, &tr, &dir.join("crop.mp4"), &["--focus", "0.5"]);
    assert!(clips(&scene).iter().all(|c| c["panX"].as_f64() == Some(0.0)));
}
