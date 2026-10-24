//! Decode only what is shown: a cover-fitted 16:9 video in a 9:16 frame shows a third of each
//! frame, yet decoding, piping and uploading the whole frame was most of the work. Video layers
//! whose framing doesn't move (`zoom`, `panX`, `panY` constant) get the part of the source they
//! show cut out in the decoder, before scaling.

use super::{Fit, Kind, Node};
use crate::media::video::CropPlan;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

/// Cropping that keeps less than this share of the frame's area isn't worth a separate decoder.
const MIN_SAVING: f64 = 0.15;

/// The part of a video layer's picture its decoder delivers.
#[derive(Clone, Debug)]
pub struct Crop {
    /// rect of the displayed source, in source pixels
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    /// shared by the layers whose frames can come from one decoder (sizes and positions over time)
    pub plan: Arc<CropPlan>,
}

/// Scale and offset that draw a `iw`x`ih` picture into a `w`x`h` box: picture pixel `p` lands at
/// `p * s + o`. The single definition of `fit`, `zoom` and `pan` (drawing uses it too).
pub fn place(fit: Fit, (w, h): (f32, f32), (iw, ih): (f32, f32), zoom: f32, pan: (f32, f32)) -> ((f32, f32), (f32, f32)) {
    let (sx, sy) = match fit {
        Fit::Cover => {
            let s = (w / iw).max(h / ih);
            (s, s)
        }
        Fit::Contain => {
            let s = (w / iw).min(h / ih);
            (s, s)
        }
        Fit::Fill => (w / iw, h / ih),
    };
    let (sx, sy) = (sx * zoom, sy * zoom);
    let ox = (w - iw * sx) / 2.0 * (1.0 + pan.0);
    let oy = (h - ih * sy) / 2.0 * (1.0 + pan.1);
    ((sx, sy), (ox, oy))
}

/// The part of the source (`x0, y0, x1, y1` in source pixels, unrounded) that shows in the box,
/// `None` when none of it does.
pub fn visible(fit: Fit, bx: (f64, f64), src: (f64, f64), zoom: f64, pan: (f64, f64)) -> Option<[f64; 4]> {
    let ((sx, sy), (ox, oy)) =
        place(fit, (bx.0 as f32, bx.1 as f32), (src.0 as f32, src.1 as f32), zoom as f32, (pan.0 as f32, pan.1 as f32));
    let (sx, sy, ox, oy) = (sx as f64, sy as f64, ox as f64, oy as f64);
    let x0 = ((0.0 - ox) / sx).max(0.0);
    let x1 = ((bx.0 - ox) / sx).min(src.0);
    let y0 = ((0.0 - oy) / sy).max(0.0);
    let y1 = ((bx.1 - oy) / sy).min(src.1);
    (x1 > x0 && y1 > y0).then_some([x0, y0, x1, y1])
}

/// Grow a source rect by `margin` px on each side (sampling at the edge of the shown part reads
/// the pixels just outside it), onto even pixels (4:2:0 chroma), within the `iw`x`ih` source.
pub fn pixel_rect(r: [f64; 4], margin: f64, (iw, ih): (u32, u32)) -> (u32, u32, u32, u32) {
    let lo = |v: f64| (((v - margin).floor().max(0.0) as u32) / 2) * 2;
    let hi = |v: f64, max: u32| ((((v + margin).ceil().max(0.0) as u32).div_ceil(2)) * 2).min(max / 2 * 2);
    let (x0, y0) = (lo(r[0]), lo(r[1]));
    let (x1, y1) = (hi(r[2], iw), hi(r[3], ih));
    (x0, y0, x1.saturating_sub(x0).max(2), y1.saturating_sub(y0).max(2))
}

