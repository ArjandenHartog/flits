//! Decoding only the part of a video that shows must not change the picture.

mod common;

use flits::compose::{Kind, Node};
use flits::render::FrameRenderer;
use flits::{Prepared, Scene};
use serde_json::json;
use std::path::Path;
use std::process::Command;

/// A 16:9 clip with fine detail everywhere (so a misplaced crop shows) and a frame counter.
fn wide_clip(dir: &Path) -> std::path::PathBuf {
    let p = dir.join("wide.mp4");
    let ok = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=1280x720:rate=25:duration=3"])
        .args(["-c:v", "libx264", "-g", "30", "-pix_fmt", "yuv420p"])
        .arg(&p)
        .status()
        .unwrap()
        .success();
    assert!(ok, "ffmpeg failed to make a test clip");
    p
}

fn video_crops(nodes: &[Node], out: &mut Vec<Option<(u32, u32, u32, u32)>>, plans: &mut Vec<usize>) {
    for n in nodes {
        if let Kind::Video { crop, .. } = &n.kind {
            out.push(crop.as_ref().map(|c| (c.x, c.y, c.w, c.h)));
            if let Some(c) = crop {
                plans.push(std::sync::Arc::as_ptr(&c.plan) as usize);
            }
        }
        video_crops(&n.children, out, plans);
    }
}

/// Render `frames` (in order, as a render does) with and without cropping; the largest per-channel
/// difference and the mean difference over all of them.
fn compare(dir: &Path, scene: serde_json::Value, frames: impl Iterator<Item = i64> + Clone) -> (u8, f64) {
    let (s, _) = Scene::from_value(scene, dir.to_path_buf()).unwrap();
    let cropped = Prepared::build(&s, 1.0).unwrap();
    let mut whole = Prepared::build(&s, 1.0).unwrap();
    whole.decode_whole_frames();
    let (mut a, mut b) = (FrameRenderer::new(&cropped, None).unwrap(), FrameRenderer::new(&whole, None).unwrap());
    let (mut max, mut sum, mut n) = (0u8, 0f64, 0usize);
    for f in frames {
        let x = a.rgba(&cropped, f).unwrap();
        let y = b.rgba(&whole, f).unwrap();
        for (p, q) in x.data().iter().zip(y.data()) {
            let d = p.abs_diff(*q);
            max = max.max(d);
            sum += d as f64;
            n += 1;
        }
    }
    (max, sum / n as f64)
}

#[test]
fn a_cropped_decode_shows_the_same_picture() {
    if !common::has_ffmpeg() {
        return;
    }
    let dir = common::tmp("crop-same");
    let clip = wide_clip(&dir);
    // a 9:16 frame of 16:9 footage: a third of each frame shows
    let scene = json!({"width": 720, "height": 1280, "fps": 25, "duration": 2,
        "layers": [{"type": "video", "src": clip, "trim": 0.5, "fit": "cover", "panX": 0.4}]});
    let (s, _) = Scene::from_value(scene.clone(), dir.clone()).unwrap();
    let p = Prepared::build(&s, 1.0).unwrap();
    let (mut crops, mut plans) = (vec![], vec![]);
    video_crops(&p.nodes, &mut crops, &mut plans);
    let (_, _, w, h) = crops[0].expect("a constant framing is cropped");
    assert!(w < 1280 / 2 && h == 720, "{w}x{h}");
    // unscaled: pixel for pixel the same, also for a decoder started mid-clip (a still)
    let (max, _) = compare(&dir, scene.clone(), 0..50);
    assert_eq!(max, 0, "cropped decode differs");
    let (max, _) = compare(&dir, scene, [37].into_iter());
    assert_eq!(max, 0, "cropped decode started mid-clip differs");
}

#[test]
fn shots_of_one_take_share_a_decoder_and_join_seamlessly() {
    if !common::has_ffmpeg() {
        return;
    }
    let dir = common::tmp("crop-shots");
    let clip = wide_clip(&dir);
    // how a reel plays a take: one layer per framing, contiguous trims, parts on whole frames,
    // one punched in, then a jump cut over a gap at a time between frames
    let v = |trim: f64, dur: f64, pan: f64, zoom: f64| json!({"type": "video", "src": clip, "trim": trim, "duration": dur, "fit": "cover", "panX": pan, "zoom": zoom});
    let scene = json!({"width": 720, "height": 1280, "fps": 25, "duration": 2.3,
        "layers": [{"type": "group", "sequence": true, "children": [
            v(0.0, 0.48, -0.8, 1.0), v(0.48, 0.6, 0.7, 1.0), v(1.08, 0.4, 0.0, 1.1), v(1.613, 0.82, 0.3, 1.0)]}]});
    let (s, _) = Scene::from_value(scene.clone(), dir.clone()).unwrap();
    let p = Prepared::build(&s, 1.0).unwrap();
    let (mut crops, mut plans) = (vec![], vec![]);
    video_crops(&p.nodes, &mut crops, &mut plans);
    assert!(crops.iter().all(Option::is_some), "{crops:?}");
    // one crop size and one plan: a single decoder runs through all the shots
    assert!(crops.iter().all(|c| c.unwrap().2 == crops[0].unwrap().2));
    plans.dedup();
    assert_eq!(plans.len(), 1);
    // every frame, through every join; the punched-in shot is resampled at a sub-pixel offset
    let (max, mean) = compare(&dir, scene, 0..57);
    assert!(max <= 3 && mean < 0.01, "max {max} mean {mean}");
}

#[test]
fn rotated_footage_is_cropped_in_place() {
    if !common::has_ffmpeg() {
        return;
    }
    let dir = common::tmp("crop-rotated");
    let clip = wide_clip(&dir);
    for rot in [90, 270, 180] {
        // phone footage: stored landscape, shown turned
        let turned = dir.join(format!("turned{rot}.mp4"));
        let ok = Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-y", "-display_rotation", &rot.to_string(), "-i"])
            .arg(&clip)
            .args(["-c", "copy"])
            .arg(&turned)
            .status()
            .unwrap()
            .success();
        assert!(ok);
        // a wide frame of the upright picture: a band across it shows, off centre
        let scene = json!({"width": 1280, "height": 720, "fps": 25, "duration": 1,
            "layers": [{"type": "video", "src": turned, "fit": "cover", "panX": 0.3, "panY": -0.6}]});
        // (offsets in both directions: rounding of the texture offset can show as 1 level)
        let (max, mean) = compare(&dir, scene, 0..25);
        assert!(max <= 1 && mean < 0.01, "rotation {rot}: max {max} mean {mean}");
    }
}
