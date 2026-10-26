//! Inspection commands on plain media files: `flits sheet` / `flits still` on a video (an implicit
//! full-frame scene).

mod common;

use std::process::Command;

fn flits() -> Command {
    Command::new(env!("CARGO_BIN_EXE_flits"))
}

fn run(c: &mut Command) -> String {
    let o = c.output().unwrap();
    assert!(o.status.success(), "flits failed: {}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8_lossy(&o.stdout).into_owned()
}

#[test]
fn sheet_and_still_take_a_video() {
    if !common::has_ffmpeg() {
        return;
    }
    let dir = common::tmp("media-sheet");
    let clip = common::clip(&dir, "clip.mp4", 2.0);

    // still: the frame at the clip's own size
    let still = dir.join("still.png");
    let out = run(flits().arg("still").arg(&clip).args(["--at", "1"]).arg("-o").arg(&still));
    assert!(out.contains("still.png"), "{out}");
    let img = image::open(&still).unwrap();
    assert_eq!((img.width(), img.height()), (640, 360));
    // testsrc2 is colourful: the frame is not empty/black
    let px = img.to_rgb8();
    let bright = px.pixels().filter(|p| p.0.iter().map(|&c| c as u32).sum::<u32>() > 150).count();
    assert!(bright > px.pixels().len() / 4, "the still looks empty");

    // sheet: 4 tiles of 160 px in 2 columns, a time range
    let sheet = dir.join("sheet.png");
    run(flits()
        .arg("sheet")
        .arg(&clip)
        .args(["--count", "4", "--cols", "2", "--thumb", "160", "--from", "0.2", "--to", "1.8"])
        .arg("-o")
        .arg(&sheet));
    let img = image::open(&sheet).unwrap();
    assert_eq!(img.width(), 2 * (160 + 8) + 8);
    assert_eq!(img.height(), 2 * (90 + 34 + 8) + 8);

    // an image works too
    let photo = common::photo(&dir, "photo.png", 300, 200);
    let still = dir.join("photo-still.png");
    run(flits().arg("still").arg(&photo).args(["--at", "0", "--scale", "0.5"]).arg("-o").arg(&still));
    let img = image::open(&still).unwrap();
    assert_eq!((img.width(), img.height()), (150, 100));
}

#[test]
fn info_peaks_finds_a_burst() {
    if !common::has_ffmpeg() {
        return;
    }
    let dir = common::tmp("media-peaks");
    // 6 s of a quiet tone with a loud burst centred on 3.6 s
    let wav = dir.join("burst.wav");
    let ok = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i"])
        .arg(r"aevalsrc=(0.01+0.8*exp(-pow((t-3.6)/0.15\,2)))*sin(300*2*PI*t):d=6:s=44100")
        .arg(&wav)
        .status()
        .unwrap()
        .success();
    assert!(ok, "ffmpeg failed to make the test sound");

    let out = run(flits().arg("info").arg(&wav).args(["--peaks", "--json"]));
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    let t = v["loudest"]["time"].as_f64().unwrap();
    assert!((t - 3.6).abs() < 0.03, "loudest at {t}");
    let db = v["loudest"]["db"].as_f64().unwrap();
    assert!(db > -8.0 && db < -3.0, "loudest {db} dBFS");
    assert!((v["peak"]["time"].as_f64().unwrap() - 3.6).abs() < 0.03);
    assert_eq!(v["step"].as_f64(), Some(1.0));
    let levels: Vec<f64> = v["levels"].as_array().unwrap().iter().map(|l| l.as_f64().unwrap()).collect();
    assert_eq!(levels.len(), 6);
    // the quiet tone: 0.01 amplitude = -43 dBFS RMS; second 3 holds the burst
    assert!((levels[0] + 43.0).abs() < 1.0, "{levels:?}");
    let loudest = levels.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).unwrap().0;
    assert_eq!(loudest, 3, "{levels:?}");

    // the text form, and a video's sound
    let out = run(flits().arg("info").arg(&wav).args(["--peaks", "--window", "0.1"]));
    assert!(out.contains("loudest    3.6"), "{out}");
    let clip = common::clip(&dir, "clip.mp4", 2.0);
    let out = run(flits().arg("info").arg(&clip).arg("--peaks"));
    assert!(out.contains("levels"), "{out}");
}
