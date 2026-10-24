//! Speech-to-text via faster-whisper (run through `uv`, so no manual Python setup).

use crate::compose::captions::{load_transcript, Word};
use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

mod daemon;

const SCRIPT: &str = include_str!("whisper.py");

fn has(cmd: &str) -> bool {
    Command::new(cmd).arg("--version").output().map(|o| o.status.success()).unwrap_or(false)
}

fn has_nvidia() -> bool {
    Command::new("nvidia-smi").arg("-L").output().map(|o| o.status.success()).unwrap_or(false)
}

#[derive(Clone, Debug)]
pub struct TranscribeOpts {
    pub model: String,
    pub language: String,
    pub device: String,
    pub python: Option<PathBuf>,
    /// remote server URL, "off", or None = use the configured one if reachable
    pub remote: Option<String>,
    /// text that primes whisper: names, jargon, spelling, style (faster-whisper `initial_prompt`)
    pub initial_prompt: Option<String>,
    /// terms to favour, comma separated (faster-whisper `hotwords`)
    pub hotwords: Option<String>,
    /// word timestamps (false = segments only, faster)
    pub word_timestamps: bool,
    /// speaker labels via pyannote.audio (optional dependency; needs HF_TOKEN)
    pub diarize: bool,
    /// number of speakers for diarization (None = automatic)
    pub speakers: Option<u32>,
}

impl Default for TranscribeOpts {
    fn default() -> Self {
        TranscribeOpts {
            model: "default".into(),
            language: "auto".into(),
            device: "auto".into(),
            python: None,
            remote: None,
            initial_prompt: None,
            hotwords: None,
            word_timestamps: true,
            diarize: false,
            speakers: None,
        }
    }
}

/// Explanation shown when `--diarize` is used without a Hugging Face token.
pub const DIARIZE_HELP: &str = "speaker diarization uses pyannote.audio, which downloads its model from Hugging Face:\n  \
     1. create a free account and a read token at https://huggingface.co/settings/tokens\n  \
     2. accept the user conditions of https://huggingface.co/pyannote/speaker-diarization-community-1\n     \
     (pyannote.audio 4; for 3.x: pyannote/speaker-diarization-3.1 and pyannote/segmentation-3.0)\n  \
     3. set HF_TOKEN to that token (e.g. `export HF_TOKEN=hf_...` or `$env:HF_TOKEN=\"hf_...\"`)\n\
     flits then installs pyannote.audio through uv on first use (a large download: PyTorch).";

/// Read a vocabulary file: terms separated by new lines or commas, `#` starts a comment.
pub fn read_vocab(path: &Path) -> Result<String> {
    let txt = std::fs::read_to_string(path).with_context(|| format!("reading vocabulary {}", path.display()))?;
    let terms: Vec<&str> = txt
        .lines()
        .map(|l| l.split('#').next().unwrap_or(""))
        .flat_map(|l| l.split([',', ';']))
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .collect();
    let mut s = terms.join(", ");
    // whisper's prompt window is ~224 tokens; keep the start of a long list
    if s.chars().count() > 800 {
        eprintln!("flits: vocabulary is long; only the first ~800 characters are used");
        s = s.chars().take(800).collect::<String>();
        if let Some(i) = s.rfind(',') {
            s.truncate(i);
        }
    }
    Ok(s)
}

fn has_hf_token() -> bool {
    ["HF_TOKEN", "HUGGINGFACE_TOKEN"].iter().any(|k| std::env::var(k).is_ok_and(|v| !v.trim().is_empty()))
}

/// Transcribe `input`; returns the path of the JSON transcript (also writes .srt/.vtt/.txt).
/// Uses a configured `flits serve` GPU machine when reachable, otherwise runs locally.
pub fn transcribe(input: &Path, out_json: &Path, o: &TranscribeOpts) -> Result<PathBuf> {
    transcribe_many(&[(input.to_path_buf(), out_json.to_path_buf())], o)?;
    Ok(out_json.to_path_buf())
}

