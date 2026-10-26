//! Speech-to-text via faster-whisper (run through `uv`, so no manual Python setup).

use crate::compose::captions::{load_transcript_full, Transcript, Word};
use crate::compose::cues;
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
                    finish(out)?;
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

/// The helper script in `dir`, written only when it changed and then atomically: the prewarm
/// thread and the transcription may get here at the same time, and a worker starting must never
/// read a half-written file.
fn write_script(dir: &Path) -> Result<PathBuf> {
    let script = dir.join("whisper.py");
    if std::fs::read_to_string(&script).ok().as_deref() != Some(SCRIPT) {
        let tmp = dir.join(format!("whisper.py.{}.{:?}.tmp", std::process::id(), std::thread::current().id()));
        std::fs::write(&tmp, SCRIPT)?;
        if std::fs::rename(&tmp, &script).is_err() {
            let _ = std::fs::remove_file(&tmp); // another writer won; its content is the same
        }
    }
    Ok(script)
}

/// The command that runs the helper script (up to and including the interpreter).
fn helper_command(o: &TranscribeOpts, use_cuda: bool) -> Result<Command> {
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
}

/// Start the warm whisper worker and load the model, waiting until it is ready (`flits warm`).
pub fn warm_now(o: &TranscribeOpts) -> Result<()> {
    if !daemon::enabled() {
        bail!("the whisper worker is turned off (FLITS_WHISPER_DAEMON=0)");
    }
    let dir = crate::media::process::user_temp_dir()?;
    let script = write_script(&dir)?;
    let use_cuda = o.device != "cpu" && has_nvidia();
    let device = if o.device == "auto" && !use_cuda { "cpu".to_string() } else { o.device.clone() };
    daemon::warm(&dir, &|| helper_command(o, use_cuda), &script, &o.model, &device)
}

/// Start the warm whisper worker and load the model in the background, so it is ready by the time
/// the audio is decoded and the transcription is asked for (call it as early as possible). Does
/// nothing when the worker is off, a GPU server is used, or diarization is on.
pub fn prewarm(o: &TranscribeOpts) {
    if !daemon::enabled() || o.diarize || remote_for(o).is_some() {
        return;
    }
    let o = o.clone();
    std::thread::spawn(move || {
        let warm = || -> Result<()> {
            let dir = crate::media::process::user_temp_dir()?;
            let script = write_script(&dir)?;
            let use_cuda = o.device != "cpu" && has_nvidia();
            let device = if o.device == "auto" && !use_cuda { "cpu".to_string() } else { o.device.clone() };
            daemon::warm(&dir, &|| helper_command(&o, use_cuda), &script, &o.model, &device)
        };
        // a failure here shows up (and falls back) when the transcription itself runs
        let _ = warm();
    });
}