/// Move a rect of size `w`x`h` (at least the size of `need`) so it covers `need`, centred on it,
/// inside the `iw`x`ih` source, on even pixels.
fn fit_rect(need: (u32, u32, u32, u32), (w, h): (u32, u32), (iw, ih): (u32, u32)) -> (u32, u32) {
    let place = |start: u32, len: u32, size: u32, max: u32| {
        let c = start as i64 - (size as i64 - len as i64) / 2;
        (c.clamp(0, (max as i64 - size as i64).max(0)) as u32) / 2 * 2
    };
    (place(need.0, need.2, w, iw), place(need.1, need.3, h, ih))
}

/// What a video layer needs decoded, worked out while the layer is built.
#[derive(Clone, Debug)]
pub struct Need {
    /// source rect with margin, in source pixels
    pub rect: (u32, u32, u32, u32),
    /// scale from source pixels to decoded pixels
    pub scale: f64,
}

/// Source pixels a layer needs decoded, `None` when its framing moves or it shows (nearly) all of it.
pub fn need(fit: Fit, bx: (f64, f64), src: (u32, u32), zoom: Option<f64>, pan: (Option<f64>, Option<f64>), scale: f64) -> Option<Need> {
    let (zoom, px, py) = (zoom?, pan.0?, pan.1?);
    let r = visible(fit, bx, (src.0 as f64, src.1 as f64), zoom, (px, py))?;
    // the decoder's scaler reads a couple of its output pixels around each one
    let margin = (2.0 / scale.max(1e-3)).ceil() + 2.0;
    let rect = pixel_rect(r, margin, src);
    let saving = 1.0 - (rect.2 as f64 * rect.3 as f64) / (src.0 as f64 * src.1 as f64);
    (saving >= MIN_SAVING).then_some(Need { rect, scale })
}

/// Decoded size of `len` source pixels at `scale` (even, as 4:2:0 needs).
pub fn decoded(len: u32, scale: f64) -> u32 {
    ((len as f64 * scale / 2.0).ceil() as u32 * 2).max(2)
}

/// One video layer that wants cropping, as seen by [`plan`].
struct Item {
    id: usize,
    rect: (u32, u32, u32, u32),
    /// source times of the first and last frame the layer shows
    from: f64,
    to: f64,
}

/// Give every video layer that asked for it a crop. Layers playing the same file at the same
/// scale and speed share one crop size, and as long as they show different moments of the file,
/// one plan: a decoder that runs from one into the next (the shots of a reel, cuts of one take)
/// moves its crop at the right frame and is never restarted for a new framing.
pub fn plan(nodes: &mut [Node], fps: f64) {
    // group by what makes frames interchangeable: file, decode scale, speed
    type Key = (PathBuf, u64, u64);
    let mut groups: HashMap<Key, Vec<Item>> = HashMap::new();
    let mut src_size: HashMap<Key, (u32, u32)> = HashMap::new();
    walk(nodes, &mut |n| {
        if let Kind::Video { src, info, trim, looped, speed, need: Some(need), .. } = &n.kind {
            let key = (src.clone(), need.scale.to_bits(), speed.to_bits());
            let (from, to) = if *looped { (f64::NEG_INFINITY, f64::INFINITY) } else { shown(n.start, n.end, *trim, *speed, fps) };
            groups.entry(key.clone()).or_default().push(Item { id: n.id, rect: need.rect, from, to });
            src_size.insert(key, (info.width, info.height));
        }
    });
    let mut crops: HashMap<usize, Crop> = HashMap::new();
    for (key, mut items) in groups {
        let size = src_size[&key];
        let scale = f64::from_bits(key.1);
        // one crop size for the group: the largest any of its layers needs
        let w = items.iter().map(|i| i.rect.2).max().unwrap_or(2);
        let h = items.iter().map(|i| i.rect.3).max().unwrap_or(2);
        if (w as f64 * h as f64) > (1.0 - MIN_SAVING) * size.0 as f64 * size.1 as f64 {
            continue; // too little left to gain: these layers decode whole frames
        }
        // layers that show overlapping moments can't share a decoder: split them over plans
        items.sort_by(|a, b| a.from.total_cmp(&b.from));
        let mut lanes: Vec<Vec<&Item>> = Vec::new();
        for it in &items {
            match lanes.iter_mut().find(|l| l.last().is_some_and(|p| p.to < it.from)) {
                Some(l) => l.push(it),
                None => lanes.push(vec![it]),
            }
        }
        if std::env::var_os("FLITS_DEBUG").is_some() {
            for (k, l) in lanes.iter().enumerate() {
                for it in l {
                    eprintln!("flits: crop plan {k}: layer {} shows {:.3}..{:.3}", it.id, it.from, it.to);
                }
            }
        }
        for lane in lanes {
            let at: Vec<(u32, u32)> = lane.iter().map(|it| fit_rect(it.rect, (w, h), size)).collect();
            let plan = Arc::new(CropPlan {
                w,
                h,
                dw: decoded(w, scale),
                dh: decoded(h, scale),
                steps: lane.iter().zip(&at).map(|(it, &(x, y))| (it.from, x, y)).collect(),
            });
            for (it, (x, y)) in lane.iter().zip(at) {
                crops.insert(it.id, Crop { x, y, w, h, plan: plan.clone() });
            }
        }
    }
    walk_mut(nodes, &mut |n| {
        if let Kind::Video { crop, .. } = &mut n.kind {
            *crop = crops.remove(&n.id);
        }
    });
}

