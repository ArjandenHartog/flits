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