fn remote_for(o: &TranscribeOpts) -> Option<crate::remote::Remote> {
    match o.remote.as_deref() {
        Some("off") => None,
        Some(url) => Some(crate::remote::Remote {
            url: url.trim_end_matches('/').to_string(),
            token: std::env::var("FLITS_TOKEN").ok().or_else(|| crate::remote::configured().map(|r| r.token)).unwrap_or_default(),
        }),
        None => crate::remote::configured(),
    }
}

/// Transcribe several files with one model load: (input, output JSON) pairs.
pub fn transcribe_many(jobs: &[(PathBuf, PathBuf)], o: &TranscribeOpts) -> Result<()> {
    for (input, _) in jobs {
        if !input.exists() {
            bail!("file not found: {}", input.display());
        }
    }
    if let Some(r) = remote_for(o) {
        match r.health() {
            Ok(h) => {
                eprintln!("flits: using GPU server {} ({})", r.url, h["gpu"].as_str().unwrap_or("no GPU"));
                let mut ok = true;
                for (input, out) in jobs {
                    if let Err(e) = r.transcribe(input, out, o) {
                        eprintln!("flits: remote transcription failed ({e:#}); transcribing locally");
                        ok = false;
                        break;
                    }
                    write_subtitles(out, &load_transcript(out)?)?;
                }
                if ok {
                    return Ok(());
                }
            }
            Err(e) => eprintln!("flits: GPU server {} not reachable ({e}); transcribing locally", r.url),
        }
    }
    transcribe_local(jobs, o)
}

fn transcribe_local(jobs: &[(PathBuf, PathBuf)], o: &TranscribeOpts) -> Result<()> {
    use rayon::prelude::*;
    let dir = std::env::temp_dir().join("flits");
    std::fs::create_dir_all(&dir)?;
    let script = dir.join("whisper.py");
    std::fs::write(&script, SCRIPT)?;
    if o.diarize && !has_hf_token() {
        bail!("--diarize needs a Hugging Face token in HF_TOKEN.\n{DIARIZE_HELP}");
    }

    let use_cuda = o.device != "cpu" && has_nvidia();
    let device = if o.device == "auto" && !use_cuda { "cpu".to_string() } else { o.device.clone() };
    let make_cmd = || -> Result<Command> {
        Ok(if let Some(py) = &o.python.clone().or_else(|| std::env::var_os("FLITS_PYTHON").map(PathBuf::from)) {
            Command::new(py)
        } else if has("uv") {
            let mut c = Command::new("uv");
            c.args(["run", "--quiet", "--no-project", "--python", "3.12", "--with", "faster-whisper>=1.1"]);
            if use_cuda {
                c.args(["--with", "nvidia-cublas-cu12", "--with", "nvidia-cudnn-cu12>=9,<10"]);
            }
            if o.diarize {
                // optional and heavy (PyTorch): only pulled in when asked for
                eprintln!("flits: diarization on; the first run installs pyannote.audio + PyTorch (a large download)");
                c.args(["--with", "pyannote.audio>=3.3"]);
            }
            c.arg("python");
            c
        } else {
            bail!(
            "transcription needs either `uv` (https://docs.astral.sh/uv/) or --python pointing at a Python with faster-whisper installed"
        );
        })
    };
    // decode everything with ffmpeg in parallel (fast, any format) to raw 16 kHz mono float32
    let t = std::time::Instant::now();
    let pcms: Vec<PathBuf> = (0..jobs.len()).map(|i| dir.join(format!("audio-{}-{i}.f32", std::process::id()))).collect();
    jobs.par_iter().zip(&pcms).try_for_each(|((input, _), pcm)| {
        crate::media::run(
            crate::media::ffmpeg().arg("-y").arg("-i").arg(input).args(["-vn", "-ac", "1", "-ar", "16000", "-f", "f32le"]).arg(pcm),
            "audio decode",
        )
    })?;
    eprintln!("flits: decoded {} file(s) in {:.1}s", jobs.len(), t.elapsed().as_secs_f64());
    let job_file = dir.join(format!("jobs-{}.json", std::process::id()));
    let list: Vec<serde_json::Value> = jobs.iter().zip(&pcms).map(|((_, out), pcm)| serde_json::json!({"pcm": pcm, "out": out})).collect();
    let cfg = serde_json::json!({
        "model": o.model, "lang": o.language, "device": device,
        "words": o.word_timestamps && std::env::var("FLITS_WORDS").map_or(true, |v| v != "0"),
        "prompt": o.initial_prompt, "hotwords": o.hotwords, "diarize": o.diarize, "speakers": o.speakers,
        "jobs": list
    });
    std::fs::write(&job_file, cfg.to_string())?;
    // the warm worker (model already loaded) when it can; diarization runs on its own
    if !o.diarize && daemon::enabled() {
        match daemon::run(&dir, make_cmd()?, &script, &job_file) {
            Ok(()) => {
                for p in pcms.iter().chain(std::iter::once(&job_file)) {
                    let _ = std::fs::remove_file(p);
                }
                for (_, out) in jobs {
                    write_subtitles(out, &load_transcript(out)?)?;
                }
                return Ok(());
            }
            Err(e) => eprintln!("flits: whisper worker unavailable ({e:#}); running whisper directly"),
        }
    }
    let mut cmd = make_cmd()?;
    cmd.arg("-I").arg(&script).arg(&job_file);
    let status = cmd.status().context("starting the transcription helper");
    for p in pcms.iter().chain(std::iter::once(&job_file)) {
        let _ = std::fs::remove_file(p);
    }
    if !status?.success() {
        bail!("transcription failed");
    }
    for (_, out) in jobs {
        write_subtitles(out, &load_transcript(out)?)?;
    }
    Ok(())
}