/// Take the crops off again: whole frames are decoded.
pub fn clear(nodes: &mut [Node]) {
    walk_mut(nodes, &mut |n| {
        if let Kind::Video { crop, .. } = &mut n.kind {
            *crop = None;
        }
    });
}

/// Source times of the first and last frame a video layer from `start` to `end` shows, the same
/// way drawing picks them: frame `f` (at `f / fps`) shows layer-local frame `f - round(start * fps)`.
pub fn shown(start: f64, end: f64, trim: f64, speed: f64, fps: f64) -> (f64, f64) {
    let s0 = (start * fps).round() as i64;
    let f = first_frame(start, fps);
    let mut l = (end * fps).ceil() as i64 + 1;
    while l > f && (l as f64 / fps) >= end {
        l -= 1;
    }
    let t = |frame: i64| trim + (frame - s0).max(0) as f64 * speed / fps;
    (t(f), t(l.max(f)))
}

/// The first frame at or after `start` (as drawing decides: frame `f` is at `f / fps`).
pub fn first_frame(start: f64, fps: f64) -> i64 {
    let mut f = (start * fps).floor() as i64 - 1;
    while (f as f64 / fps) < start {
        f += 1;
    }
    f
}

fn walk(nodes: &[Node], f: &mut impl FnMut(&Node)) {
    for n in nodes {
        f(n);
        walk(&n.children, f);
    }
}

