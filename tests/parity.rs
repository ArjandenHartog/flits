//! The GPU and CPU compositors must produce (nearly) the same pixels.

mod common;

use flits::render::FrameRenderer;
use flits::{Prepared, Scene};
use serde_json::json;

fn diff(a: &[u8], b: &[u8]) -> (f64, f64) {
    // (mean abs difference, 99.5th percentile) over RGB
    let mut d: Vec<u8> = a.chunks_exact(4).zip(b.chunks_exact(4)).flat_map(|(x, y)| (0..3).map(move |i| x[i].abs_diff(y[i]))).collect();
    let mean = d.iter().map(|&v| v as f64).sum::<f64>() / d.len() as f64;
    d.sort_unstable();
    (mean, d[d.len() * 995 / 1000] as f64)
}

#[test]
fn gpu_matches_cpu() {
    let Ok(Some(gpu)) = flits::pick_gpu("auto") else {
        eprintln!("no GPU: skipping");
        return;
    };
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
    let (s, warnings) = Scene::from_value(scene, dir.clone()).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    let p = Prepared::build(&s, 1.0).unwrap();
    let mut cpu = FrameRenderer::new(&p, None).unwrap();
    let mut gpu = FrameRenderer::new(&p, Some(gpu)).unwrap();
    for f in [0, 10, 30, 75] {
        let a = cpu.rgba(&p, f).unwrap();
        let b = gpu.rgba(&p, f).unwrap();
        let (mean, p995) = diff(a.data(), b.data());
        eprintln!("frame {f}: mean {mean:.3} p99.5 {p995}");
        if mean > 1.0 || p995 > 24.0 || std::env::var("FLITS_TEST_DUMP").is_ok() {
            a.save_png(dir.join(format!("cpu-{f}.png"))).unwrap();
            b.save_png(dir.join(format!("gpu-{f}.png"))).unwrap();
        }
        assert!(mean <= 1.0 && p995 <= 24.0, "frame {f}: mean diff {mean:.2}, p99.5 {p995} (images in {})", dir.display());
    }
}