fn transcribe_local(jobs: &[(PathBuf, PathBuf)], o: &TranscribeOpts) -> Result<()> {
    use rayon::prelude::*;
    // the helper script and its job files: in a folder no other user can write to
    let dir = crate::media::process::user_temp_dir()?;
    let script = write_script(&dir)?;
    if o.diarize && !has_hf_token() {
        bail!("--diarize needs a Hugging Face token in HF_TOKEN.\n{DIARIZE_HELP}");
    }

    let use_cuda = o.device != "cpu" && has_nvidia();
    let device = if o.device == "auto" && !use_cuda { "cpu".to_string() } else { o.device.clone() };
    let make_cmd = || helper_command(o, use_cuda);
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
        match daemon::run(&dir, &make_cmd, &script, &job_file) {
            Ok(()) => {
                for p in pcms.iter().chain(std::iter::once(&job_file)) {
                    let _ = std::fs::remove_file(p);
                }
                for (_, out) in jobs {
                    finish(out)?;
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
        finish(out)?;
    }
    Ok(())
}

fn ts(t: f64, sep: char) -> String {
    let ms = (t.max(0.0) * 1000.0).round() as u64;
    format!("{:02}:{:02}:{:02}{sep}{:03}", ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000)
}

/// Subtitle cues with the speaker of their words (None without diarization): the captions layer's
/// grouping rules on one line of at most 42 characters, phrase-aware for languages with a
/// `noBreakAfter` list, and never overlapping.
fn speaker_cues(words: &[Word], language: Option<&str>) -> Vec<(f64, f64, String, Option<String>)> {
    let weak = language.and_then(cues::weak_words);
    let rules = cues::Rules {
        max_chars: 42,
        max_gap: 0.8,
        sentence_end: &['.', '?', '!'],
        comma_break: false,
        phrase: weak.is_some(),
        no_break_after: weak.unwrap_or_default().iter().map(|w| w.to_string()).collect(),
        ..cues::Rules::default()
    };
    let groups = rules.group(words, &mut |_| true);
    let timing = cues::Timing { lead: 0.0, hold: 0.0, gap_close: 0.0, min_duration: None };
    let times = timing.apply(&groups, f64::INFINITY);
    groups.iter().zip(times).map(|(g, (a, b))| (a, b, cues::text(g), g[0].speaker.clone())).collect()
}

/// Readable subtitle lines (max ~42 chars, split at pauses/punctuation and speaker changes).
pub fn cues(words: &[Word]) -> Vec<(f64, f64, String)> {
    speaker_cues(words, None).into_iter().map(|(a, b, t, _)| (a, b, t)).collect()
}

/// SRT text for (start, end, text) cues.
pub fn srt(cues: &[(f64, f64, String)]) -> String {
    cues.iter().enumerate().map(|(i, (a, b, t))| format!("{}\n{} --> {}\n{t}\n\n", i + 1, ts(*a, ','), ts(*b, ','))).collect()
}

/// Write `<json>.srt/.vtt/.txt` next to a transcript JSON.
pub fn write_subtitles(json: &Path, tr: &Transcript) -> Result<()> {
    let c = speaker_cues(&tr.words(), tr.language.as_deref());
    let mut srt = String::new();
    let mut vtt = String::from("WEBVTT\n\n");
    let mut txt_lines = vec![];
    let mut prev_speaker = None;
    for (i, (a, b, t, sp)) in c.iter().enumerate() {
        // SRT/TXT: name the speaker when it changes; VTT: a voice tag on every cue
        let label = match sp {
            Some(s) if prev_speaker != Some(s.as_str()) => format!("[{s}] "),
            _ => String::new(),
        };
        prev_speaker = sp.as_deref();
        srt += &format!("{}\n{} --> {}\n{label}{t}\n\n", i + 1, ts(*a, ','), ts(*b, ','));
        let voice = sp.as_ref().map(|s| format!("<v {s}>")).unwrap_or_default();
        vtt += &format!("{} --> {}\n{voice}{t}\n\n", ts(*a, '.'), ts(*b, '.'));
        txt_lines.push(format!("{label}{t}"));
    }
    let txt = txt_lines.join("\n");
    std::fs::write(json.with_extension("srt"), srt)?;
    std::fs::write(json.with_extension("vtt"), vtt)?;
    std::fs::write(json.with_extension("txt"), txt)?;
    Ok(())
}

/// Tidy a fresh transcript JSON (sentence-sized segments) and write its subtitles next to it.
fn finish(json: &Path) -> Result<()> {
    split_segments(json)?;
    write_subtitles(json, &load_transcript_full(json)?)
}

/// A pause between words this long (s) starts a new segment.
const SEGMENT_PAUSE: f64 = 0.7;

/// Re-split whisper's segments (sometimes one per file: 20+ s and many sentences) at sentence ends
/// and long pauses, using the word timings. Words, their fields and the rest of the file stay as they are.
pub fn split_segments(json: &Path) -> Result<()> {
    let mut v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(json)?)?;
    let Some(segs) = v["segments"].as_array() else { return Ok(()) };
    let mut out = Vec::with_capacity(segs.len());
    let mut changed = false;
    for s in segs {
        let words = s["words"].as_array().map(Vec::as_slice).unwrap_or_default();
        let parts = sentence_parts(words);
        if parts.len() <= 1 {
            out.push(s.clone());
            continue;
        }
        changed = true;
        for part in parts {
            let mut seg = s.clone();
            let text: String = part.iter().filter_map(|w| w["word"].as_str()).collect();
            seg["text"] = serde_json::Value::from(text.trim());
            seg["start"] = part[0]["start"].clone();
            seg["end"] = part[part.len() - 1]["end"].clone();
            seg["words"] = serde_json::Value::Array(part.to_vec());
            out.push(seg);
        }
    }
    if changed {
        v["segments"] = serde_json::Value::Array(out);
        std::fs::write(json, serde_json::to_string_pretty(&v)?)?;
    }
    Ok(())
}

/// Split words after a sentence end (.?!, also before a closing quote or bracket) that another word
/// follows, and at pauses of `SEGMENT_PAUSE` or more.
fn sentence_parts(words: &[serde_json::Value]) -> Vec<&[serde_json::Value]> {
    let mut parts = vec![];
    let mut from = 0;
    for i in 1..words.len() {
        let prev = words[i - 1]["word"].as_str().unwrap_or("").trim_end_matches(['"', '\'', '\u{201d}', '\u{2019}', ')']);
        let gap = words[i]["start"].as_f64().unwrap_or(0.0) - words[i - 1]["end"].as_f64().unwrap_or(0.0);
        if prev.ends_with(['.', '?', '!']) || gap >= SEGMENT_PAUSE {
            parts.push(&words[from..i]);
            from = i;
        }
    }
    if from < words.len() {
        parts.push(&words[from..]);
    }
    parts
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

    fn transcript(words: &[Word], language: Option<&str>) -> Transcript {
        let seg = crate::compose::captions::Segment {
            start: words[0].start,
            end: words[words.len() - 1].end,
            text: cues::text(words),
            speaker: None,
            words: words.to_vec(),
        };
        Transcript { language: language.map(str::to_string), segments: vec![seg] }
    }

    #[test]
    fn splits_segments_at_sentences_and_pauses() {
        let w = |t: &str, a: f64, b: f64| serde_json::json!({"word": t, "start": a, "end": b, "probability": 0.9});
        let v = serde_json::json!({"language": "nl", "segments": [{"start": 0.0, "end": 3.6, "text": "x", "words": [
            w(" Zo,", 0.0, 0.4), w(" klaar.", 0.5, 0.9), w(" Over", 1.0, 1.2), w(" twee", 1.2, 1.4),
            w(" weken", 2.5, 2.8), w(" dan!", 2.9, 3.1), w(" Tot", 3.2, 3.4), w(" 3.5", 3.4, 3.6)]}]});
        let p = std::env::temp_dir().join(format!("flits-split-{}.words.json", std::process::id()));
        std::fs::write(&p, v.to_string()).unwrap();
        split_segments(&p).unwrap();
        let out: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        let _ = std::fs::remove_file(&p);
        let segs = out["segments"].as_array().unwrap();
        let texts: Vec<&str> = segs.iter().map(|s| s["text"].as_str().unwrap()).collect();
        // sentence ends and the 1.1 s pause split; "3.5" (a number) does not
        assert_eq!(texts, ["Zo, klaar.", "Over twee", "weken dan!", "Tot 3.5"]);
        assert_eq!((segs[2]["start"].as_f64(), segs[2]["end"].as_f64()), (Some(2.5), Some(3.1)));
        assert_eq!(segs[0]["words"][0]["probability"], 0.9);
        assert_eq!(out["language"], "nl");
    }

    #[test]
    fn subtitle_cues_use_the_caption_rules() {
        // words that run into each other: cues still never overlap
        let words: Vec<Word> = "We walked along the beach with all of the children of the village and then went home."
            .split_whitespace()
            .enumerate()
            .map(|(i, t)| Word { text: t.into(), start: i as f64 * 0.3, end: i as f64 * 0.3 + 0.45, speaker: None })
            .collect();
        let c = speaker_cues(&words, Some("en"));
        assert!(c.len() > 1);
        for p in c.windows(2) {
            assert!(p[0].1 <= p[1].0, "{p:?}");
        }
        // English: no cue ends on "the", "of", "with", ...
        for (_, _, t, _) in &c[..c.len() - 1] {
            let last = t.split_whitespace().last().unwrap();
            assert!(!cues::WEAK_EN.contains(&last), "cue ends on a weak word: {c:?}");
        }
        assert!(srt(&[(0.0, 1.5, "Hi".into())]).starts_with("1\n00:00:00,000 --> 00:00:01,500\nHi\n"));
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
        write_subtitles(&json, &transcript(&words, None)).unwrap();
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
