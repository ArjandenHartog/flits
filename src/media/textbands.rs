//! Text burned into the picture: subtitles of an earlier edit, lower thirds, name straps, slide
//! text. Found on the grey frames the reframe analysis decodes anyway, so captions can keep clear.
//!
//! Per picture, on a copy scaled to about [`LEVEL_H`] px high, every row is scanned for strokes:
//! a steep rise in brightness and a fall of about the same size a few px later (a bright stroke)
//! or the other way round (a dark one). Typeset text is a horizontal band whose rows are full of
//! such strokes, close together and about equally wide; the band is a line of text when it also
//! - is 2..12% of the picture high and at least three times as wide,
//! - has strokes of strong contrast (burned-in text is made to be read),
//! - stands free: the rows just above and below it have far fewer strokes (textures, foliage
//!   and brickwork go on, a line of text ends),
//! - has edges across as well, inside the band: the bars and curves of letters, which the bars
//!   of a railing, a fence or a radiator lack,
//! - is not one of a block of many lines (a page, a ballot paper, a table on a screen), doesn't
//!   overlap a face and keeps a margin from the edge of the picture, as graphics do.
//!
//! Per shot only text that stays counts ([`stable`]): subtitles change their words but keep their
//! place, lower thirds sit still, while a texture that happens to pass in one picture doesn't.

use serde::{Deserialize, Serialize};

/// A band of text, in fractions of the picture: `top`..`bottom`, `x0`..`x1`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextBand {
    pub top: f64,
    pub bottom: f64,
    pub x0: f64,
    pub x1: f64,
}

/// height (px) the picture is scaled to (in whole steps) before looking for text
const LEVEL_H: f64 = 420.0;
/// brightness step (0..255, over 2 px) that counts as the edge of a stroke
const EDGE: i32 = 48;
/// widest stroke (px at the level, edge to edge)
const MAX_SW: u16 = 8;
/// strokes further apart than this (px) start a new run
const GAP: u16 = 12;
/// a run of text has at least this many strokes
const MIN_RUN: usize = 4;
/// text in this many probes in a row (about 3 s) is there to stay, however long the shot
pub const MIN_RUN_PROBES: usize = 9;

#[derive(Clone, Copy, Debug)]
struct Stroke {
    x: u16,
    w: u16,
    contrast: u8,
}

/// A run of strokes close together in one row.
#[derive(Clone, Copy, Debug)]
struct Run {
    y: u16,
    x0: u16,
    x1: u16,
    /// the run's strokes: `rows[y][i0..i1]`
    i0: u32,
    i1: u32,
}

/// A candidate band (connected runs) with what decides about it, in px of the level.
#[derive(Clone, Debug)]
pub struct Candidate {
    pub y0: usize,
    pub y1: usize,
    pub x0: usize,
    pub x1: usize,
    /// strokes per row (rows with runs) per 100 px of width
    pub density: f64,
    /// how much of the bounding box the runs cover
    pub fill: f64,
    /// median contrast of the strokes
    pub contrast: f64,
    /// share of strokes within 0.5..2x the median width
    pub even: f64,
    /// strokes per row just above and below, relative to inside
    pub outside: f64,
    /// horizontal edges (tops, bottoms, bars of letters) per stroke edge
    pub across: f64,
    /// why it isn't text (`None`: it is)
    pub reject: Option<&'static str>,
}

/// Grey picture `px` (`w` x `h`) scaled down by whole steps to about [`LEVEL_H`] high.
fn level(px: &[u8], w: usize, h: usize) -> (Vec<u8>, usize, usize) {
    let k = ((h as f64 / LEVEL_H).round() as usize).max(1);
    if k == 1 {
        return (px.to_vec(), w, h);
    }
    let (lw, lh) = (w / k, h / k);
    let mut out = vec![0u8; lw * lh];
    let n = (k * k) as u32;
    for y in 0..lh {
        for x in 0..lw {
            let mut s = 0u32;
            for dy in 0..k {
                let row = &px[(y * k + dy) * w + x * k..][..k];
                s += row.iter().map(|&v| v as u32).sum::<u32>();
            }
            out[y * lw + x] = ((s + n / 2) / n) as u8;
        }
    }
    (out, lw, lh)
}

