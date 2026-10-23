//! Video encoders (NVENC, QSV, VAAPI, x264/x265/SVT-AV1) fed with raw YUV frames.

use super::ffmpeg;
use anyhow::{bail, Context, Result};
use std::path::Path;
use std::process::{Child, Stdio};

/// Where the encoding happens.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Encoder {
    Nvenc,
    Qsv,
    Vaapi,
    /// software (libx264 / libx265 / libsvtav1)
    Cpu,
    /// discard frames (benchmarking the renderer alone)
    Null,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Codec {
    H264,
    Hevc,
    Av1,
}

/// Encoder speed/quality trade-off.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Speed {
    Fast,
    Balanced,
    Quality,
}

#[derive(Clone, Copy, Debug)]
pub struct Settings {
    pub encoder: Encoder,
    pub codec: Codec,
    pub speed: Speed,
    /// CRF/CQ-like quality, lower = better (18..30)
    pub quality: u32,
}

impl Codec {
    pub fn parse(s: &str) -> Result<Codec> {
        Ok(match s {
            "h264" | "avc" => Codec::H264,
            "hevc" | "h265" => Codec::Hevc,
            "av1" => Codec::Av1,
            o => bail!("unknown codec '{o}' (h264, hevc, av1)"),
        })
    }
}

impl Speed {
    pub fn parse(s: &str) -> Result<Speed> {
        Ok(match s {
            "fast" => Speed::Fast,
            "balanced" | "default" => Speed::Balanced,
            "quality" | "slow" => Speed::Quality,
            o => bail!("unknown speed '{o}' (fast, balanced, quality)"),
        })
    }
}

impl Settings {
    /// ffmpeg codec name.
    pub fn codec_name(&self) -> &'static str {
        use Codec::*;
        use Encoder::*;
        match (self.encoder, self.codec) {
            (Nvenc, H264) => "h264_nvenc",
            (Nvenc, Hevc) => "hevc_nvenc",
            (Nvenc, Av1) => "av1_nvenc",
            (Qsv, H264) => "h264_qsv",
            (Qsv, Hevc) => "hevc_qsv",
            (Qsv, Av1) => "av1_qsv",
            (Vaapi, H264) => "h264_vaapi",
            (Vaapi, Hevc) => "hevc_vaapi",
            (Vaapi, Av1) => "av1_vaapi",
            (Cpu, H264) => "libx264",
            (Cpu, Hevc) => "libx265",
            (Cpu, Av1) => "libsvtav1",
            (Null, _) => "null",
        }
    }

    pub fn describe(&self) -> String {
        let hw = match self.encoder {
            Encoder::Nvenc => "NVIDIA GPU",
            Encoder::Qsv => "Intel GPU",
            Encoder::Vaapi => "VAAPI GPU",
            Encoder::Cpu => "CPU",
            Encoder::Null => "discarded",
        };
        format!("{} ({hw})", self.codec_name())
    }

    /// Encoders that share the GPU with rendering, or are single engines, gain nothing from many sessions.
    pub fn max_sessions(&self) -> usize {
        match self.encoder {
            Encoder::Nvenc => 8,
            Encoder::Qsv | Encoder::Vaapi => 4,
            Encoder::Cpu | Encoder::Null => usize::MAX,
        }
    }
}

pub fn vaapi_device() -> String {
    std::env::var("FLITS_VAAPI_DEVICE").unwrap_or_else(|_| "/dev/dri/renderD128".into())
}

pub fn encoder_works(codec: &str) -> bool {
    let mut c = ffmpeg();
    let vaapi = codec.ends_with("_vaapi");
    if vaapi {
        c.args(["-vaapi_device", &vaapi_device()]);
    }
    c.args(["-f", "lavfi", "-i", "color=c=black:s=256x256:d=0.2"]);
    if vaapi {
        c.args(["-vf", "format=nv12,hwupload"]);
    }
    c.args(["-c:v", codec, "-f", "null", "-"]).stdout(Stdio::null()).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false)
}

