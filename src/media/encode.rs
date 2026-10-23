//! Video encoders (NVENC, QSV, VAAPI, x264) fed with raw YUV frames.

use super::ffmpeg;
use anyhow::{bail, Context, Result};
use std::path::Path;
use std::process::{Child, Stdio};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Encoder {
    Nvenc,
    Qsv,
    Vaapi,
    X264,
}

impl Encoder {
    pub fn name(self) -> &'static str {
        match self {
            Encoder::Nvenc => "h264_nvenc (GPU)",
            Encoder::Qsv => "h264_qsv (Intel GPU)",
            Encoder::Vaapi => "h264_vaapi (GPU)",
            Encoder::X264 => "libx264 (CPU)",
        }
    }
}

pub fn vaapi_device() -> String {
    std::env::var("FLITS_VAAPI_DEVICE").unwrap_or_else(|_| "/dev/dri/renderD128".into())
}

fn encoder_works(codec: &str) -> bool {
    let mut c = ffmpeg();
    if codec == "h264_vaapi" {
        c.args(["-vaapi_device", &vaapi_device()]);
    }
    c.args(["-f", "lavfi", "-i", "color=c=black:s=256x256:d=0.2"]);
    if codec == "h264_vaapi" {
        c.args(["-vf", "format=nv12,hwupload"]);
    }
    c.args(["-c:v", codec, "-f", "null", "-"]).stdout(Stdio::null()).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false)
}

pub fn pick_encoder(choice: &str) -> Result<Encoder> {
    Ok(match choice {
        "nvenc" | "gpu" => Encoder::Nvenc,
        "qsv" => Encoder::Qsv,
        "vaapi" => Encoder::Vaapi,
        "x264" | "cpu" => Encoder::X264,
        "auto" => {
            if encoder_works("h264_nvenc") {
                Encoder::Nvenc
            } else if cfg!(target_os = "linux") && encoder_works("h264_vaapi") {
                Encoder::Vaapi
            } else if encoder_works("h264_qsv") {
                Encoder::Qsv
            } else {
                Encoder::X264
            }
        }
        o => bail!("unknown encoder '{o}' (auto, nvenc, qsv, vaapi, x264)"),
    })
}

/// ffmpeg process that reads raw yuv420p frames on stdin and writes an H.264 file.
/// `in_w` may be wider than `w` (padded GPU frames); the extra columns are cropped.
pub fn spawn_encoder(enc: Encoder, in_w: u32, w: u32, h: u32, fps: f64, quality: u32, out: &Path) -> Result<Child> {
    let mut cmd = ffmpeg();
    if enc == Encoder::Vaapi {
        cmd.args(["-vaapi_device", &vaapi_device()]);
    }
    cmd.args(["-y", "-f", "rawvideo", "-pix_fmt", "yuv420p", "-s", &format!("{in_w}x{h}"), "-r", &format!("{fps}"), "-i", "-"]);
    let crop = if in_w != w { format!("crop={w}:{h}:0:0,") } else { String::new() };
    if !crop.is_empty() && enc != Encoder::Vaapi {
        cmd.args(["-vf", crop.trim_end_matches(',')]);
    }
    match enc {
        Encoder::Nvenc => {
            cmd.args([
                "-c:v",
                "h264_nvenc",
                "-preset",
                "p4",
                "-tune",
                "hq",
                "-rc",
                "vbr",
                "-cq",
                &quality.to_string(),
                "-b:v",
                "0",
                "-profile:v",
                "high",
            ]);
        }
        Encoder::Qsv => {
            cmd.args(["-c:v", "h264_qsv", "-preset", "fast", "-global_quality", &quality.to_string()]);
        }
        Encoder::Vaapi => {
            cmd.args(["-vf", &format!("{crop}format=nv12,hwupload"), "-c:v", "h264_vaapi", "-qp", &quality.to_string()]);
        }
        Encoder::X264 => {
            cmd.args(["-c:v", "libx264", "-preset", "veryfast", "-crf", &quality.to_string(), "-threads", "4"]);
        }
    }
    if enc != Encoder::Vaapi {
        cmd.args(["-pix_fmt", "yuv420p"]);
    }
    cmd.args(["-color_primaries", "bt709", "-color_trc", "bt709", "-colorspace", "bt709", "-color_range", "tv"]);
    cmd.arg(out);
    cmd.stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped());
    cmd.spawn().context("starting ffmpeg encoder")
}