fn ts(t: f64, sep: char) -> String {
    let ms = (t.max(0.0) * 1000.0).round() as u64;
    format!("{:02}:{:02}:{:02}{sep}{:03}", ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000)
}

/// Subtitle cues with the speaker of their words (None without diarization).
fn speaker_cues(words: &[Word]) -> Vec<(f64, f64, String, Option<&str>)> {
    fn flush<'a>(cur: &mut Vec<&'a Word>, out: &mut Vec<(f64, f64, String, Option<&'a str>)>) {
        if let (Some(a), Some(b)) = (cur.first(), cur.last()) {
            let text = cur.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ");
            out.push((a.start, b.end, text, a.speaker.as_deref()));
        }
        cur.clear();
    }
    let mut out = vec![];
    let mut cur: Vec<&Word> = vec![];
    for w in words {
        if let Some(last) = cur.last() {
            let len: usize = cur.iter().map(|w| w.text.len() + 1).sum();
            if len + w.text.len() > 42 || w.start - last.end > 0.8 || last.text.ends_with(['.', '?', '!']) || last.speaker != w.speaker {
                flush(&mut cur, &mut out);
            }
        }
        cur.push(w);
    }
    flush(&mut cur, &mut out);
    out
}

/// Readable subtitle lines (max ~42 chars, split at pauses/punctuation and speaker changes).
pub fn cues(words: &[Word]) -> Vec<(f64, f64, String)> {
    speaker_cues(words).into_iter().map(|(a, b, t, _)| (a, b, t)).collect()
}