/// The strokes of one row, left to right.
fn row_strokes(row: &[u8], out: &mut Vec<Stroke>) {
    let n = row.len();
    if n < 5 {
        return;
    }
    // edges: the steepest point of every strong step (position, signed size)
    let d = |x: usize| row[x + 2] as i32 - row[x] as i32;
    let mut edges: Vec<(u16, i32)> = vec![];
    for x in 0..n - 2 {
        let v = d(x);
        if v.abs() < EDGE {
            continue;
        }
        let l = if x > 0 { d(x - 1) } else { 0 };
        let r = if x + 3 < n { d(x + 1) } else { 0 };
        if (l.signum() == v.signum() && l.abs() > v.abs()) || (r.signum() == v.signum() && r.abs() >= v.abs()) {
            continue;
        }
        edges.push(((x + 1) as u16, v));
    }
    for e in edges.windows(2) {
        let ((xa, a), (xb, b)) = (e[0], e[1]);
        let w = xb - xa;
        // up and down again (or down and up) about as far: a stroke, not a step
        if a.signum() != b.signum() && w <= MAX_SW && a.abs().min(b.abs()) * 5 >= a.abs().max(b.abs()) * 2 {
            out.push(Stroke { x: (xa + xb) / 2, w, contrast: a.abs().min(b.abs()).min(255) as u8 });
        }
    }
}

fn find(p: &mut [usize], i: usize) -> usize {
    let mut r = i;
    while p[r] != r {
        r = p[r];
    }
    let mut i = i;
    while p[i] != r {
        let n = p[i];
        p[i] = r;
        i = n;
    }
    r
}

