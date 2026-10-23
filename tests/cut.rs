//! `flits cut` end to end on synthetic media with a synthetic transcript (no whisper needed):
//! silences + filler words on audio, and the --edit / --apply / --export flow on a 29.97 fps video.

mod common;

use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;

fn flits() -> Command {
    Command::new(env!("CARGO_BIN_EXE_flits"))
}

/// 8 s of tone with silence from 2 to 4 s (as audio, or as a 29.97 fps video with that sound).
fn media(dir: &Path, name: &str, video: bool) -> PathBuf {
    let p = dir.join(name);
    let mut c = Command::new("ffmpeg");
    c.args(["-hide_banner", "-loglevel", "error", "-y"]);
    if video {
        c.args(["-f", "lavfi", "-i", "testsrc2=size=320x180:rate=30000/1001:duration=8"]);
    }
    c.args(["-f", "lavfi", "-i", r"aevalsrc=if(between(t\,2\,4)\,0\,0.5*sin(440*2*PI*t)):d=8:s=48000"]);
    if video {
        c.args(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest"]);
    }
    assert!(c.arg(&p).status().unwrap().success(), "ffmpeg failed to make {name}");
    p
}

/// `<stem>.words.json` next to the media, so `flits cut` finds it without transcribing.
fn transcript(media: &Path) -> PathBuf {
    let w = |word: &str, start: f64, end: f64| json!({"word": word, "start": start, "end": end});
    let v = json!({"language": "en", "segments": [
        {"start": 0.2, "end": 1.9, "text": "Hello uh world.", "words": [w("Hello", 0.2, 0.6), w("uh", 0.8, 1.2), w("world.", 1.3, 1.9)]},
        {"start": 4.2, "end": 7.5, "text": "This is a test sentence.",
         "words": [w("This", 4.2, 4.5), w("is", 4.6, 4.8), w("a", 4.9, 5.0), w("test", 5.2, 5.8), w("sentence.", 6.0, 7.5)]}
    ]});
    let stem = media.file_stem().unwrap().to_str().unwrap();
    let p = media.with_file_name(format!("{stem}.words.json"));
    std::fs::write(&p, v.to_string()).unwrap();
    p
}

fn duration(p: &Path) -> f64 {
    let o = Command::new("ffprobe").args(["-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0"]).arg(p).output().unwrap();
    String::from_utf8_lossy(&o.stdout).trim().parse().unwrap()
}

fn run(c: &mut Command) -> String {
    let o = c.output().unwrap();
    assert!(o.status.success(), "flits failed: {}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8_lossy(&o.stderr).into_owned()
}

#[test]
fn cuts_silences_and_fillers_from_audio() {
    if !common::has_ffmpeg() {
        return;
    }
    let dir = common::tmp("cut-audio");
    let a = media(&dir, "a.wav", false);
    transcript(&a);
    let out = dir.join("out.wav");
    let log = run(flits().arg("cut").arg(&a).args(["--fillers", "--keep", "0.2", "-o"]).arg(&out));
    assert!(log.contains("filler words (en)"), "{log}");
    // silence 2..4 s shrinks to 0.2 s (-1.8 s); "uh" goes from 0.7 to 1.25 s (-0.55 s)
    let d = duration(&out);
    assert!((d - (8.0 - 1.8 - 0.55)).abs() < 0.15, "duration {d}");
}

#[test]
fn edit_apply_and_export_timelines() {
    if !common::has_ffmpeg() {
        return;
    }
    let dir = common::tmp("cut-edit");
    let v = media(&dir, "v.mp4", true);
    transcript(&v);
    // 1. the edit file: one line per sentence
    run(flits().arg("cut").arg(&v).arg("--edit"));
    let edit = dir.join("v.edit.txt");
    let text = std::fs::read_to_string(&edit).unwrap();
    let lines: Vec<&str> = text.lines().filter(|l| l.starts_with('[')).collect();
    assert_eq!(lines, ["[00:00.20] Hello uh world.", "[00:04.20] This is a test sentence."]);
    // a second --edit must not overwrite the user's work
    assert!(!flits().arg("cut").arg(&v).arg("--edit").output().unwrap().status.success());

    // 2. cut "uh" with [[ ]] and "is a" by deleting the words; no silence tightening
    let edited = text.replace("Hello uh world.", "Hello [[uh]] world.").replace("This is a test", "This test");
    std::fs::write(&edit, edited).unwrap();
    let out = dir.join("final.mp4");
    let log = run(flits()
        .arg("cut")
        .arg(&v)
        .arg("--apply")
        .arg(&edit)
        .args(["--no-silence", "--export", "fcpxml,premiere,edl,json", "-o"])
        .arg(&out));
    assert!(log.contains("keeps 2 line(s)"), "{log}");
    assert!(!out.exists(), "--export without --render must not render");

    // 3. the cut list: "uh" (0.7..1.25) and "is a" (4.55..5.1) are gone, the rest stays
    let cuts: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("final.cuts.json")).unwrap()).unwrap();
    let ranges: Vec<(f64, f64)> =
        cuts["ranges"].as_array().unwrap().iter().map(|r| (r["start"].as_f64().unwrap(), r["end"].as_f64().unwrap())).collect();
    let near = |a: f64, b: f64| (a - b).abs() < 1e-6;
    assert_eq!(ranges.len(), 3, "{ranges:?}");
    assert!(near(ranges[0].0, 0.0) && near(ranges[0].1, 0.7), "{ranges:?}");
    assert!(near(ranges[1].0, 1.25) && near(ranges[1].1, 4.55), "{ranges:?}");
    // (the container may be a frame longer than 8 s)
    assert!(near(ranges[2].0, 5.1) && (ranges[2].1 - 8.0).abs() < 0.05, "{ranges:?}");
    assert_eq!(cuts["timebase"], "30000/1001");

    // 4. the editor files are there and reference the source
    let fcpxml = std::fs::read_to_string(dir.join("final.fcpxml")).unwrap();
    assert_eq!(fcpxml.matches("<asset-clip ").count(), 3);
    assert!(fcpxml.contains("frameDuration=\"1001/30000s\"") && fcpxml.contains("v.mp4"));
    let xml = std::fs::read_to_string(dir.join("final.xml")).unwrap();
    assert_eq!(xml.matches("<clipitem ").count(), 6, "3 video + 3 audio clip items");
    assert!(xml.contains("<ntsc>TRUE</ntsc>"));
    let edl = std::fs::read_to_string(dir.join("final.edl")).unwrap();
    assert_eq!(edl.lines().filter(|l| l.starts_with("00")).count(), 3);

    // 5. --dry-run on the same edit prints the plan
    let o = flits().arg("cut").arg(&v).arg("--apply").arg(&edit).args(["--no-silence", "--dry-run"]).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&o.stdout).lines().filter(|l| l.starts_with("keep ")).count(), 3);
}