/// Resolve "auto" | "nvenc" | "qsv" | "vaapi" | "cpu"/"x264" | "null" for the given codec.
pub fn pick(choice: &str, codec: Codec, speed: Speed, quality: u32) -> Result<Settings> {
    let mk = |encoder| Settings { encoder, codec, speed, quality };
    let explicit = match choice {
        "nvenc" | "gpu" => Some(Encoder::Nvenc),
        "qsv" => Some(Encoder::Qsv),
        "vaapi" => Some(Encoder::Vaapi),
        "cpu" | "x264" | "x265" | "software" => Some(Encoder::Cpu),
        "null" => Some(Encoder::Null),
        "auto" => None,
        o => bail!("unknown encoder '{o}' (auto, nvenc, qsv, vaapi, cpu, null)"),
    };
    if let Some(e) = explicit {
        return Ok(mk(e));
    }
    let mut order = vec![Encoder::Nvenc];
    if cfg!(target_os = "linux") {
        order.push(Encoder::Vaapi);
    }
    order.push(Encoder::Qsv);
    for e in order {
        if encoder_works(mk(e).codec_name()) {
            return Ok(mk(e));
        }
    }
    Ok(mk(Encoder::Cpu))
}

/// ffmpeg process that reads raw yuv420p frames on stdin and writes a video file.
/// `in_w` may be wider than `w` (padded GPU frames); the extra columns are cropped.
pub fn spawn_encoder(s: &Settings, in_w: u32, w: u32, h: u32, fps: f64, out: &Path) -> Result<Child> {
    let mut cmd = ffmpeg();
    if s.encoder == Encoder::Vaapi {
        cmd.args(["-vaapi_device", &vaapi_device()]);
    }
    cmd.args(["-y", "-f", "rawvideo", "-pix_fmt", "yuv420p", "-s", &format!("{in_w}x{h}"), "-r", &format!("{fps}"), "-i", "-"]);
    if s.encoder == Encoder::Null {
        cmd.args(["-f", "null", "-"]);
        cmd.stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped());
        return cmd.spawn().context("starting ffmpeg");
    }
    let mut vf = Vec::new();
    if in_w != w {
        vf.push(format!("crop={w}:{h}:0:0"));
    }
    if s.encoder == Encoder::Vaapi {
        vf.push("format=nv12,hwupload".into());
    }
    if !vf.is_empty() {
        cmd.args(["-vf", &vf.join(",")]);
    }
    let q = s.quality.to_string();
    let codec = s.codec_name();
    cmd.args(["-c:v", codec]);
    match s.encoder {
        Encoder::Nvenc => {
            let preset = match s.speed {
                Speed::Fast => "p2", // as fast as p1 here, slightly better quality
                Speed::Balanced => "p4",
                Speed::Quality => "p6",
            };
            cmd.args(["-preset", preset, "-tune", "hq", "-rc", "vbr", "-cq", &q, "-b:v", "0"]);
            if s.codec == Codec::H264 {
                cmd.args(["-profile:v", "high"]);
            }
        }
        Encoder::Qsv => {
            let preset = match s.speed {
                Speed::Fast => "veryfast",
                Speed::Balanced => "fast",
                Speed::Quality => "slow",
            };
            cmd.args(["-preset", preset, "-global_quality", &q]);
        }
        Encoder::Vaapi => {
            cmd.args(["-qp", &q]);
        }
        Encoder::Cpu => {
            match s.codec {
                Codec::Av1 => {
                    let preset = match s.speed {
                        Speed::Fast => "10",
                        Speed::Balanced => "8",
                        Speed::Quality => "5",
                    };
                    cmd.args(["-preset", preset, "-crf", &(s.quality + 8).to_string()]);
                }
                _ => {
                    let preset = match s.speed {
                        Speed::Fast => "ultrafast",
                        Speed::Balanced => "veryfast",
                        Speed::Quality => "medium",
                    };
                    cmd.args(["-preset", preset, "-crf", &q, "-threads", "4"]);
                }
            }
            if s.codec == Codec::Hevc {
                cmd.args(["-x265-params", "log-level=error"]);
            }
        }
        Encoder::Null => unreachable!(),
    }
    if s.codec == Codec::Hevc {
        cmd.args(["-tag:v", "hvc1"]); // plays in QuickTime/iOS
    }
    if s.encoder != Encoder::Vaapi {
        cmd.args(["-pix_fmt", "yuv420p"]);
    }
    cmd.args(["-color_primaries", "bt709", "-color_trc", "bt709", "-colorspace", "bt709", "-color_range", "tv"]);
    cmd.arg(out);
    cmd.stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped());
    cmd.spawn().context("starting ffmpeg encoder")
}
