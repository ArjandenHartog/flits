//! Automatic reframing for vertical crops: where are the people in each shot, and who talks?
//!
//! One ffmpeg pass decodes just the chosen parts, small and grey. Shot changes (multi-camera
//! edits) are found on every frame from the picture difference; faces are looked for a few times
//! per second with SeetaFace (pure Rust, model built in). Each shot then gets one framing, the way
//! an editor would set it: no drifting crop inside a shot, a jump exactly on the cut. Framings
//! also carry the height of the faces in the crop, so captions can keep clear of them.
//!
//! A two-shot whose people don't fit the crop together follows the speaker instead: a hard cut
//! to whoever talks, only on turns that last (see [`turns`]). Who talks comes from the
//! transcript's diarization labels when it has them (mapped to faces by mouth movement), else
//! from the mouth movement alone, read from a second, sharper decode of just that shot.

use super::ffmpeg;
use crate::compose::captions::Word;
use anyhow::{bail, Context, Result};
use rayon::prelude::*;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;

const MODEL: &[u8] = include_bytes!("../../assets/seeta_fd_frontal_v1.0.bin");
/// analysis width (px)
const AW: u32 = 768;
/// look for faces this often (s)
const FACE_EVERY: f64 = 0.33;
/// mean absolute difference (0..255) of the 16x9 thumbnails that counts as a cut
const CUT_DIFF: f64 = 22.0;
/// width (px) of the sharper decode that reads mouths in two-shots
const MOUTH_W: u32 = 1280;
/// a speaker turn must last this long (s) before the crop cuts to the speaker
pub const MIN_TURN: f64 = 1.2;

/// One framing: from `start` (source seconds) the point of interest is at `x` (0..1 of the width).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Framing {
    pub start: f64,
    pub x: f64,
    /// a face was found in the shot (else `x` is the middle)
    pub face: bool,
    /// vertical extent (top, bottom; 0..1 of the source height) of the faces the crop shows
    pub band: Option<(f64, f64)>,
}

/// A face in one picture, in fractions of the picture: centre `x`, `top` and `bottom` of the
/// face box (about eyebrows to mouth), width `w`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Face {
    pub x: f64,
    pub top: f64,
    pub bottom: f64,
    pub w: f64,
}

/// One person in a shot: their face (medians over the shot) and in how many probes it was seen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Person {
    pub face: Face,
    pub seen: usize,
}

/// One shot of the analysed window.
pub struct Shot {
    /// source seconds
    pub start: f64,
    pub end: f64,
    /// (source time, faces) a few times per second
    pub probes: Vec<(f64, Vec<Face>)>,
    src: PathBuf,
    fps: f64,
}

/// The shots of the source window `[a, b)`, with the faces in them.
pub fn analyse(src: &Path, a: f64, b: f64, fps: f64) -> Result<Vec<Shot>> {
    let fps = if fps > 0.0 { fps } else { 25.0 };
    let step = ((FACE_EVERY * fps).round() as usize).max(1);
    let min_shot = (fps * 0.4) as usize;

    // stream the frames: a thumbnail of every one (for the cuts), the whole picture only of the
    // frames faces are looked for in
    let (w, h, mut child) = spawn_grey(src, a, b, AW)?;
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

    let found: Vec<(usize, Vec<Face>)> = probes.par_iter().map_init(detector, |det, (i, px)| (*i, faces(det.as_mut(), px, w, h))).collect();

    Ok(shots
        .windows(2)
        .map(|s| Shot {
            start: a + s[0] as f64 / fps,
            end: a + s[1] as f64 / fps,
            probes: found.iter().filter(|(i, _)| *i >= s[0] && *i < s[1]).map(|(i, f)| (a + *i as f64 / fps, f.clone())).collect(),
            src: src.to_path_buf(),
            fps,
        })
        .collect())
}

