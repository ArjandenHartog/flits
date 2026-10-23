//! The GPU and CPU compositors must produce (nearly) the same pixels.

mod common;

use flits::render::FrameRenderer;
use flits::{Prepared, Scene};
use serde_json::json;

fn diff(a: &[u8], b: &[u8]) -> (f64, f64) {
    // (mean abs difference, 99.5th percentile) over RGB
    let mut d: Vec<u8> =
        a.as_chunks::<4>().0.iter().zip(b.as_chunks::<4>().0).flat_map(|(x, y)| (0..3).map(move |i| x[i].abs_diff(y[i]))).collect();
    let mean = d.iter().map(|&v| v as f64).sum::<f64>() / d.len() as f64;
    d.sort_unstable();
    (mean, d[d.len() * 995 / 1000] as f64)
}

fn assert_parity(name: &str, scene: serde_json::Value, frames: &[i64]) {
    let Ok(Some(gpu)) = flits::pick_gpu("auto") else {
        eprintln!("no GPU: skipping");
        return;
    };
    let dir = std::env::temp_dir().join("flits-tests").join(name);
    let (s, warnings) = Scene::from_value(scene, dir.clone()).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    let p = Prepared::build(&s, 1.0).unwrap();
    let mut cpu = FrameRenderer::new(&p, None).unwrap();
    let mut gpu = FrameRenderer::new(&p, Some(gpu)).unwrap();
    for &f in frames {
        let a = cpu.rgba(&p, f).unwrap();
        let b = gpu.rgba(&p, f).unwrap();
        let (mean, p995) = diff(a.data(), b.data());
        eprintln!("{name} frame {f}: mean {mean:.3} p99.5 {p995}");
        if mean > 1.0 || p995 > 24.0 || std::env::var("FLITS_TEST_DUMP").is_ok() {
            a.save_png(dir.join(format!("cpu-{f}.png"))).unwrap();
            b.save_png(dir.join(format!("gpu-{f}.png"))).unwrap();
        }
        assert!(mean <= 1.0 && p995 <= 24.0, "{name} frame {f}: mean diff {mean:.2}, p99.5 {p995} (images in {})", dir.display());
    }
}

#[test]
fn gpu_matches_cpu_presets_and_transitions() {
    let dir = common::tmp("parity-presets");
    let img = common::photo(&dir, "a.png", 900, 600);
    let tr = common::transcript(&dir);
    let scene = json!({
        "width": 1280, "height": 720, "fps": 30, "duration": 3, "background": "#202024",
        "layers": [
            {"type": "image", "src": img, "in": {"type": "iris", "duration": 1.5}},
            {"type": "rect", "x": 260, "y": 200, "width": 300, "height": 180, "radius": 24, "color": "#f26328",
             "in": {"type": "rotate-in", "duration": 1.2}, "out": {"type": "push-left", "duration": 1.5}},
            {"type": "group", "x": 1000, "y": 200, "width": 340, "height": 200, "in": {"type": "flip-x", "duration": 1.2},
             "out": {"type": "zoom-fade", "duration": 1.5}, "children": [
                {"type": "rect", "radius": 20, "gradient": {"type": "linear", "colors": ["#4ac5da", "#123a44"]}},
                {"type": "text", "text": "flip", "size": 60}
            ]},
            {"type": "captions", "src": tr, "preset": "karaoke-box", "y": 360, "size": 44, "width": 900},
            {"type": "captions", "src": tr, "preset": "highlighter", "y": 480, "size": 44, "width": 900},
            {"type": "captions", "src": tr, "preset": "subtitle", "y": 620, "size": 36, "width": 900,
             "speakerColors": {"SPEAKER_00": "#ffd60a"}}
        ]
    });
    assert_parity("parity-presets", scene, &[8, 20, 40, 70]);
}

#[test]
fn gpu_matches_cpu() {
    if !matches!(flits::pick_gpu("auto"), Ok(Some(_))) {
        eprintln!("no GPU: skipping");
        return;
    }
    let dir = common::tmp("parity");
    let img = common::photo(&dir, "a.png", 1200, 800);
    let tr = common::transcript(&dir);
    let clip = common::clip(&dir, "c.mp4", 3.0);
    let scene = json!({
        "width": 1280, "height": 720, "fps": 30, "duration": 3, "background": "#202024",
        "layers": [
            {"type": "image", "src": img, "blur": 30, "zoom": {"from": 1.1, "to": 1.3}},
            {"type": "rect", "gradient": {"type": "radial", "colors": ["#00000000", "#000000aa"]}},
            {"type": "image", "src": img, "height": 420, "x": 820, "y": 360, "radius": 18, "rotation": 4,
             "border": {"width": 10, "color": "#fffaf3"}, "shadow": {"blur": 24, "y": 16}, "zoom": 1.05, "panX": 0.4},
            {"type": "video", "src": clip, "x": 200, "y": 470, "height": 180, "radius": 12, "volume": 0, "zoom": 1.1},
            {"type": "rect", "x": 60, "y": 640, "anchor": [0, 0.5], "width": 500, "height": 14, "radius": 7, "color": "#f26328",
             "scaleX": {"from": 0.2, "to": 1}},
            {"type": "text", "text": "Pariteit tussen\nGPU en CPU", "x": 330, "y": 200, "size": 54, "align": "left",
             "shadow": {"blur": 12, "y": 4}, "stroke": {"width": 2, "color": "#000"}, "in": "pop"},
            {"type": "captions", "src": tr, "y": 520, "x": 330, "width": 560, "size": 40, "style": "karaoke", "highlightBg": "#f26328"},
            {"type": "rect", "x": 640, "y": 360, "width": 200, "height": 120, "color": "#4ac5da", "opacity": 0.5,
             "rotation": -12, "radius": 30, "in": {"type": "wipe-right", "duration": 2}}
        ]
    });
    assert_parity("parity", scene, &[0, 10, 30, 75]);
}
