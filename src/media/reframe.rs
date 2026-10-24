//! Automatic reframing for vertical crops: where are the people in each shot?
//!
//! One ffmpeg pass decodes just the chosen parts, small and grey. Shot changes (multi-camera
//! edits) are found on every frame from the picture difference; faces are looked for a few times
//! per second with SeetaFace (pure Rust, model built in). Each shot then gets one framing, the way
//! an editor would set it: no drifting crop inside a shot, a jump exactly on the cut.

use super::ffmpeg;
use anyhow::{bail, Context, Result};
use rayon::prelude::*;
use std::io::Read;
use std::path::Path;
use std::process::Stdio;

const MODEL: &[u8] = include_bytes!("../../assets/seeta_fd_frontal_v1.0.bin");
/// analysis width (px)
const AW: u32 = 768;
/// look for faces this often (s)
const FACE_EVERY: f64 = 0.33;
/// mean absolute difference (0..255) of the 16x9 thumbnails that counts as a cut
const CUT_DIFF: f64 = 22.0;

/// One framing: from `start` (source seconds) the point of interest is at `x` (0..1 of the width).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Framing {
    pub start: f64,
    pub x: f64,
    /// a face was found in the shot (else `x` is the middle)
    pub face: bool,
}

/// Framings for the source window `[a, b)`: one per shot. `visible` is the fraction of the source
/// width the crop shows (two faces closer together than that are framed together).
pub fn analyse(src: &Path, a: f64, b: f64, fps: f64, visible: f64) -> Result<Vec<Framing>> {
    let fps = if fps > 0.0 { fps } else { 25.0 };
    let step = ((FACE_EVERY * fps).round() as usize).max(1);
    let min_shot = (fps * 0.4) as usize;

    // stream the frames: a thumbnail of every one (for the cuts), the whole picture only of the
    // frames faces are looked for in
    let (w, h, mut child) = spawn_grey(src, a, b)?;
    let size = (w * h) as usize;
    let mut out = child.stdout.take().unwrap();
    let mut frame = vec![0u8; size];
    let mut shots = vec![0usize];
    let mut probes: Vec<(usize, Vec<u8>)> = vec![];
    let mut prev: Option<[f64; 144]> = None;
    let mut n = 0usize;
    while out.read_exact(&mut frame).is_ok() {
        let t = thumb(&frame, w, h);
        let mut cut = false;
        if let Some(p) = prev {
            let d: f64 = t.iter().zip(&p).map(|(x, y)| (x - y).abs()).sum::<f64>() / 144.0;
            if d > CUT_DIFF && n - shots.last().unwrap() > min_shot {
                shots.push(n);
                cut = true;
            }
        }
        if n.is_multiple_of(step) || cut {
            probes.push((n, frame.clone()));
        }
        prev = Some(t);
        n += 1;
    }
    child.wait()?;
    if n == 0 {
        bail!("no frames decoded from {} ({a:.1}..{b:.1}s)", src.display());
    }
    shots.push(n);

    let found: Vec<(usize, Option<f64>)> =
        probes.par_iter().map_init(detector, |det, (i, px)| (*i, focus_x(det.as_mut(), px, w, h, visible))).collect();

    // one framing per shot: the median of its detections; a shot without a face (a wide shot, a
    // profile) is framed in the middle, which suits wide shots best
    let mut framings: Vec<Framing> = vec![];
    for s in shots.windows(2) {
        let mut xs: Vec<f64> = found.iter().filter(|(i, _)| *i >= s[0] && *i < s[1]).filter_map(|(_, x)| *x).collect();
        let face = !xs.is_empty();
        let x = if xs.is_empty() {
            0.5
        } else {
            xs.sort_by(f64::total_cmp);
            xs[xs.len() / 2]
        };
        if std::env::var_os("FLITS_DEBUG_REFRAME").is_some() {
            let det: Vec<String> = found
                .iter()
                .filter(|(i, _)| *i >= s[0] && *i < s[1])
                .map(|(_, x)| x.map(|v| format!("{v:.2}")).unwrap_or("-".into()))
                .collect();
            eprintln!("shot {:7.2}s..{:7.2}s  x {x:.2}  faces [{}]", a + s[0] as f64 / fps, a + s[1] as f64 / fps, det.join(" "));
        }
        if framings.last().is_none_or(|f: &Framing| (f.x - x).abs() > 0.02) {
            framings.push(Framing { start: a + s[0] as f64 / fps, x, face });
        }
    }
    Ok(framings)
}