/// Framings for analysed shots: one per shot, or one per speaker turn in a two-shot whose people
/// don't fit the crop together. `visible` is the fraction of the source width the crop shows;
/// `words` (source times) may carry diarization `speaker` labels.
pub fn framings(shots: &[Shot], visible: f64, words: &[Word]) -> Vec<Framing> {
    let debug = std::env::var_os("FLITS_DEBUG_REFRAME").is_some();
    let mut out: Vec<Framing> = vec![];
    let mut push = |f: Framing| {
        if out.last().is_none_or(|l: &Framing| !l.same(&f)) {
            out.push(f);
        }
    };
    for s in shots {
        let ppl = people(&s.probes);
        if debug {
            let p: Vec<String> = ppl
                .iter()
                .map(|p| format!("x {:.2} y {:.2}-{:.2} w {:.3} seen {}", p.face.x, p.face.top, p.face.bottom, p.face.w, p.seen))
                .collect();
            eprintln!("shot {:7.2}s..{:7.2}s  {} probes  people [{}]", s.start, s.end, s.probes.len(), p.join("; "));
        }
        let at = |start: f64, x: f64| Framing { start, x, face: true, band: band(&ppl, x, visible) };
        match ppl.as_slice() {
            [] => push(Framing { start: s.start, x: 0.5, face: false, band: None }),
            // two people about as big: together when they fit, else whoever talks
            [p, q, ..] if q.face.w > p.face.w * 0.7 => {
                if (p.face.x - q.face.x).abs() < visible * 0.75 {
                    push(at(s.start, (p.face.x + q.face.x) / 2.0));
                } else {
                    let pair = if p.face.x < q.face.x { [*p, *q] } else { [*q, *p] };
                    let tl = s.follow(&pair, words);
                    if debug {
                        let t: Vec<String> = tl.iter().map(|(t, i)| format!("{t:.2}s {}", ["left", "right"][*i])).collect();
                        eprintln!("    two-shot: {}", t.join(", "));
                    }
                    for (t, i) in tl {
                        push(at(t, pair[i].face.x));
                    }
                }
            }
            [p, ..] => push(at(s.start, p.face.x)),
        }
    }
    out
}

impl Framing {
    /// Same picture as `o` (no new key needed).
    fn same(&self, o: &Framing) -> bool {
        let band = match (self.band, o.band) {
            (None, None) => true,
            (Some(a), Some(b)) => (a.0 - b.0).abs() < 0.03 && (a.1 - b.1).abs() < 0.03,
            _ => false,
        };
        (self.x - o.x).abs() <= 0.02 && band
    }
}

/// Vertical extent of the faces a crop around `x` shows (`visible` of the width).
fn band(ppl: &[Person], x: f64, visible: f64) -> Option<(f64, f64)> {
    let v = visible.min(1.0);
    let left = (x - v / 2.0).clamp(0.0, 1.0 - v);
    ppl.iter()
        .filter(|p| p.face.x >= left - 1e-9 && p.face.x <= left + v + 1e-9)
        .map(|p| (p.face.top, p.face.bottom))
        .reduce(|a, b| (a.0.min(b.0), a.1.max(b.1)))
}

fn median(mut v: Vec<f64>) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

/// The people of a shot, most prominent first: faces of the probes grouped by position (small
/// faces in the background and one-off false detections left out).
fn people(probes: &[(f64, Vec<Face>)]) -> Vec<Person> {
    let with: Vec<&Vec<Face>> = probes.iter().map(|p| &p.1).filter(|f| !f.is_empty()).collect();
    if with.is_empty() {
        return vec![];
    }
    let big = median(with.iter().map(|fs| fs.iter().map(|f| f.w).fold(0.0, f64::max)).collect());
    let mut all: Vec<(usize, Face)> =
        with.iter().enumerate().flat_map(|(i, fs)| fs.iter().filter(|f| f.w >= big * 0.55).map(move |f| (i, *f))).collect();
    all.sort_by(|p, q| p.1.x.total_cmp(&q.1.x));
    let gap = (big * 0.8).max(0.04);
    let mut groups: Vec<Vec<(usize, Face)>> = vec![];
    for f in all {
        match groups.last_mut() {
            Some(g) if f.1.x - g.last().unwrap().1.x < gap => g.push(f),
            _ => groups.push(vec![f]),
        }
    }
    let need = if with.len() <= 3 { 1 } else { (with.len() as f64 * 0.3).ceil() as usize };
    let mut ppl: Vec<Person> = groups
        .iter()
        .filter_map(|g| {
            let mut ids: Vec<usize> = g.iter().map(|f| f.0).collect();
            ids.sort_unstable();
            ids.dedup();
            let m = |k: fn(&Face) -> f64| median(g.iter().map(|f| k(&f.1)).collect());
            let face = Face { x: m(|f| f.x), top: m(|f| f.top), bottom: m(|f| f.bottom), w: m(|f| f.w) };
            (ids.len() >= need).then_some(Person { face, seen: ids.len() })
        })
        .collect();
    ppl.sort_by(|p, q| (q.seen as f64 * q.face.w).total_cmp(&(p.seen as f64 * p.face.w)));
    ppl
}

