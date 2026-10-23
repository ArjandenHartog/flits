//! One frame, either backend: the single entry point used by the video pipeline,
//! stills, contact sheets, benchmarks and tests.

use super::profile::{self, Stage};
use super::yuv::rgba_to_yuv420;
use crate::compose::{advance, draw_frame, render_frame, Prepared, VideoState};
use crate::paint::cpu::CpuPainter;
use crate::paint::gpu::{padded_width, Gpu, GpuPainter, GpuTarget, Readback};
use anyhow::{anyhow, Result};
use std::sync::Arc;
use tiny_skia::{IntSize, Pixmap};

pub struct FrameRenderer {
    gpu: Option<(Arc<Gpu>, GpuTarget)>,
    canvas: Option<Pixmap>,
    videos: VideoState,
    width: u32,
    height: u32,
}

impl FrameRenderer {
    pub fn new(p: &Prepared, gpu: Option<Arc<Gpu>>) -> Result<FrameRenderer> {
        let (gpu, canvas) = match gpu {
            Some(g) => {
                let t = g.target(p.width, p.height);
                (Some((g, t)), None)
            }
            None => (None, Some(Pixmap::new(p.width, p.height).ok_or_else(|| anyhow!("bad output size {}x{}", p.width, p.height))?)),
        };
        let mut videos = VideoState::default();
        videos.high_bit = gpu.as_ref().is_some_and(|(g, _)| g.high_bit_video);
        Ok(FrameRenderer { gpu, canvas, videos, width: p.width, height: p.height })
    }

    pub fn is_gpu(&self) -> bool {
        self.gpu.is_some()
    }

    /// Width of the YUV frames produced by [`yuv`](Self::yuv) (GPU frames are padded to 8 px).
    pub fn yuv_width(&self) -> u32 {
        if self.gpu.is_some() {
            padded_width(self.width)
        } else {
            self.width
        }
    }

    /// Close video decoders (e.g. before jumping to a far-away part of the timeline).
    pub fn reset_videos(&mut self) {
        let high_bit = self.videos.high_bit;
        self.videos = VideoState::default();
        self.videos.high_bit = high_bit;
    }

    /// Render `frame` as planar YUV 4:2:0 into `out`.
    pub fn yuv(&mut self, p: &Prepared, frame: i64, out: &mut Vec<u8>) -> Result<()> {
        match (&mut self.gpu, &mut self.canvas) {
            (Some((g, tg)), _) => {
                profile::time(Stage::Decode, || advance(p, frame, &mut self.videos))?;
                let mut gp = GpuPainter::default();
                profile::time(Stage::Compose, || draw_frame(p, frame, &mut gp, &self.videos));
                profile::time(Stage::Gpu, || g.render(tg, p.background, gp, Readback::Yuv420, out))
            }
            (None, Some(c)) => {
                profile::time(Stage::Decode, || advance(p, frame, &mut self.videos))?;
                profile::time(Stage::Cpu, || {
                    c.fill(p.background.skia(1.0));
                    draw_frame(p, frame, &mut CpuPainter { canvas: c }, &self.videos);
                    rgba_to_yuv420(c.data(), self.width as usize, self.height as usize, out);
                });
                Ok(())
            }
            _ => unreachable!(),
        }
    }

    /// Render `frame` as an RGBA bitmap.
    pub fn rgba(&mut self, p: &Prepared, frame: i64) -> Result<Pixmap> {
        match (&mut self.gpu, &mut self.canvas) {
            (Some((g, tg)), _) => {
                advance(p, frame, &mut self.videos)?;
                let mut gp = GpuPainter::default();
                draw_frame(p, frame, &mut gp, &self.videos);
                let mut data = Vec::new();
                g.render(tg, p.background, gp, Readback::Rgba, &mut data)?;
                Pixmap::from_vec(data, IntSize::from_wh(self.width, self.height).unwrap()).ok_or_else(|| anyhow!("bad GPU readback"))
            }
            (None, Some(c)) => {
                render_frame(p, frame, c, &mut self.videos)?;
                Ok(c.clone())
            }
            _ => unreachable!(),
        }
    }
}
