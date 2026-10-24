//! Stills and contact sheets for quick visual checks.

use super::FrameRenderer;
use crate::compose::Prepared;
use crate::paint::gpu::Gpu;
use crate::scene::color::Rgba;
use crate::text::{self, Align, FontDb, TextStyle};
use anyhow::{Context, Result};
use rayon::prelude::*;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tiny_skia::{Pixmap, PixmapPaint, Transform};

/// Render single frames at the given times to PNG.
pub fn stills(p: &Prepared, gpu: Option<Arc<Gpu>>, times: &[f64], out: &Path) -> Result<Vec<PathBuf>> {
    let paths: Vec<PathBuf> = if times.len() == 1 {
        vec![out.to_path_buf()]
    } else {
        let stem = out.file_stem().and_then(|s| s.to_str()).unwrap_or("still");
        let dir = out.parent().unwrap_or(Path::new("."));
        times.iter().map(|t| dir.join(format!("{stem}-{t:.2}s.png"))).collect()
    };
    times
        .par_iter()
        .zip(paths.par_iter())
        .map(|(t, path)| -> Result<()> {
            let f = ((t * p.fps).round() as i64).clamp(0, (p.frames - 1).max(0));
            let canvas = FrameRenderer::new(p, gpu.clone())?.rgba(p, f)?;
            canvas.save_png(path).with_context(|| format!("writing {}", path.display()))?;
            Ok(())
        })
        .collect::<Result<()>>()?;
    Ok(paths)
}

/// Which frames a contact sheet shows: the frames at `at` (seconds) when given; else `count` frames
/// from `from` to `to` (both included) when either is given; else `count` evenly spaced over the video
/// (the middle of each of `count` equal parts).
pub fn sheet_frames(p: &Prepared, count: usize, at: &[f64], from: Option<f64>, to: Option<f64>) -> Vec<i64> {
    let last = (p.frames - 1).max(0);
    let frame = |t: f64| ((t * p.fps).round() as i64).clamp(0, last);
    let count = count.max(1);
    if !at.is_empty() {
        return at.iter().map(|&t| frame(t)).collect();
    }
    if from.is_none() && to.is_none() {
        return (0..count).map(|i| ((i as f64 + 0.5) / count as f64 * p.frames as f64) as i64).collect();
    }
    let (a, b) = (from.unwrap_or(0.0), to.unwrap_or(p.duration).min(last as f64 / p.fps));
    if count == 1 {
        return vec![frame(a)];
    }
    (0..count).map(|i| frame(a + (b - a) * i as f64 / (count - 1) as f64)).collect()
}

/// Contact sheet: the given frames in a grid, each labelled with its time in seconds and frame number.
pub fn sheet(p: &Prepared, gpu: Option<Arc<Gpu>>, frames: &[i64], cols: usize, out: &Path) -> Result<()> {
    let count = frames.len().max(1);
    let cols = cols.clamp(1, count);
    let rows = count.div_ceil(cols);
    let (tw, th) = (p.width, p.height);
    let label_h = 34u32;
    let gap = 8u32;
    let sw = cols as u32 * (tw + gap) + gap;
    let sh = rows as u32 * (th + label_h + gap) + gap;
    let tiles: Vec<Pixmap> =
        frames.par_iter().map(|&f| -> Result<Pixmap> { FrameRenderer::new(p, gpu.clone())?.rgba(p, f) }).collect::<Result<_>>()?;
    let mut sheet = Pixmap::new(sw, sh).unwrap();
    sheet.fill(Rgba::rgba8(24, 24, 27, 1.0).skia(1.0));
    let fonts = FontDb::new(Default::default());
    let style = TextStyle {
        font: fonts.get(None, None)?,
        size: 22.0,
        color: Rgba::rgba8(230, 230, 235, 1.0),
        line_height: 1.2,
        letter_spacing: 0.0,
        word_spacing: 0.0,
        align: Align::Left,
        max_width: None,
        stroke: None,
        balance: false,
        shadow: None,
        bg: None,
    };
    for (i, tile) in tiles.iter().enumerate() {
        let (c, r) = ((i % cols) as u32, (i / cols) as u32);
        let x = gap + c * (tw + gap);
        let y = gap + r * (th + label_h + gap);
        sheet.draw_pixmap(x as i32, (y + label_h) as i32, tile.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
        let t = frames[i] as f64 / p.fps;
        let label = format!("{t:.2}s  (frame {})", frames[i]);
        let lay = text::layout(&style, &[label]);
        let spr = text::render(&style, &lay, None, None)?;
        sheet.draw_pixmap(
            0,
            0,
            spr.pm.as_ref(),
            &PixmapPaint::default(),
            spr.place(Transform::from_translate(x as f32, y as f32 + 2.0)),
            None,
        );
    }
    sheet.save_png(out).with_context(|| format!("writing {}", out.display()))?;
    Ok(())
}