/// Where a person's face was at each probe that found it: the face nearest their usual place.
fn track(probes: &[(f64, Vec<Face>)], p: &Person) -> Vec<(f64, Face)> {
    probes
        .iter()
        .filter_map(|(t, fs)| {
            fs.iter()
                .filter(|f| (f.x - p.face.x).abs() < p.face.w.max(0.03) && f.w > p.face.w * 0.6 && f.w < p.face.w * 1.6)
                .min_by(|a, b| (a.x - p.face.x).abs().total_cmp(&(b.x - p.face.x).abs()))
                .map(|f| (*t, *f))
        })
        .collect()
}

impl Shot {
    /// How much the mouths of two people move, per frame of the shot, from a second, sharper
    /// decode of just this shot. Each mouth patch is matched against the previous frame within a
    /// few pixels, so a moving head doesn't count, only what still differs: the mouth opening and
    /// closing. The same measured around the eyes (blinks, a turning head) is subtracted.
    /// Averaged over ~0.4 s.
    fn mouths(&self, pair: &[Person; 2]) -> Result<[Vec<f64>; 2]> {
        let (w, h, mut child) = spawn_grey(&self.src, self.start, self.end, MOUTH_W)?;
        let mut out = child.stdout.take().unwrap();
        let (wf, hf) = (w as f64, h as f64);
        // (mouth, eyes) boxes in px and the search radius, for a face
        let boxes = |f: Face| {
            let fh = f.bottom - f.top;
            let r = ((f.w * wf * 0.06).round() as i32).clamp(2, 8);
            let px = |x0: f64, y0: f64, x1: f64, y1: f64| {
                let c = |v: f64, max: f64| (v.round() as i32).clamp(r, max as i32 - r - 1);
                [c(x0 * wf, wf), c(y0 * hf, hf), c(x1 * wf, wf), c(y1 * hf, hf)]
            };
            let mouth = px(f.x - f.w * 0.3, f.top + fh * 0.62, f.x + f.w * 0.3, f.bottom + fh * 0.1);
            let eyes = px(f.x - f.w * 0.35, f.top + fh * 0.12, f.x + f.w * 0.35, f.top + fh * 0.45);
            (mouth, eyes, r)
        };
        // each person's face as last detected (the camera may push in or pan during the shot)
        let tracks: Vec<Vec<(f64, Face)>> = pair.iter().map(|p| track(&self.probes, p)).collect();
        let mut frame = vec![0u8; (w * h) as usize];
        let mut prev = frame.clone();
        let mut raw: [Vec<f64>; 2] = [vec![], vec![]];
        let mut k = 0usize;
        while out.read_exact(&mut frame).is_ok() {
            let t = self.start + k as f64 / self.fps;
            for (i, p) in pair.iter().enumerate() {
                let f = tracks[i].iter().rev().find(|(ft, _)| *ft <= t + 1e-6).map(|(_, f)| *f).unwrap_or(p.face);
                let (m, e, r) = boxes(f);
                raw[i].push(if k == 0 {
                    0.0
                } else {
                    (residual(&frame, &prev, w as usize, m, r) - 0.5 * residual(&frame, &prev, w as usize, e, r)).max(0.0)
                });
            }
            k += 1;
            std::mem::swap(&mut frame, &mut prev);
        }
        child.wait()?;
        let r = ((self.fps * 0.2).round() as usize).max(1);
        Ok(raw.map(|raw| {
            (0..raw.len())
                .map(|k| {
                    let s = &raw[k.saturating_sub(r)..(k + r + 1).min(raw.len())];
                    s.iter().sum::<f64>() / s.len() as f64
                })
                .collect()
        }))
    }