/// Every band of text-like strokes in a grey picture, with the measures that decide whether it
/// is text (`reject`), in px of the level the picture is looked at (`(w, h)` returned too).
/// `avoid`: boxes (x0, y0, x1, y1; fractions) where text is not looked for (faces).
pub fn candidates(px: &[u8], w: u32, h: u32, avoid: &[[f64; 4]]) -> (Vec<Candidate>, (usize, usize)) {
    let (g, w, h) = level(px, w as usize, h as usize);
    let mut rows: Vec<Vec<Stroke>> = vec![vec![]; h];
    for (y, r) in rows.iter_mut().enumerate() {
        row_strokes(&g[y * w..(y + 1) * w], r);
    }
    let mut runs: Vec<Run> = vec![];
    // first run of each row (runs are in row order)
    let mut row_start = vec![0usize; h + 1];
    for (y, r) in rows.iter().enumerate() {
        row_start[y] = runs.len();
        let mut i = 0;
        while i < r.len() {
            let mut j = i + 1;
            while j < r.len() && r[j].x - r[j - 1].x <= GAP {
                j += 1;
            }
            if j - i >= MIN_RUN {
                let (a, b) = (r[i], r[j - 1]);
                runs.push(Run { y: y as u16, x0: a.x.saturating_sub(a.w), x1: (b.x + b.w).min(w as u16), i0: i as u32, i1: j as u32 });
            }
            i = j;
        }
    }
    row_start[h] = runs.len();
    // runs in the next two rows that overlap are one band (a row without a run, e.g. through the
    // bars of an E, doesn't split a line)
    let mut parent: Vec<usize> = (0..runs.len()).collect();
    for y in 0..h {
        for i in row_start[y]..row_start[y + 1] {
            for j in row_start[(y + 1).min(h)]..row_start[(y + 3).min(h)] {
                if runs[j].x0 <= runs[i].x1 + 1 && runs[i].x0 <= runs[j].x1 + 1 {
                    let (a, b) = (find(&mut parent, i), find(&mut parent, j));
                    parent[a] = b;
                }
            }
        }
    }
    let mut groups: std::collections::BTreeMap<usize, Vec<usize>> = Default::default();
    for i in 0..runs.len() {
        groups.entry(find(&mut parent, i)).or_default().push(i);
    }
    let strokes_in = |y: usize, x0: usize, x1: usize| rows[y].iter().filter(move |s| (s.x as usize) >= x0 && (s.x as usize) <= x1);
    let (wf, hf) = (w as f64, h as f64);
    let mut out: Vec<Candidate> = vec![];
    for ids in groups.values() {
        let rs: Vec<&Run> = ids.iter().map(|&i| &runs[i]).collect();
        let y0 = rs.iter().map(|r| r.y as usize).min().unwrap();
        let y1 = rs.iter().map(|r| r.y as usize).max().unwrap();
        let x0 = rs.iter().map(|r| r.x0 as usize).min().unwrap();
        let x1 = rs.iter().map(|r| r.x1 as usize).max().unwrap();
        let (bh, bw) = (y1 - y0 + 1, (x1 - x0).max(1));
        // tiny specks aren't worth measuring
        if bh < 3 || bw < 12 {
            continue;
        }
        let mut with = vec![false; bh];
        let mut len = 0usize;
        let mut widths: Vec<u16> = vec![];
        let mut contrasts: Vec<u8> = vec![];
        for r in &rs {
            with[r.y as usize - y0] = true;
            len += (r.x1 - r.x0) as usize;
            for s in &rows[r.y as usize][r.i0 as usize..r.i1 as usize] {
                widths.push(s.w);
                contrasts.push(s.contrast);
            }
        }
        let rows_with = with.iter().filter(|v| **v).count().max(1);
        let n = widths.len();
        widths.sort_unstable();
        contrasts.sort_unstable();
        let med_w = widths[n / 2] as f64;
        let even = widths.iter().filter(|&&v| v as f64 >= med_w * 0.5 && v as f64 <= med_w * 2.0).count() as f64 / n as f64;
        let inside = n as f64 / rows_with as f64;
        let m = (bh / 3).max(2);
        let mut around = 0usize;
        let mut around_rows = 0usize;
        for y in y0.saturating_sub(m)..y0 {
            around += strokes_in(y, x0, x1).count();
            around_rows += 1;
        }
        for y in (y1 + 1)..(y1 + 1 + m).min(h) {
            around += strokes_in(y, x0, x1).count();
            around_rows += 1;
        }
        let outside = if around_rows == 0 { 0.0 } else { around as f64 / around_rows as f64 / inside };
        // inside the band only: the rails along the top and bottom of a railing have edges
        // across too
        let (iy0, iy1) = ((y0 + bh / 5).max(1), (y1 - bh / 5).min(h - 2));
        let mut flat = 0usize;
        let mut inner = 0usize;
        for y in iy0..=iy1 {
            inner += strokes_in(y, x0, x1).count();
            for x in x0..x1.min(w) {
                if (g[(y + 1) * w + x] as i32 - g[(y - 1) * w + x] as i32).abs() >= EDGE {
                    flat += 1;
                }
            }
        }
        let across = flat as f64 / (2 * inner).max(1) as f64;
        let edge = x0 as f64 <= wf * 0.03 || x1 as f64 >= wf * 0.97 || y0 as f64 <= hf * 0.03 || (y1 + 1) as f64 >= hf * 0.97;
        let mut c = Candidate {
            y0,
            y1,
            x0,
            x1,
            density: inside / bw as f64 * 100.0,
            fill: len as f64 / (bw * bh) as f64,
            contrast: contrasts[n / 2] as f64,
            even,
            outside,
            across,
            reject: None,
        };
        let on_face =
            avoid.iter().any(|b| x0 as f64 / wf < b[2] && x1 as f64 / wf > b[0] && y0 as f64 / hf < b[3] && (y1 + 1) as f64 / hf > b[1]);
        c.reject = if (bh as f64) < (hf * 0.022).max(6.0) {
            Some("too low")
        } else if bh as f64 > hf * 0.12 {
            Some("too high")
        } else if bw < bh * 3 {
            Some("too narrow")
        } else if (rows_with as f64) < bh as f64 * 0.75 || c.fill < 0.45 {
            Some("gappy")
        } else if c.density < 6.0 {
            Some("sparse")
        } else if c.contrast < 70.0 {
            Some("faint")
        } else if c.even < 0.75 {
            Some("uneven strokes")
        } else if c.outside > 0.3 {
            Some("not free-standing")
        } else if c.across < 0.3 {
            Some("straight bars")
        } else if on_face {
            Some("on a face")
        } else if edge {
            // captions, straps and titles keep a margin from the edge of the picture
            Some("at the edge")
        } else {
            None
        };
        out.push(c);
    }
    // a block of many lines is a page, a table or a ballot paper in the picture, not a caption:
    // the text-like bands of about the same size (whatever their verdict) side by side and
    // stacked closely make one block; four rows or more and none of it counts
    let size = |c: &Candidate| (c.y1 - c.y0 + 1) as f64;
    let mid = |c: &Candidate| (c.y0 + c.y1) as f64 / 2.0;
    let like: Vec<usize> = (0..out.len()).filter(|&i| !matches!(out[i].reject, Some("too high") | Some("faint"))).collect();
    let mut parent: Vec<usize> = (0..out.len()).collect();
    for (k, &i) in like.iter().enumerate() {
        for &j in &like[k + 1..] {
            let (a, b) = (&out[i], &out[j]);
            let (lo, hi) = (size(a).min(size(b)), size(a).max(size(b)));
            if lo < hi * 0.5 {
                continue;
            }
            let dy = (mid(a) - mid(b)).abs();
            let gap = (a.x0.max(b.x0) as f64 - a.x1.min(b.x1) as f64).max(0.0);
            let side = dy < lo * 0.5 && gap < hi * 3.0;
            let stacked = a.x0 < b.x1 && b.x0 < a.x1 && dy <= hi * 4.0;
            if side || stacked {
                let (ra, rb) = (find(&mut parent, i), find(&mut parent, j));
                parent[ra] = rb;
            }
        }
    }
    let mut blocks: std::collections::BTreeMap<usize, Vec<usize>> = Default::default();
    for &i in &like {
        blocks.entry(find(&mut parent, i)).or_default().push(i);
    }
    for ids in blocks.values() {
        let mut mids: Vec<(f64, f64)> = ids.iter().map(|&i| (mid(&out[i]), size(&out[i]))).collect();
        mids.sort_by(|p, q| p.0.total_cmp(&q.0));
        let mut rows = 1;
        let mut last = mids[0].0;
        for &(m, sz) in &mids[1..] {
            if m - last > sz * 0.6 {
                rows += 1;
                last = m;
            }
        }
        if rows >= 4 {
            for &i in ids {
                if out[i].reject.is_none() {
                    out[i].reject = Some("one of many lines");
                }
            }
        }
    }
    (out, (w, h))
}

