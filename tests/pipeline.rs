//! End-to-end: scene -> MP4 with the right number of frames and an audio track.

mod common;

use serde_json::json;

fn render(name: &str, scene: serde_json::Value, compositor: &str, encoder: &str) -> std::path::PathBuf {
    let dir = common::tmp(name);
    let out = dir.join("out.mp4");
    let o = flits::Options { compositor: compositor.into(), encoder: encoder.into(), workers: Some(3), ..Default::default() };
    let stats = flits::render_json(scene, &dir, &out, &o).unwrap();
    assert!(stats.frames > 0);
    out
}

fn scene(dir_name: &str) -> serde_json::Value {
    let dir = common::tmp(&format!("{dir_name}-assets"));
    let img = common::photo(&dir, "p.png", 800, 600);
    let clip = common::clip(&dir, "c.mp4", 2.0);
    json!({
        "width": 638, "height": 360, "fps": 25, "background": "#111",
        "layers": [
            {"type": "group", "sequence": true, "overlap": 0.5, "children": [
                {"type": "image", "src": img, "duration": 1.5, "zoom": {"from": 1, "to": 1.2}},
                {"type": "video", "src": clip, "in": "fade", "volume": 0.5},
                {"type": "text", "text": "einde", "duration": 1.0, "in": "pop"}
            ]}
        ]
    })
}

#[test]
fn cpu_x264_pipeline() {
    if !common::has_ffmpeg() {
        return;
    }
    let out = render("pipe-cpu", scene("pipe-cpu"), "cpu", "x264");
    // 1.5 + 2.0 + 1.0 - 2 * 0.5 overlap = 3.5 s at 25 fps
    assert_eq!(common::probe_frames(&out), 88);
    assert!(common::has_audio(&out));
}

#[test]
fn gpu_auto_pipeline_odd_width() {
    if !common::has_ffmpeg() {
        return;
    }
    // width 638 is not a multiple of 8: GPU frames are padded and cropped by the encoder
    let out = render("pipe-gpu", scene("pipe-gpu"), "auto", "auto");
    assert_eq!(common::probe_frames(&out), 88);
    let o = std::process::Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries", "stream=width,height", "-of", "csv=p=0"])
        .arg(&out)
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&o.stdout).trim(), "638,360");
}

#[test]
fn audio_ducking_loudness_and_speed() {
    if !common::has_ffmpeg() {
        return;
    }
    let dir = common::tmp("pipe-audio-assets");
    let clip = common::clip(&dir, "c.mp4", 4.0);
    let music = common::clip(&dir, "m.mp4", 6.0);
    let scene = json!({
        "width": 320, "height": 180, "fps": 25, "loudness": -16,
        "audio": [{"src": music, "duck": 0.2, "volume": [[0, 0], [0.5, 1, "out"], ["end-0.5", 1], ["end", 0]]}],
        "layers": [{"type": "video", "src": clip, "speed": 2.0}]
    });
    let out = render("pipe-audio", scene, "cpu", "cpu");
    // 4 s clip at 2x speed = 2 s
    assert_eq!(common::probe_frames(&out), 50);
    assert!(common::has_audio(&out));
}
