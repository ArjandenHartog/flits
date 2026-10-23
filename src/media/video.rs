//! Streaming decoded video frames out of ffmpeg as planar YUV, decoded ahead on a
//! background thread. Colour conversion and HDR tone mapping happen on the GPU.

use super::colorspace::FrameFormat;
use super::{ffmpeg, MediaInfo};
use anyhow::{Context, Result};
use std::io::Read;
use std::path::Path;
use std::process::{Child, Stdio};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};

/// Frames decoded ahead of the renderer.
const AHEAD: usize = 6;

/// Streams decoded YUV 4:2:0 frames of a video, already scaled to the size it is drawn at.
pub struct VideoReader {
    child: Child,
    frames: Receiver<Vec<u8>>,
    recycle: SyncSender<Vec<u8>>,
    /// the current frame: Y plane, U plane, V plane (see `fmt`)
    pub frame: Vec<u8>,
    pub fmt: FrameFormat,
    pub w: u32,
    pub h: u32,
    /// layer-local frame index of the frame currently in `frame` (first - 1 = none yet)
    pub cur: i64,
    first: i64,
    eof: bool,
}

impl VideoReader {
    /// Start decoding so that the first frame read corresponds to layer-local frame `first`.
    /// `high_bit`: deliver 10-bit samples for 10-bit/HDR sources (the consumer supports 16-bit planes).
    #[allow(clippy::too_many_arguments)]
    pub fn spawn(
        src: &Path,
        info: &MediaInfo,
        trim: f64,
        first: i64,
        fps: f64,
        w: u32,
        h: u32,
        looped: bool,
        speed: f64,
        high_bit: bool,
    ) -> Result<VideoReader> {
        let mut seek = trim + first as f64 / fps * speed;
        if looped && info.duration > 0.0 {
            seek %= info.duration;
        }
        let mut fmt = info.color;
        fmt.high_bit = high_bit && (fmt.high_bit || fmt.is_hdr());
        let pix = if fmt.high_bit { "yuv420p10le" } else { "yuv420p" };
        // keep the source colour encoding: only retime, resample + scale here, the GPU converts
        let retime = if (speed - 1.0).abs() > 1e-6 { format!("setpts=PTS/{speed},") } else { String::new() };
        let vf = format!("{retime}fps={fps},scale={w}:{h}:flags=bicubic,format={pix}");
        let mut cmd = ffmpeg();
        cmd.args(["-threads", "2", "-filter_threads", "1"]);
        if looped {
            cmd.args(["-stream_loop", "-1"]);
        }
        cmd.args(["-ss", &format!("{seek:.4}")]).arg("-i").arg(src);
        cmd.args(["-an", "-sn", "-vf", &vf, "-f", "rawvideo", "-"]);
        cmd.stdout(Stdio::piped()).stderr(Stdio::null()).stdin(Stdio::null());
        let mut child = cmd.spawn().context("starting ffmpeg decoder")?;
        let mut out = child.stdout.take().unwrap();

        let size = fmt.frame_size(w, h);
        let (tx, frames) = sync_channel::<Vec<u8>>(AHEAD);
        let (recycle, free) = sync_channel::<Vec<u8>>(AHEAD + 2);
        std::thread::Builder::new()
            .name("flits-video".into())
            .spawn(move || loop {
                let mut buf = free.try_recv().unwrap_or_else(|_| vec![0; size]);
                if out.read_exact(&mut buf).is_err() || tx.send(buf).is_err() {
                    return; // EOF, or the reader was dropped
                }
            })
            .context("starting decoder thread")?;
        let mut frame = vec![0; size];
        // start as black (Y=16, U=V=128 in 8-bit; zeros are fine for 16-bit before the first frame)
        if !fmt.high_bit {
            frame[..(w * h) as usize].fill(16);
            frame[(w * h) as usize..].fill(128);
        }
        Ok(VideoReader { child, frames, recycle, frame, fmt, w, h, cur: first - 1, first, eof: false })
    }

    pub fn first(&self) -> i64 {
        self.first
    }

    /// Advance to layer-local frame `idx` (must be >= current). Holds the last frame at EOF.
    pub fn seek_forward(&mut self, idx: i64) -> Result<()> {
        while self.cur < idx && !self.eof {
            match self.frames.recv() {
                Ok(buf) => {
                    let old = std::mem::replace(&mut self.frame, buf);
                    let _ = self.recycle.try_send(old);
                    self.cur += 1;
                }
                Err(_) => self.eof = true, // keep showing the last frame (black if none)
            }
        }
        Ok(())
    }
}

impl Drop for VideoReader {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