/// The lines of text in a grey picture (`w` x `h`), in fractions of the picture; `avoid`: boxes
/// (x0, y0, x1, y1; fractions) to leave out (faces).
pub fn lines(px: &[u8], w: u32, h: u32, avoid: &[[f64; 4]]) -> Vec<TextBand> {
    let (cs, (lw, lh)) = candidates(px, w, h, avoid);
    let (wf, hf) = (lw as f64, lh as f64);
    let mut found: Vec<TextBand> = cs
        .iter()
        .filter(|c| c.reject.is_none())
        .map(|c| TextBand { top: c.y0 as f64 / hf, bottom: (c.y1 + 1) as f64 / hf, x0: c.x0 as f64 / wf, x1: c.x1 as f64 / wf })
        .collect();
    found.sort_by(|a, b| a.x0.total_cmp(&b.x0));
    // the words of one line (side by side, about level) are one band
    let mut out: Vec<TextBand> = vec![];
    for b in found {
        let level = |a: &TextBand| {
            let (ha, hb) = (a.bottom - a.top, b.bottom - b.top);
            let over = a.bottom.min(b.bottom) - a.top.max(b.top);
            over >= 0.5 * ha.min(hb) && (b.x0 - a.x1) * wf <= 3.0 * ha.max(hb) * hf
        };
        match out.iter_mut().find(|a| level(a)) {
            Some(a) => *a = TextBand { top: a.top.min(b.top), bottom: a.bottom.max(b.bottom), x0: a.x0.min(b.x0), x1: a.x1.max(b.x1) },
            None => out.push(b),
        }
    }
    out
}