pub fn write_subtitles(json: &Path, words: &[Word]) -> Result<()> {
    let c = speaker_cues(words);
    let mut srt = String::new();
    let mut vtt = String::from("WEBVTT\n\n");
    let mut txt_lines = vec![];
    let mut prev_speaker = None;
    for (i, (a, b, t, sp)) in c.iter().enumerate() {
        // SRT/TXT: name the speaker when it changes; VTT: a voice tag on every cue
        let label = match sp {
            Some(s) if prev_speaker != Some(*s) => format!("[{s}] "),
            _ => String::new(),
        };
        prev_speaker = *sp;
        srt += &format!("{}\n{} --> {}\n{label}{t}\n\n", i + 1, ts(*a, ','), ts(*b, ','));
        let voice = sp.map(|s| format!("<v {s}>")).unwrap_or_default();
        vtt += &format!("{} --> {}\n{voice}{t}\n\n", ts(*a, '.'), ts(*b, '.'));
        txt_lines.push(format!("{label}{t}"));
    }
    let txt = txt_lines.join("\n");
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

/// Rewrite a transcript JSON as one flat word list (`--words-only`):
/// `{"language": .., "duration": .., "words": [{"word", "start", "end", "prob", "speaker"?}]}`.
pub fn to_words_only(json: &Path) -> Result<()> {
    let mut v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(json)?)?;
    let mut words = vec![];
    for s in v["segments"].as_array().map(Vec::as_slice).unwrap_or_default() {
        for w in s["words"].as_array().map(Vec::as_slice).unwrap_or_default() {
            let mut w = w.clone();
            if w["speaker"].is_null() && !s["speaker"].is_null() {
                w["speaker"] = s["speaker"].clone();
            }
            words.push(w);
        }
    }
    if let Some(o) = v.as_object_mut() {
        o.remove("segments");
        o.insert("words".into(), serde_json::Value::Array(words));
    }
    std::fs::write(json, serde_json::to_string_pretty(&v)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(text: &str, start: f64, speaker: Option<&str>) -> Word {
        Word { text: text.into(), start, end: start + 0.3, speaker: speaker.map(str::to_string) }
    }

    #[test]
    fn subtitles_split_and_label_speakers() {
        let words =
            [word("Hi", 0.0, Some("A")), word("there", 0.4, Some("A")), word("hello", 0.8, Some("B")), word("again", 1.2, Some("B"))];
        let c = cues(&words);
        assert_eq!(c.len(), 2);
        assert_eq!(c[1].2, "hello again");
        let dir = std::env::temp_dir().join(format!("flits-subs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let json = dir.join("t.words.json");
        write_subtitles(&json, &words).unwrap();
        let srt = std::fs::read_to_string(dir.join("t.words.srt")).unwrap();
        assert!(srt.contains("[A] Hi there") && srt.contains("[B] hello again"), "{srt}");
        let vtt = std::fs::read_to_string(dir.join("t.words.vtt")).unwrap();
        assert!(vtt.contains("<v B>hello again"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn words_only_flattens_segments_with_speakers() {
        let dir = std::env::temp_dir().join(format!("flits-wo-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("t.json");
        let v = serde_json::json!({"language": "en", "segments": [
            {"start": 0.0, "end": 1.0, "text": "Hi there", "speaker": "SPEAKER_00",
             "words": [{"word": "Hi", "start": 0.0, "end": 0.4}, {"word": "there", "start": 0.5, "end": 1.0, "speaker": "SPEAKER_01"}]}
        ]});
        std::fs::write(&p, v.to_string()).unwrap();
        to_words_only(&p).unwrap();
        let out: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert!(out.get("segments").is_none());
        assert_eq!(out["language"], "en");
        assert_eq!(out["words"][0]["speaker"], "SPEAKER_00");
        assert_eq!(out["words"][1]["speaker"], "SPEAKER_01");
        let tr = crate::compose::captions::load_transcript_full(&p).unwrap();
        assert_eq!(tr.language.as_deref(), Some("en"));
        assert_eq!(tr.words().len(), 2);
        assert_eq!(tr.words()[1].speaker.as_deref(), Some("SPEAKER_01"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reads_vocabulary_files() {
        let dir = std::env::temp_dir().join(format!("flits-vocab-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("v.txt");
        std::fs::write(&p, "# names\nArjan den Hartog\nflits, NVENC; wgpu\n\n").unwrap();
        assert_eq!(read_vocab(&p).unwrap(), "Arjan den Hartog, flits, NVENC, wgpu");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