    /// Who to show in a two-shot (`pair`: left, right) and from when: (source time, 0 = left /
    /// 1 = right), starting at the shot's start.
    fn follow(&self, pair: &[Person; 2], words: &[Word]) -> Vec<(f64, usize)> {
        let act = match self.mouths(pair) {
            Ok(a) => a,
            Err(e) => {
                eprintln!("warning: reading mouth movement failed ({e}); framing the bigger face");
                [vec![], vec![]]
            }
        };
        let fps = self.fps;
        let n = act[0].len().min(act[1].len());
        let time = |k: usize| self.start + k as f64 / fps;
        if std::env::var_os("FLITS_DEBUG_REFRAME").is_some() {
            // mouth movement left/right, per half second
            let h = ((fps / 2.0) as usize).max(1);
            let m: Vec<String> = (0..n).step_by(h).map(|k| format!("{:.1}/{:.1}", act[0][k], act[1][k])).collect();
            eprintln!("    mouths (left/right per 0.5 s): {}", m.join(" "));
        }
        let words: Vec<&Word> = words.iter().filter(|w| w.end > self.start - 1.0 && w.start < self.end + 1.0).collect();
        let d: Vec<Option<usize>> = match speaker_sides(&words, self.start, self.end, |k| act[0][k] - act[1][k], n, fps) {
            Some(sides) => (0..n)
                .map(|k| {
                    let label = heard(&words, time(k))?;
                    sides.iter().find(|(l, _)| l == label).map(|(_, s)| *s)
                })
                .collect(),
            None => (0..n).map(|k| dominant(act[0][k], act[1][k])).collect(),
        };
        let tl = turns(&d, 1.0 / fps, MIN_TURN);
        match tl.first() {
            // the first speaker is shown from the start of the shot
            Some(_) => tl.iter().enumerate().map(|(j, (k, p))| (if j == 0 { self.start } else { time(*k) }, *p)).collect(),
            None => {
                // nobody talks long enough: whoever moves their mouth most, else the bigger face
                let (s0, s1): (f64, f64) = (act[0].iter().sum(), act[1].iter().sum());
                let p = if s0 > s1 * 1.3 {
                    0
                } else if s1 > s0 * 1.3 {
                    1
                } else {
                    usize::from(pair[1].face.w * pair[1].seen as f64 > pair[0].face.w * pair[0].seen as f64)
                };
                vec![(self.start, p)]
            }
        }
    }
}

/// Which diarization label is which person in a two-shot (0 = left, 1 = right): the side whose
/// mouth moves more while the label is heard. `diff(k)` = left minus right mouth activity of
/// frame `k`. `None` without labels in `[a, b)` (or none that can be told apart).
fn speaker_sides(words: &[&Word], a: f64, b: f64, diff: impl Fn(usize) -> f64, n: usize, fps: f64) -> Option<Vec<(String, usize)>> {
    let mut score: Vec<(String, f64, usize)> = vec![];
    for w in words.iter().filter(|w| w.end > a && w.start < b) {
        let Some(l) = &w.speaker else { continue };
        let (k0, k1) = (((w.start - a) * fps).max(0.0) as usize, (((w.end - a) * fps) as usize).min(n));
        let i = match score.iter().position(|s| &s.0 == l) {
            Some(i) => i,
            None => {
                score.push((l.clone(), 0.0, 0));
                score.len() - 1
            }
        };
        for k in k0..k1 {
            score[i].1 += diff(k);
            score[i].2 += 1;
        }
    }
    let mut means: Vec<(String, f64)> = score.into_iter().filter(|s| s.2 > 0).map(|s| (s.0, s.1 / s.2 as f64)).collect();
    if means.is_empty() {
        return None;
    }
    means.sort_by(|p, q| q.1.total_cmp(&p.1));
    let sides: Vec<(String, usize)> = if means.len() >= 2 {
        // the most "left" label is left, the most "right" one right; others by their sign
        let last = means.len() - 1;
        means
            .iter()
            .enumerate()
            .map(|(i, (l, m))| {
                (
                    l.clone(),
                    if i == 0 {
                        0
                    } else if i == last {
                        1
                    } else {
                        usize::from(*m < 0.0)
                    },
                )
            })
            .collect()
    } else {
        means.iter().filter(|(_, m)| m.abs() > 0.05).map(|(l, m)| (l.clone(), usize::from(*m < 0.0))).collect()
    };
    (!sides.is_empty()).then_some(sides)
}

/// The speaker label heard at `t`: the word being said, or the speaker of the words on both
/// sides of a short pause (`words` in time order).
fn heard<'a>(words: &[&'a Word], t: f64) -> Option<&'a String> {
    let i = words.partition_point(|w| w.start <= t);
    let prev = words[..i].last()?;
    if prev.end >= t {
        return prev.speaker.as_ref();
    }
    let next = words.get(i)?;
    (next.start - prev.end < 0.5 && next.speaker == prev.speaker).then_some(prev.speaker.as_ref()).flatten()
}

/// Who clearly moves their mouth more (mouth activities of the left and right person), if anyone.
fn dominant(l: f64, r: f64) -> Option<usize> {
    const FLOOR: f64 = 0.3;
    if l > FLOOR && l > r * 1.5 {
        Some(0)
    } else if r > FLOOR && r > l * 1.5 {
        Some(1)
    } else {
        None
    }
}