fn quantile(mut v: Vec<f64>, q: f64) -> f64 {
    v.sort_by(f64::total_cmp);
    v[((v.len() - 1) as f64 * q).round() as usize]
}

/// The text bands of a shot from the lines found in each of its probes (taken at even steps).
/// Lines that overlap or follow each other closely (the lines of a subtitle, a name and a title)
/// make one zone; a zone counts when it has text in at least half of the probes (and two), or
/// in [`MIN_RUN_PROBES`] probes in a row: subtitles come and go and change their words but keep
/// to their zone, a lower third stays a few seconds. Its extent leaves out the odd outlier.
pub fn stable(probes: &[Vec<TextBand>]) -> Vec<TextBand> {
    let n = probes.len();
    let need = n.div_ceil(2).max(2);
    if n < need {
        return vec![];
    }
    let mut all: Vec<(usize, TextBand)> = probes.iter().enumerate().flat_map(|(i, ls)| ls.iter().map(move |l| (i, *l))).collect();
    all.sort_by(|a, b| a.1.top.total_cmp(&b.1.top));
    let mut zones: Vec<(f64, Vec<(usize, TextBand)>)> = vec![];
    for l in all {
        match zones.last_mut() {
            Some((bottom, z)) if l.1.top <= *bottom + 0.5 * (l.1.bottom - l.1.top) => {
                *bottom = bottom.max(l.1.bottom);
                z.push(l);
            }
            _ => zones.push((l.1.bottom, vec![l])),
        }
    }
    zones
        .iter()
        .filter(|(_, z)| {
            let mut seen: Vec<usize> = z.iter().map(|m| m.0).collect();
            seen.sort_unstable();
            seen.dedup();
            // or on screen for a good while in one go (a lower third in a long shot), missing
            // from one probe at a time at most
            let mut run = 1;
            let mut longest = 1;
            for p in seen.windows(2) {
                run = if p[1] - p[0] <= 2 { run + p[1] - p[0] } else { 1 };
                longest = longest.max(run);
            }
            seen.len() >= need || longest >= MIN_RUN_PROBES
        })
        .map(|(_, z)| {
            let q = |f: fn(&TextBand) -> f64, q: f64| quantile(z.iter().map(|m| f(&m.1)).collect(), q);
            TextBand { top: q(|b| b.top, 0.05), bottom: q(|b| b.bottom, 0.95), x0: q(|b| b.x0, 0.1), x1: q(|b| b.x1, 0.9) }
        })
        .collect()
}

