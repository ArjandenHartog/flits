//! Still images: decode once, fix orientation, downscale to the size actually shown, pre-blur.

use crate::gfx::blur;
use crate::media;
use crate::media::colorspace::{adjust_rgba8, Adjust};
use anyhow::{anyhow, bail, Context, Result};
use fast_image_resize::{images::Image as FrImage, FilterType, PixelType, ResizeAlg, ResizeOptions, Resizer};
use image::{DynamicImage, ImageDecoder, ImageReader};
use std::path::Path;
use tiny_skia::{IntSize, Pixmap};

/// Natural (orientation-corrected) size of an image without fully decoding it.
pub fn image_size(path: &Path) -> Result<(u32, u32)> {
    if let Ok(r) = ImageReader::open(path).and_then(|r| r.with_guessed_format()) {
        if let Ok(mut dec) = r.into_decoder() {
            let (w, h) = dec.dimensions();
            let rotated = matches!(
                dec.orientation().unwrap_or(image::metadata::Orientation::NoTransforms),
                image::metadata::Orientation::Rotate90
                    | image::metadata::Orientation::Rotate270
                    | image::metadata::Orientation::Rotate90FlipH
                    | image::metadata::Orientation::Rotate270FlipH
            );
            return Ok(if rotated { (h, w) } else { (w, h) });
        }
    }
    // formats the image crate can't read (HEIC, AVIF, ...) go through ffprobe
    let info = media::probe(path)?;
    Ok((info.width, info.height))
}

fn decode(path: &Path) -> Result<(u32, u32, Vec<u8>)> {
    let attempt = || -> Result<DynamicImage> {
        let mut dec = ImageReader::open(path)?.with_guessed_format()?.into_decoder()?;
        let orient = dec.orientation().unwrap_or(image::metadata::Orientation::NoTransforms);
        let mut img = DynamicImage::from_decoder(dec)?;
        img.apply_orientation(orient);
        Ok(img)
    };
    match attempt() {
        Ok(img) => {
            let rgba = img.into_rgba8();
            let (w, h) = rgba.dimensions();
            Ok((w, h, rgba.into_raw()))
        }
        Err(e) => {
            // fall back to ffmpeg for anything else
            let info = media::probe(path).map_err(|_| e)?;
            let data = decode_with_ffmpeg(path, info.width, info.height)?;
            Ok((info.width, info.height, data))
        }
    }
}

fn premultiply(data: &mut [u8]) {
    for px in data.as_chunks_mut::<4>().0 {
        let a = px[3] as u16;
        if a < 255 {
            px[0] = ((px[0] as u16 * a + 127) / 255) as u8;
            px[1] = ((px[1] as u16 * a + 127) / 255) as u8;
            px[2] = ((px[2] as u16 * a + 127) / 255) as u8;
        }
    }
}

pub fn resize_rgba(w: u32, h: u32, data: Vec<u8>, nw: u32, nh: u32) -> Result<Vec<u8>> {
    if nw == w && nh == h {
        return Ok(data);
    }
    let src = FrImage::from_vec_u8(w, h, data, PixelType::U8x4)?;
    let mut dst = FrImage::new(nw, nh, PixelType::U8x4);
    let mut resizer = Resizer::new();
    let alg = if nw < w / 2 { FilterType::Lanczos3 } else { FilterType::CatmullRom };
    resizer.resize(&src, &mut dst, &ResizeOptions::new().resize_alg(ResizeAlg::Convolution(alg)))?;
    Ok(dst.into_vec())
}

/// Load an image so its largest on-screen size is (max_w, max_h); blur in px at that size.
pub fn load_image(path: &Path, max_w: f64, max_h: f64, blur_sigma: f64, adj: &Adjust) -> Result<Pixmap> {
    let (w, h, mut data) = decode(path).with_context(|| format!("loading image {}", path.display()))?;
    adjust_rgba8(&mut data, adj);
    premultiply(&mut data);
    let mut f = (max_w / w as f64).max(max_h / h as f64).min(1.0);
    // blurred images can live at a much lower resolution
    let down = if blur_sigma > 0.0 { (blur_sigma / 4.0).clamp(1.0, 12.0) } else { 1.0 };
    f /= down;
    let nw = ((w as f64 * f).round() as u32).max(1);
    let nh = ((h as f64 * f).round() as u32).max(1);
    let data = resize_rgba(w, h, data, nw, nh)?;
    let mut pm = Pixmap::from_vec(data, IntSize::from_wh(nw, nh).ok_or_else(|| anyhow!("bad image size"))?)
        .ok_or_else(|| anyhow!("bad image buffer"))?;
    if blur_sigma > 0.0 {
        blur(&mut pm, (blur_sigma / down) as f32);
    }
    Ok(pm)
}

fn decode_with_ffmpeg(path: &Path, w: u32, h: u32) -> Result<Vec<u8>> {
    let out = media::ffmpeg().arg("-i").arg(path).args(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgba", "-"]).output()?;
    if !out.status.success() || out.stdout.len() < (w * h * 4) as usize {
        bail!("ffmpeg could not decode {}", path.display());
    }
    Ok(out.stdout[..(w * h * 4) as usize].to_vec())
}