fn walk_mut(nodes: &mut [Node], f: &mut impl FnMut(&mut Node)) {
    for n in nodes {
        f(n);
        walk_mut(&mut n.children, f);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_vertical_crop_of_wide_footage() {
        // 1920x1080 cover-fitted into 1080x1920: a 607.5 px wide slice, centred without pan
        let r = visible(Fit::Cover, (1080.0, 1920.0), (1920.0, 1080.0), 1.0, (0.0, 0.0)).unwrap();
        assert!((r[0] - 656.25).abs() < 1e-3 && (r[2] - 1263.75).abs() < 1e-3, "{r:?}");
        assert!(r[1].abs() < 1e-3 && (r[3] - 1080.0).abs() < 1e-3, "{r:?}");
        // panned fully left / right
        let l = visible(Fit::Cover, (1080.0, 1920.0), (1920.0, 1080.0), 1.0, (-1.0, 0.0)).unwrap();
        assert!(l[0].abs() < 1e-3 && (l[2] - 607.5).abs() < 1e-3, "{l:?}");
        let r = visible(Fit::Cover, (1080.0, 1920.0), (1920.0, 1080.0), 1.0, (1.0, 0.0)).unwrap();
        assert!((r[0] - 1312.5).abs() < 1e-3 && (r[2] - 1920.0).abs() < 1e-3, "{r:?}");
        // zoomed in 1.1x: a smaller part, in both directions
        let z = visible(Fit::Cover, (1080.0, 1920.0), (1920.0, 1080.0), 1.1, (0.0, 0.0)).unwrap();
        assert!((z[2] - z[0] - 607.5 / 1.1).abs() < 1e-2 && (z[3] - z[1] - 1080.0 / 1.1).abs() < 1e-2, "{z:?}");
    }

    #[test]
    fn whole_pictures_are_not_cropped() {
        // contain shows everything; cover of the same aspect too
        assert!(need(Fit::Contain, (1080.0, 1920.0), (1920, 1080), Some(1.0), (Some(0.0), Some(0.0)), 1.0).is_none());
        assert!(need(Fit::Cover, (1920.0, 1080.0), (3840, 2160), Some(1.0), (Some(0.0), Some(0.0)), 0.5).is_none());
        // a moving framing can't be cropped up front
        assert!(need(Fit::Cover, (1080.0, 1920.0), (1920, 1080), None, (Some(0.0), Some(0.0)), 1.0).is_none());
        let n = need(Fit::Cover, (1080.0, 1920.0), (1920, 1080), Some(1.0), (Some(0.0), Some(0.0)), 1.0).unwrap();
        // 656.25..1263.75 plus a 4 px margin, on even pixels
        assert_eq!(n.rect, (652, 0, 616, 1080));
    }

    #[test]
    fn rects_stay_even_and_inside() {
        assert_eq!(pixel_rect([0.5, 0.0, 99.5, 50.0], 2.0, (100, 50)), (0, 0, 100, 50));
        assert_eq!(pixel_rect([10.3, 3.0, 20.2, 7.0], 1.0, (101, 51)), (8, 2, 14, 6));
        // a 4K source shown at 0.5x: the margin covers the scaler's reach in source pixels
        let n = need(Fit::Cover, (1080.0, 1920.0), (3840, 2160), Some(1.0), (Some(0.0), Some(0.0)), 0.889).unwrap();
        let (x, _, w, _) = n.rect;
        assert!(x % 2 == 0 && w % 2 == 0 && x + w <= 3840);
        assert!(x <= 1312 - 3 && x + w >= 2527 + 3, "{:?}", n.rect);
    }

    #[test]
    fn group_rects_cover_each_need() {
        let size = (1920, 1080);
        // a smaller (punched-in) need centred inside the group's larger crop
        let (x, y) = fit_rect((700, 50, 560, 980), (616, 1080), size);
        assert!(x <= 700 && x + 616 >= 1260 && y == 0, "{x} {y}");
        // pushed back inside at the edges
        assert_eq!(fit_rect((1880, 0, 40, 1080), (616, 1080), size), (1304, 0));
        assert_eq!(fit_rect((0, 0, 40, 1080), (616, 1080), size), (0, 0));
    }

    #[test]
    fn shown_frames_match_drawing() {
        // 25 fps, a layer from 1.0 s to 2.0 s: frames 25..=49, source trim .. trim + 24/25
        let (a, b) = shown(1.0, 2.0, 10.0, 1.0, 25.0);
        assert!((a - 10.0).abs() < 1e-9 && (b - 10.96).abs() < 1e-9, "{a} {b}");
        // a start between frames: the first frame drawn is the next one, already one frame in
        let (a, _) = shown(1.01, 2.0, 10.0, 1.0, 25.0);
        assert!((a - 10.04).abs() < 1e-9, "{a}");
        // the next layer starts where this one ends: they show different source frames
        let (_, b1) = shown(0.0, 1.0, 5.0, 1.0, 25.0);
        let (a2, _) = shown(1.0, 2.0, 6.0, 1.0, 25.0);
        assert!(a2 > b1);
    }
}