/// Vertical extent (top, bottom; fractions) of the text a crop around `x` showing `visible` of
/// the width keeps in the picture: bands it shows a fair part of.
pub fn visible_band(bands: &[TextBand], x: f64, visible: f64) -> Option<(f64, f64)> {
    let v = visible.min(1.0);
    let left = (x - v / 2.0).clamp(0.0, 1.0 - v);
    bands
        .iter()
        .filter(|b| {
            let shown = b.x1.min(left + v) - b.x0.max(left);
            shown > 0.0 && (shown >= 0.4 * (b.x1 - b.x0) || shown >= 0.25 * v)
        })
        .map(|b| (b.top, b.bottom))
        .reduce(|a, b| (a.0.min(b.0), a.1.max(b.1)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::color::Rgba;
    use crate::text::{layout, render, tokenize, Align, FontDb, TextStyle};

    const W: usize = 768;
    const H: usize = 432;

    /// A deterministic pseudo-random number 0..1 for (x, y, seed).
    fn noise(x: usize, y: usize, seed: u64) -> f64 {
        let mut v = (x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (y as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F) ^ seed;
        v ^= v >> 29;
        v = v.wrapping_mul(0xBF58_476D_1CE4_E5B9);
        v ^= v >> 32;
        (v % 10_000) as f64 / 10_000.0
    }

    /// A calm scene: a soft gradient with a little sensor noise.
    fn scene(w: usize, h: usize) -> Vec<u8> {
        (0..w * h)
            .map(|i| (60.0 + 50.0 * (i % w) as f64 / w as f64 + 40.0 * (i / w) as f64 / h as f64 + 6.0 * noise(i % w, i / w, 1)) as u8)
            .collect()
    }

    /// Draw `text` (grey `fill`, optional `outline`, optional `box_grey` behind it) centred on
    /// (`cx`, `cy`) with flits' own text renderer.
    #[allow(clippy::too_many_arguments)]
    fn draw(px: &mut [u8], w: usize, text: &str, size: f32, (cx, cy): (f64, f64), fill: u8, outline: Option<u8>, box_grey: Option<u8>) {
        let grey = |v: u8| Rgba::rgba8(v, v, v, 1.0);
        let style = TextStyle {
            font: FontDb::new(Default::default()).get(None, None).unwrap(),
            size,
            color: grey(fill),
            line_height: 1.2,
            letter_spacing: 0.0,
            word_spacing: 0.0,
            align: Align::Left,
            max_width: None,
            stroke: outline.map(|o| ((size * 0.09).max(1.5), grey(o))),
            balance: false,
            shadow: None,
            bg: box_grey.map(|b| (grey(b), size * 0.4, size * 0.2, 0.0, false)),
        };
        let lay = layout(&style, &tokenize(text));
        let sprite = render(&style, &lay, None, None).unwrap();
        let (sw, sh) = (sprite.pm.width() as usize, sprite.pm.height() as usize);
        let (ox, oy) = ((cx - sw as f64 / 2.0) as usize, (cy - sh as f64 / 2.0) as usize);
        for (i, p) in sprite.pm.pixels().iter().enumerate() {
            let (x, y) = (ox + i % sw, oy + i / sw);
            if x < w && y * w + x < px.len() {
                let bg = px[y * w + x] as u32;
                px[y * w + x] = (p.red() as u32 + bg * (255 - p.alpha() as u32) / 255).min(255) as u8;
            }
        }
    }

    fn found(px: &[u8], w: usize, h: usize) -> Vec<TextBand> {
        lines(px, w as u32, h as u32, &[])
    }

    #[test]
    fn finds_subtitles_and_lower_thirds() {
        // white subtitles with a black outline near the bottom
        let mut px = scene(W, H);
        draw(&mut px, W, "and that is why we went there", 28.0, (W as f64 * 0.5, H as f64 * 0.85), 255, Some(0), None);
        let l = found(&px, W, H);
        assert_eq!(l.len(), 1, "{l:?}");
        assert!(l[0].top > 0.78 && l[0].top < 0.85 && l[0].bottom > 0.85 && l[0].bottom < 0.92, "{l:?}");
        assert!(l[0].x0 > 0.15 && l[0].x1 < 0.85);
        // a lower third: dark text on a light strap, at the left
        let mut px = scene(W, H);
        draw(&mut px, W, "Jane Doe, chair of the board", 24.0, (W as f64 * 0.3, H as f64 * 0.8), 20, None, Some(235));
        let l = found(&px, W, H);
        assert_eq!(l.len(), 1, "{l:?}");
        assert!(l[0].x1 < 0.6 && (l[0].top + l[0].bottom) / 2.0 > 0.75);
        // a tall (phone) picture is looked at smaller: reel captions on a 9:16 picture
        let (w, h) = (768, 1366);
        let mut px = scene(w, h);
        draw(&mut px, w, "STUDY ADVICE", 65.0, (w as f64 * 0.5, h as f64 * 0.72), 255, Some(0), None);
        let l = found(&px, w, h);
        assert_eq!(l.len(), 1, "{l:?}");
        assert!(l[0].top < 0.72 && l[0].bottom > 0.72);
    }

    #[test]
    fn ignores_textures_bars_and_pages() {
        // foliage / gravel: busy, high-contrast specks everywhere
        let px: Vec<u8> = (0..W * H).map(|i| if noise(i % W / 2, i / W / 2, 7) > 0.5 { 210 } else { 40 }).collect();
        assert!(found(&px, W, H).is_empty());
        // finer specks, blotchy: leaves in the wind
        let px: Vec<u8> = (0..W * H).map(|i| (noise(i % W / 3, i / W / 3, 9) * 255.0) as u8).collect();
        assert!(found(&px, W, H).is_empty());
        // a railing: thin bars between a hand rail and a base, short and long
        for (top, len) in [(200, 14), (150, 60)] {
            let mut px = scene(W, H);
            for y in top..top + len {
                for x in 100..600 {
                    if x % 11 < 3 {
                        px[y * W + x] = 20;
                    }
                }
            }
            for x in 95..605 {
                for y in [top - 3, top - 2, top - 1, top + len, top + len + 1] {
                    px[y * W + x] = 20;
                }
            }
            assert!(found(&px, W, H).is_empty(), "railing {len} px");
        }
        // a building: rows of windows
        let mut px = scene(W, H);
        for y in 60..380 {
            for x in 50..700 {
                if x % 18 < 9 && y % 24 < 14 {
                    px[y * W + x] = 30;
                }
            }
        }
        assert!(found(&px, W, H).is_empty());
        // stairs: light and dark steps
        let px: Vec<u8> = (0..W * H).map(|i| if (i / W) % 12 < 3 { 200 } else { 30 }).collect();
        assert!(found(&px, W, H).is_empty());
        // a page of text (a ballot paper, a table on a screen): many lines, not a caption
        let mut px = scene(W, H);
        for y in 60..400 {
            for x in 150..620 {
                px[y * W + x] = 235;
            }
        }
        for i in 0..9 {
            draw(&mut px, W, "Smith, J. (John) Rotterdam", 15.0, (385.0, 85.0 + i as f64 * 34.0), 20, None, None);
        }
        assert!(found(&px, W, H).is_empty(), "{:?}", found(&px, W, H));
    }

    #[test]
    fn text_on_a_face_is_left_out() {
        let mut px = scene(W, H);
        draw(&mut px, W, "and that is why we went there", 28.0, (W as f64 * 0.5, H as f64 * 0.5), 255, Some(0), None);
        assert_eq!(found(&px, W, H).len(), 1);
        assert!(lines(&px, W as u32, H as u32, &[[0.4, 0.35, 0.6, 0.65]]).is_empty());
    }

    fn band(top: f64, x0: f64, x1: f64) -> TextBand {
        TextBand { top, bottom: top + 0.04, x0, x1 }
    }

    #[test]
    fn only_text_that_stays_counts() {
        // subtitles in 6 of 10 probes, changing words (width) and now and then a second line;
        // a one-off false find elsewhere
        let mut probes: Vec<Vec<TextBand>> = vec![vec![]; 10];
        for (i, p) in probes.iter_mut().enumerate().filter(|(i, _)| i % 5 < 3) {
            p.push(band(0.8, 0.3 - i as f64 * 0.01, 0.7 + i as f64 * 0.01));
            if i == 5 {
                p.push(band(0.75, 0.35, 0.65));
            }
        }
        probes[3].push(band(0.3, 0.1, 0.4));
        let s = stable(&probes);
        assert_eq!(s.len(), 1, "{s:?}");
        assert!((s[0].top - 0.75).abs() < 1e-9 && (s[0].bottom - 0.84).abs() < 1e-9, "{s:?}");
        // text in a third of the probes, scattered: not there to stay
        let probes: Vec<Vec<TextBand>> = (0..30).map(|i| if i % 3 == 0 { vec![band(0.8, 0.3, 0.7)] } else { vec![] }).collect();
        assert!(stable(&probes).is_empty());
        // a lower third for a few seconds of a long shot
        let probes: Vec<Vec<TextBand>> =
            (0..60).map(|i| if (20..32).contains(&i) { vec![band(0.82, 0.05, 0.5)] } else { vec![] }).collect();
        assert_eq!(stable(&probes).len(), 1);
        // a single probe proves nothing
        assert!(stable(&[vec![band(0.8, 0.3, 0.7)]]).is_empty());
    }

    #[test]
    fn crops_keep_the_text_they_show() {
        let t = [TextBand { top: 0.82, bottom: 0.9, x0: 0.05, x1: 0.5 }];
        // the whole picture, or a crop on the left: there
        assert_eq!(visible_band(&t, 0.5, 1.0), Some((0.82, 0.9)));
        assert_eq!(visible_band(&t, 0.25, 0.316), Some((0.82, 0.9)));
        // a 9:16 crop on the right: cut away
        assert_eq!(visible_band(&t, 0.75, 0.316), None);
        // just the tail of it at the crop's edge doesn't count
        assert_eq!(visible_band(&t, 0.62, 0.316), None);
    }

    /// Tuning aid: `FLITS_TEXT_TUNE=<dir of grey pngs> cargo test --profile fast tune -- --ignored --nocapture`
    /// prints the candidates of every picture and writes `<name>.bands.png` with the text found.
    #[test]
    #[ignore]
    fn tune() {
        let Some(dir) = std::env::var_os("FLITS_TEXT_TUNE") else { return };
        let mut files: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.path()).collect();
        files.sort();
        for f in files.iter().filter(|f| f.extension().is_some_and(|e| e == "png") && !f.to_string_lossy().contains(".bands")) {
            let img = image::open(f).unwrap().to_luma8();
            let (w, h) = img.dimensions();
            let (cs, (lw, lh)) = candidates(img.as_raw(), w, h, &[]);
            let name = f.file_name().unwrap().to_string_lossy().to_string();
            let mut out = image::RgbImage::from_fn(lw as u32, lh as u32, |x, y| {
                let k = (h as usize / lh).max(1);
                let v = img.get_pixel((x as usize * k) as u32, (y as usize * k) as u32)[0];
                image::Rgb([v, v, v])
            });
            for c in &cs {
                let interesting = !matches!(c.reject, Some("too low") | Some("too narrow") | Some("gappy") | Some("too high"));
                if !interesting {
                    continue;
                }
                println!(
                    "{name}: y {}-{} x {}-{} dens {:.1} fill {:.2} con {:.0} even {:.2} out {:.2} across {:.2} -> {}",
                    c.y0,
                    c.y1,
                    c.x0,
                    c.x1,
                    c.density,
                    c.fill,
                    c.contrast,
                    c.even,
                    c.outside,
                    c.across,
                    c.reject.unwrap_or("TEXT")
                );
                let col = if c.reject.is_none() { image::Rgb([255, 0, 0]) } else { image::Rgb([0, 120, 255]) };
                for x in c.x0..c.x1.min(lw - 1) {
                    out.put_pixel(x as u32, c.y0 as u32, col);
                    out.put_pixel(x as u32, c.y1 as u32, col);
                }
                for y in c.y0..=c.y1 {
                    out.put_pixel(c.x0 as u32, y as u32, col);
                    out.put_pixel(c.x1.min(lw - 1) as u32, y as u32, col);
                }
            }
            out.save(f.with_extension("bands.png")).unwrap();
        }
    }
}