fn detector() -> Box<dyn rustface::Detector> {
    let model = rustface::read_model(MODEL).expect("built-in face model");
    let mut d = rustface::create_detector_with_model(model);
    d.set_min_face_size(20); // the smallest the model allows
    d.set_score_thresh(2.0);
    d.set_pyramid_scale_factor(0.8);
    d.set_slide_window_step(4, 4);
    d
}

/// Where to frame one picture: the biggest face, or the middle of two faces that fit the crop
/// together (an interview two-shot). `None` without a face.
fn focus_x(det: &mut dyn rustface::Detector, grey: &[u8], w: u32, h: u32, visible: f64) -> Option<f64> {
    let mut faces = det.detect(&rustface::ImageData::new(grey, w, h));
    faces.sort_by_key(|f| std::cmp::Reverse(f.bbox().width() as u64 * f.bbox().height() as u64));
    let centre = |f: &rustface::FaceInfo| (f.bbox().x() as f64 + f.bbox().width() as f64 / 2.0) / w as f64;
    let big = faces.first()?;
    let x = centre(big);
    if let Some(second) = faces.get(1) {
        let similar = second.bbox().width() as f64 > big.bbox().width() as f64 * 0.7;
        let x2 = centre(second);
        if similar && (x - x2).abs() < visible * 0.75 {
            return Some((x + x2) / 2.0);
        }
    }
    Some(x)
}

/// 16x9 grid of mean brightness.
fn thumb(px: &[u8], w: u32, h: u32) -> [f64; 144] {
    let mut t = [0.0; 144];
    let (cw, ch) = (w as usize / 16, h as usize / 9);
    for gy in 0..9 {
        for gx in 0..16 {
            let mut s = 0u64;
            for y in (gy * ch..(gy + 1) * ch).step_by(2) {
                let row = &px[y * w as usize + gx * cw..y * w as usize + (gx + 1) * cw];
                s += row.iter().step_by(2).map(|&v| v as u64).sum::<u64>();
            }
            t[gy * 16 + gx] = s as f64 / ((cw / 2).max(1) * (ch / 2).max(1)) as f64;
        }
    }
    t
}

/// ffmpeg decoding `[a, b)` of the video as `AW`-wide grey frames on stdout: (width, height, process).
fn spawn_grey(src: &Path, a: f64, b: f64) -> Result<(u32, u32, std::process::Child)> {
    let info = super::probe(src)?;
    let h = ((AW as f64 * info.height as f64 / info.width.max(1) as f64 / 2.0).round() * 2.0) as u32;
    let mut cmd = ffmpeg();
    cmd.args(["-ss", &format!("{a:.3}"), "-t", &format!("{:.3}", (b - a).max(0.04))]).arg("-i").arg(src);
    cmd.args(["-an", "-vf", &format!("scale={AW}:{h}:flags=fast_bilinear,format=gray"), "-f", "rawvideo", "-"]);
    let child = cmd.stdout(Stdio::piped()).stderr(Stdio::null()).spawn().context("starting ffmpeg for reframing")?;
    Ok((AW, h, child))
}

/// Pan (-1..1, the `panX` of a cover-fitted video) that puts source point `x` (0..1) in the middle
/// of a crop showing `visible` of the width, as far as the edges allow.
pub fn pan_for(x: f64, visible: f64) -> f64 {
    if visible >= 1.0 {
        return 0.0;
    }
    (2.0 * (x - visible / 2.0) / (1.0 - visible) - 1.0).clamp(-1.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pan_maps_points_to_the_crop() {
        let v = 1080.0 / (1920.0 * 1920.0 / 1080.0); // 9:16 out of 16:9
        assert!((pan_for(0.5, v)).abs() < 1e-9);
        assert_eq!(pan_for(0.0, v), -1.0);
        assert_eq!(pan_for(1.0, v), 1.0);
        // a point a quarter in: the crop's centre lands on it
        let p = pan_for(0.25, v);
        let left = (1.0 + p) / 2.0 * (1.0 - v);
        assert!((left + v / 2.0 - 0.25).abs() < 1e-9);
    }

    #[test]
    fn model_loads() {
        let mut d = detector();
        let img = vec![128u8; 64 * 48];
        assert!(d.detect(&rustface::ImageData::new(&img, 64, 48)).is_empty());
    }
}
