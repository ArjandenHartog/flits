//! Streaming decoded video frames out of ffmpeg.

use super::{ffmpeg, MediaInfo};
use anyhow::{Context, Result};
use std::io::{BufReader, Read};
use std::path::Path;
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::OnceLock;

static HAS_ZSCALE: OnceLock<bool> = OnceLock::new();

fn has_zscale() -> bool {
    *HAS_ZSCALE.get_or_init(|| {
        Command::new("ffmpeg")
            .args(["-hide_banner", "-filters"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).contains(" zscale "))
            .unwrap_or(false)
    })
}

/// Streams decoded RGBA frames of a video, already scaled to the size it is drawn at.
pub struct VideoReader {
    child: Child,
    out: BufReader<ChildStdout>,
    pub frame: Vec<u8>,
    pub w: u32,
    pub h: u32,
    /// layer-local frame index of the frame currently in `frame` (-1 = none yet)
    pub cur: i64,
    first: i64,
    eof: bool,
}

impl VideoReader {
    /// Start decoding so that the first frame read corresponds to layer-local frame `first`.
    pub fn spawn(src: &Path, info: &MediaInfo, trim: f64, first: i64, fps: f64, w: u32, h: u32, looped: bool) -> Result<VideoReader> {
        let mut seek = trim + first as f64 / fps;
        if looped && info.duration > 0.0 {
            seek %= info.duration;
        }
        // resample + scale first so the (expensive) tone mapping runs at the final, smaller size
        let mut vf = format!("fps={fps},scale={w}:{h}:flags=bicubic");
        if info.hdr && has_zscale() {
            // HLG/PQ phone footage -> SDR, otherwise it looks washed out
            vf.push_str(",zscale=t=linear:npl=203,format=gbrpf32le,zscale=p=bt709,tonemap=hable:desat=0,zscale=t=bt709:m=bt709:r=tv");
        }
        vf.push_str(",format=rgba");
        let mut cmd = ffmpeg();
        cmd.args(["-threads", "2"]);
        if looped {
            cmd.args(["-stream_loop", "-1"]);
        }
        cmd.args(["-ss", &format!("{seek:.4}")]).arg("-i").arg(src);
        cmd.args(["-an", "-sn", "-vf", &vf, "-f", "rawvideo", "-"]);
        cmd.stdout(Stdio::piped()).stderr(Stdio::null()).stdin(Stdio::null());
        let mut child = cmd.spawn().context("starting ffmpeg decoder")?;
        let out = BufReader::with_capacity(1 << 20, child.stdout.take().unwrap());
        Ok(VideoReader { child, out, frame: vec![0; (w * h * 4) as usize], w, h, cur: first - 1, first, eof: false })
    }

    pub fn first(&self) -> i64 {
        self.first
    }

    /// Advance to layer-local frame `idx` (must be >= current). Holds the last frame at EOF.
    pub fn seek_forward(&mut self, idx: i64) -> Result<()> {
        while self.cur < idx && !self.eof {
            match self.out.read_exact(&mut self.frame) {
                Ok(()) => {
                    // ffmpeg's rgba is straight alpha; video is opaque so force alpha
                    self.cur += 1;
                }
                Err(_) => {
                    self.eof = true;
                    if self.cur < self.first {
                        // nothing decoded at all: leave black
                        self.frame.fill(0);
                    }
                }
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