/// Speaker turns from who clearly talks at each sample (`None`: nobody, or unclear), samples `dt`
/// seconds apart. A switch needs the new person to talk through most of the next `min` seconds
/// while the other hardly does, and after a switch the framing holds for at least `min`: short
/// interjections and back-and-forth don't make the crop ping-pong. Returns (sample, person) for
/// the first person and every switch.
pub fn turns(d: &[Option<usize>], dt: f64, min: f64) -> Vec<(usize, usize)> {
    let win = ((min / dt).round() as usize).max(1);
    let mut out: Vec<(usize, usize)> = vec![];
    let mut i = 0;
    while i + win <= d.len() {
        let Some(p) = d[i] else {
            i += 1;
            continue;
        };
        if out.last().is_some_and(|l| l.1 == p) {
            i += 1;
            continue;
        }
        let w = &d[i..i + win];
        let mine = w.iter().filter(|v| **v == Some(p)).count();
        let other = w.iter().filter(|v| v.is_some_and(|q| q != p)).count();
        if mine * 10 >= win * 6 && other * 10 < win * 2 {
            out.push((i, p));
            i += win;
        } else {
            i += 1;
        }
    }
    out
}

/// How much the box `[x0, y0, x1, y1]` (px) of `cur` differs from `prev`, after shifting it up
/// to `r` px to where it matches best: mean absolute difference per pixel (every other one).
fn residual(cur: &[u8], prev: &[u8], w: usize, b: [i32; 4], r: i32) -> f64 {
    let [x0, y0, x1, y1] = b;
    if x1 <= x0 || y1 <= y0 {
        return 0.0;
    }
    let mut best = u64::MAX;
    for dy in -r..=r {
        for dx in -r..=r {
            let mut s = 0u64;
            for y in (y0..y1).step_by(2) {
                let c = &cur[y as usize * w + x0 as usize..y as usize * w + x1 as usize];
                let o = (y + dy) as usize * w + (x0 + dx) as usize;
                let p = &prev[o..o + c.len()];
                s += c.iter().zip(p).step_by(2).map(|(&a, &b)| a.abs_diff(b) as u64).sum::<u64>();
            }
            best = best.min(s);
        }
    }
    let n = (((y1 - y0 + 1) / 2) * ((x1 - x0 + 1) / 2)).max(1) as f64;
    best as f64 / n
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

/// The faces in one picture, biggest first.
fn faces(det: &mut dyn rustface::Detector, grey: &[u8], w: u32, h: u32) -> Vec<Face> {
    let mut faces = det.detect(&rustface::ImageData::new(grey, w, h));
    faces.sort_by_key(|f| std::cmp::Reverse(f.bbox().width() as u64 * f.bbox().height() as u64));
    let (wf, hf) = (w as f64, h as f64);
    faces
        .iter()
        .map(|f| {
            let b = f.bbox();
            Face {
                x: (b.x() as f64 + b.width() as f64 / 2.0) / wf,
                top: b.y() as f64 / hf,
                bottom: (b.y() as f64 + b.height() as f64) / hf,
                w: b.width() as f64 / wf,
            }
        })
        .collect()
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

/// ffmpeg decoding `[a, b)` of the video as `aw`-wide grey frames on stdout: (width, height, process).
fn spawn_grey(src: &Path, a: f64, b: f64, aw: u32) -> Result<(u32, u32, std::process::Child)> {
    let info = super::probe(src)?;
    let h = ((aw as f64 * info.height as f64 / info.width.max(1) as f64 / 2.0).round() * 2.0) as u32;
    let mut cmd = ffmpeg();
    cmd.args(["-ss", &format!("{a:.3}"), "-t", &format!("{:.3}", (b - a).max(0.04))]).arg("-i").arg(src);
    cmd.args(["-an", "-vf", &format!("scale={aw}:{h}:flags=fast_bilinear,format=gray"), "-f", "rawvideo", "-"]);
    let child = cmd.stdout(Stdio::piped()).stderr(Stdio::null()).spawn().context("starting ffmpeg for reframing")?;
    Ok((aw, h, child))
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

    /// `L`/`R`: that person clearly talks, `.`: nobody; one char per 0.1 s
    fn seq(s: &str) -> Vec<Option<usize>> {
        s.chars()
            .map(|c| match c {
                'L' => Some(0),
                'R' => Some(1),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn turns_switch_on_lasting_turns_only() {
        // left talks, right answers for 2 s, left again
        let d = seq(&format!("{}{}{}", "L".repeat(30), "R".repeat(20), "L".repeat(20)));
        assert_eq!(turns(&d, 0.1, 1.2), vec![(0, 0), (30, 1), (50, 0)]);
        // a 0.8 s interjection doesn't cut away
        let d = seq(&format!("{}{}{}", "L".repeat(30), "R".repeat(8), "L".repeat(20)));
        assert_eq!(turns(&d, 0.1, 1.2), vec![(0, 0)]);
    }

    #[test]
    fn turns_bridge_pauses_and_hold() {
        // a speaker with pauses (".") is still one turn; nobody talking keeps the current person
        let d = seq("..LLL.LLLL.LLL......RRRR.RRRRRRRR..");
        assert_eq!(turns(&d, 0.1, 1.2), vec![(2, 0), (20, 1)]);
        // fast back-and-forth: after a switch the framing holds for the minimum turn
        let d = seq("LLLLLLLLLLLLRRRRRRRRRRRRLLLLLLLLLLLL");
        assert_eq!(turns(&d, 0.1, 1.2), vec![(0, 0), (12, 1), (24, 0)]);
        let d = seq(&"LLLLLRRRRR".repeat(4));
        assert_eq!(turns(&d, 0.1, 1.2), vec![]);
        // a turn starting too close to the end of the shot is ignored
        let d = seq(&format!("{}{}", "L".repeat(20), "R".repeat(10)));
        assert_eq!(turns(&d, 0.1, 1.2), vec![(0, 0)]);
    }

    fn face(x: f64, w: f64) -> Face {
        Face { x, top: 0.2, bottom: 0.2 + w * 1.7, w }
    }

    #[test]
    fn people_are_grouped_by_position() {
        // two people, one false detection, a small face in the background
        let mut probes = vec![];
        for i in 0..10 {
            let mut f = vec![face(0.3 + i as f64 * 0.002, 0.08), face(0.7, 0.075), face(0.5, 0.02)];
            if i == 4 {
                f.push(face(0.9, 0.08));
            }
            probes.push((i as f64 * 0.33, f));
        }
        let ppl = people(&probes);
        assert_eq!(ppl.len(), 2);
        assert!((ppl[0].face.x - 0.31).abs() < 0.01 && ppl[0].seen == 10);
        assert!((ppl[1].face.x - 0.7).abs() < 1e-9);
        // the crop around the left person shows only their face
        let b = band(&ppl, 0.3, 0.316).unwrap();
        assert!((b.0 - 0.2).abs() < 1e-9 && (b.1 - (0.2 + 0.08 * 1.7)).abs() < 1e-9);
        assert_eq!(band(&ppl, 0.5, 1.0).unwrap().0, 0.2);
    }

    fn word(start: f64, end: f64, speaker: &str) -> Word {
        Word { text: "w".into(), start, end, speaker: Some(speaker.into()) }
    }

    #[test]
    fn labels_map_to_the_moving_mouth() {
        // frames at 10 fps; the left mouth moves in 0..2 s, the right one in 2..4 s
        let diff = |k: usize| if k < 20 { 3.0 } else { -2.5 };
        let words = [word(0.1, 1.9, "B"), word(2.1, 3.9, "A")];
        let words: Vec<&Word> = words.iter().collect();
        let sides = speaker_sides(&words, 0.0, 4.0, diff, 40, 10.0).unwrap();
        assert!(sides.contains(&("B".into(), 0)) && sides.contains(&("A".into(), 1)));
        assert!(speaker_sides(&[], 0.0, 4.0, diff, 40, 10.0).is_none());
    }

    #[test]
    fn labels_bridge_pauses_of_one_speaker_only() {
        let words = [word(0.0, 1.0, "A"), word(1.3, 2.0, "A"), word(2.2, 3.0, "B"), word(4.0, 5.0, "B")];
        let words: Vec<&Word> = words.iter().collect();
        let at = |t: f64| heard(&words, t).map(String::as_str);
        assert_eq!(at(0.5), Some("A"));
        assert_eq!(at(1.15), Some("A")); // a short pause inside A's turn
        assert_eq!(at(2.1), None); // between two speakers
        assert_eq!(at(3.5), None); // a long pause
        assert_eq!(at(4.5), Some("B"));
        assert_eq!(at(6.0), None);
    }
}
