//! Speech-to-text via faster-whisper (run through `uv`, so no manual Python setup).

use crate::compose::captions::{load_transcript, Word};
use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

const SCRIPT: &str = include_str!("whisper.py");

fn has(cmd: &str) -> bool {
    Command::new(cmd).arg("--version").output().map(|o| o.status.success()).unwrap_or(false)
}

fn has_nvidia() -> bool {
    Command::new("nvidia-smi").arg("-L").output().map(|o| o.status.success()).unwrap_or(false)
}

pub struct TranscribeOpts {
    pub model: String,
    pub language: String,
    pub device: String,
    pub python: Option<PathBuf>,
    /// remote server URL, "off", or None = use the configured one if reachable
    pub remote: Option<String>,
}

/// Transcribe `input`; returns the path of the JSON transcript (also writes .srt/.vtt/.txt).
/// Uses a configured `flits serve` GPU machine when reachable, otherwise runs locally.
pub fn transcribe(input: &Path, out_json: &Path, o: &TranscribeOpts) -> Result<PathBuf> {
    if !input.exists() {
        bail!("file not found: {}", input.display());
    }
    let remote = match o.remote.as_deref() {
        Some("off") => None,
        Some(url) => Some(crate::remote::Remote {
            url: url.trim_end_matches('/').to_string(),
            token: std::env::var("FLITS_TOKEN").ok().or_else(|| crate::remote::configured().map(|r| r.token)).unwrap_or_default(),
        }),
        None => crate::remote::configured(),
    };
    if let Some(r) = remote {
        match r.health() {
            Ok(h) => {
                eprintln!("flits: using GPU server {} ({})", r.url, h["gpu"].as_str().unwrap_or("no GPU"));
                match r.transcribe(input, out_json, &o.language, &o.model) {
                    Ok(()) => {
                        let words = load_transcript(out_json)?;
                        write_subtitles(out_json, &words)?;
                        return Ok(out_json.to_path_buf());
                    }
                    Err(e) => eprintln!("flits: remote transcription failed ({e:#}); transcribing locally"),
                }
            }
            Err(e) => eprintln!("flits: GPU server {} not reachable ({e}); transcribing locally", r.url),
        }
    }
    transcribe_local(input, out_json, o)
}

fn transcribe_local(input: &Path, out_json: &Path, o: &TranscribeOpts) -> Result<PathBuf> {
    let dir = std::env::temp_dir().join("flits");
    std::fs::create_dir_all(&dir)?;
    let script = dir.join("transcribe.py");
    std::fs::write(&script, SCRIPT)?;

    let use_cuda = o.device != "cpu" && has_nvidia();
    let device = if o.device == "auto" && !use_cuda { "cpu".to_string() } else { o.device.clone() };
    let mut cmd = if let Some(py) = &o.python.clone().or_else(|| std::env::var_os("FLITS_PYTHON").map(PathBuf::from)) {
        Command::new(py)
    } else if has("uv") {
        let mut c = Command::new("uv");
        c.args(["run", "--quiet", "--no-project", "--python", "3.12", "--with", "faster-whisper>=1.1"]);
        if use_cuda {
            c.args(["--with", "nvidia-cublas-cu12", "--with", "nvidia-cudnn-cu12>=9,<10"]);
        }
        c.arg("python");
        c
    } else {
        bail!(
            "transcription needs either `uv` (https://docs.astral.sh/uv/) or --python pointing at a Python with faster-whisper installed"
        );
    };
    // decode with ffmpeg (fast, any format) to raw 16 kHz mono float32
    let pcm = dir.join(format!("audio-{}.f32", std::process::id()));
    let t = std::time::Instant::now();
    crate::media::run(
        crate::media::ffmpeg().arg("-y").arg("-i").arg(input).args(["-vn", "-ac", "1", "-ar", "16000", "-f", "f32le"]).arg(&pcm),
        "audio decode",
    )?;
    eprintln!("flits: decoded audio in {:.1}s", t.elapsed().as_secs_f64());
    cmd.arg("-I").arg(&script).arg(&pcm).arg(out_json).arg(&o.model).arg(&o.language).arg(&device);
    let status = cmd.status().context("starting the transcription helper");
    let _ = std::fs::remove_file(&pcm);
    if !status?.success() {
        bail!("transcription failed");
    }
    let words = load_transcript(out_json)?;
    write_subtitles(out_json, &words)?;
    Ok(out_json.to_path_buf())
}

fn ts(t: f64, sep: char) -> String {
    let ms = (t.max(0.0) * 1000.0).round() as u64;
    format!("{:02}:{:02}:{:02}{sep}{:03}", ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000)
}

/// Readable subtitle lines (max ~42 chars, split at pauses/punctuation).
pub fn cues(words: &[Word]) -> Vec<(f64, f64, String)> {
    let mut out: Vec<(f64, f64, String)> = vec![];
    let mut cur: Vec<&Word> = vec![];
    let flush = |cur: &mut Vec<&Word>, out: &mut Vec<(f64, f64, String)>| {
        if let (Some(a), Some(b)) = (cur.first(), cur.last()) {
            out.push((a.start, b.end, cur.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ")));
        }
        cur.clear();
    };
    for w in words {
        if let Some(last) = cur.last() {
            let len: usize = cur.iter().map(|w| w.text.len() + 1).sum();
            if len + w.text.len() > 42 || w.start - last.end > 0.8 || last.text.ends_with(['.', '?', '!']) {
                flush(&mut cur, &mut out);
            }
        }
        cur.push(w);
    }
    flush(&mut cur, &mut out);
    out
}

pub fn write_subtitles(json: &Path, words: &[Word]) -> Result<()> {
    let c = cues(words);
    let mut srt = String::new();
    let mut vtt = String::from("WEBVTT\n\n");
    for (i, (a, b, t)) in c.iter().enumerate() {
        srt += &format!("{}\n{} --> {}\n{}\n\n", i + 1, ts(*a, ','), ts(*b, ','), t);
        vtt += &format!("{} --> {}\n{}\n\n", ts(*a, '.'), ts(*b, '.'), t);
    }
    let txt = c.iter().map(|x| x.2.as_str()).collect::<Vec<_>>().join("\n");
    std::fs::write(json.with_extension("srt"), srt)?;
    std::fs::write(json.with_extension("vtt"), vtt)?;
    std::fs::write(json.with_extension("txt"), txt)?;
    Ok(())
}

/// `<dir>/<stem>.words.json` next to the media file.
pub fn default_json(input: &Path) -> PathBuf {
    let stem = input.file_stem().and_then(|s| s.to_str()).unwrap_or("transcript");
    input.with_file_name(format!("{stem}.words.json"))
}

/// Reuse an existing transcript or make one.
pub fn ensure(input: &Path, explicit: Option<&Path>, o: &TranscribeOpts) -> Result<PathBuf> {
    if let Some(p) = explicit {
        return Ok(p.to_path_buf());
    }
    let p = default_json(input);
    if p.exists() {
        eprintln!("flits: using existing transcript {}", p.display());
        return Ok(p);
    }
    transcribe(input, &p, o)
}
