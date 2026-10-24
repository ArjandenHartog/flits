//! Command line interface: argument definitions and dispatch. The commands live in
//! `make` (things that produce videos/audio) and `tools` (inspection, servers).

mod make;
mod tools;

use anyhow::{anyhow, Result};
use clap::{Args, Parser, Subcommand};
use flits::{transcribe, Prepared, Scene};
use std::path::{Path, PathBuf};
use std::time::Instant;

#[derive(Parser)]
#[command(
    name = "flits",
    version,
    about = "Blazing-fast GPU video and podcast renderer (JSON scene -> MP4) with built-in transcription",
    after_help = "Start with `flits new intro -o intro.json && flits render intro.json`, or write videos in TypeScript:\n`flits init my-video && flits preview my-video/src/video.ts`. `flits spec` documents the scene format;\n`flits doctor` checks your system."
)]
pub struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
    /// props for a TypeScript/JavaScript scene that exports a function: inline JSON or a .json file
    #[arg(long, global = true)]
    props: Option<String>,
}

#[derive(Args, Clone)]
pub struct RenderArgs {
    /// parallel render workers (default: automatic)
    #[arg(long, short = 'j')]
    pub workers: Option<usize>,
    /// auto | nvenc | qsv | vaapi | cpu | null (benchmark: discard frames)
    #[arg(long, default_value = "auto")]
    pub encoder: String,
    /// h264 | hevc | av1
    #[arg(long, default_value = "h264")]
    pub codec: String,
    /// encoder preset: fast | balanced | quality
    #[arg(long, default_value = "balanced")]
    pub speed: String,
    /// quality: lower = better (CRF/CQ), 18..28
    #[arg(long, default_value_t = 21)]
    pub quality: u32,
    /// render scale, e.g. 0.5 for a fast half-resolution preview
    #[arg(long, default_value_t = 1.0)]
    pub scale: f64,
    /// compositor: auto | gpu | cpu
    #[arg(long, default_value = "auto")]
    pub gpu: String,
    /// keep the temporary segment files
    #[arg(long)]
    pub keep_temp: bool,
    /// no progress output
    #[arg(long, short = 'q')]
    pub quiet: bool,
}

impl RenderArgs {
    pub fn options(&self) -> flits::Options {
        flits::Options {
            workers: self.workers,
            encoder: self.encoder.clone(),
            codec: self.codec.clone(),
            speed: self.speed.clone(),
            quality: self.quality,
            scale: self.scale,
            compositor: self.gpu.clone(),
            keep_temp: self.keep_temp,
            quiet: self.quiet,
        }
    }
}

#[derive(Args, Clone)]
pub struct WhisperArgs {
    /// whisper model: default (= large-v3-turbo on GPU, small on CPU), tiny, base, small, medium, large-v3, large-v3-turbo
    #[arg(long, default_value = "default")]
    pub model: String,
    /// language code (nl, en, ...) or auto
    #[arg(long, default_value = "auto")]
    pub lang: String,
    /// auto | cuda | cpu
    #[arg(long, default_value = "auto")]
    pub device: String,
    /// python with faster-whisper installed (default: run through uv)
    #[arg(long)]
    pub python: Option<PathBuf>,
    /// GPU server URL (default: the one saved with `flits remote`), or "off" to force local
    #[arg(long)]
    pub remote: Option<String>,
    /// text that primes whisper: names, jargon and spelling it should use ("Arjan, flits, NVENC")
    #[arg(long)]
    pub initial_prompt: Option<String>,
    /// file with terms to favour (one per line or comma separated): names, products, jargon
    #[arg(long)]
    pub vocab: Option<PathBuf>,
    /// label speakers (pyannote.audio, installed through uv on first use; needs HF_TOKEN)
    #[arg(long)]
    pub diarize: bool,
    /// number of speakers for --diarize (default: detect)
    #[arg(long, requires = "diarize")]
    pub speakers: Option<u32>,
}

impl WhisperArgs {
    pub fn opts(&self) -> Result<transcribe::TranscribeOpts> {
        Ok(transcribe::TranscribeOpts {
            model: self.model.clone(),
            language: self.lang.clone(),
            device: self.device.clone(),
            python: self.python.clone(),
            remote: self.remote.clone(),
            initial_prompt: self.initial_prompt.clone().filter(|p| !p.trim().is_empty()),
            hotwords: self.vocab.as_deref().map(transcribe::read_vocab).transpose()?.filter(|v| !v.is_empty()),
            word_timestamps: true,
            diarize: self.diarize,
            speakers: self.speakers,
        })
    }
}

#[derive(Args, Clone)]
pub struct CutArgs {
    /// silence threshold in dB
    #[arg(long, default_value_t = -35.0, allow_negative_numbers = true)]
    pub threshold: f64,
    /// shorten silences longer than this (seconds)
    #[arg(long, default_value_t = 0.6)]
    pub min: f64,
    /// pause left in place of a removed silence (seconds)
    #[arg(long, default_value_t = 0.25)]
    pub keep: f64,
    /// don't shorten silences (only cut words: fillers, repeats, --apply)
    #[arg(long)]
    pub no_silence: bool,
    /// also remove filler words, per transcript language: nl, en, de, fr, es, it, pt (transcribes automatically)
    #[arg(long)]
    pub fillers: bool,
    /// your own filler words/phrases instead of the built-in list, e.g. "uh,um,you know" (implies --fillers)
    #[arg(long)]
    pub fillers_list: Option<String>,
    /// also remove repeated words and stutters ("I I I think", "th- the")
    #[arg(long)]
    pub repeats: bool,
    /// only print the cut plan
    #[arg(long)]
    pub dry_run: bool,
    /// write an editor timeline instead of rendering: fcpxml | premiere | resolve | edl | json (comma separated for several)
    #[arg(long, value_delimiter = ',')]
    pub export: Vec<String>,
    /// with --export: render the cut as well
    #[arg(long)]
    pub render: bool,
    /// write <input>.edit.txt (one line per sentence) to edit the recording as text, then use --apply
    #[arg(long, conflicts_with_all = ["apply", "export", "dry_run"])]
    pub edit: bool,
    /// cut what was deleted from an edit file (made with --edit); silences are still shortened
    #[arg(long)]
    pub apply: Option<PathBuf>,
    /// with --edit: overwrite an existing edit file
    #[arg(long)]
    pub force: bool,
}

#[derive(Subcommand)]
enum Cmd {
    // ------------------------------------------------ scenes
    /// Render a scene (.json, or a .ts/.js module exporting one) to a video
    Render {
        scene: PathBuf,
        #[arg(short, long)]
        out: Option<PathBuf>,
        #[command(flatten)]
        r: RenderArgs,
        /// render on the configured GPU server (`flits remote`)
        #[arg(long)]
        remote: bool,
    },
    /// Render single frames to PNG (quick visual check)
    Still {
        scene: PathBuf,
        /// time(s) in seconds or mm:ss; repeat for several
        #[arg(long = "at", required = true, value_parser = parse_time)]
        at: Vec<f64>,
        #[arg(short, long)]
        out: PathBuf,
        #[arg(long, default_value_t = 1.0)]
        scale: f64,
    },
    /// Contact sheet: evenly spaced frames in one PNG (check a whole video at a glance)
    Sheet {
        scene: PathBuf,
        #[arg(short, long)]
        out: PathBuf,
        #[arg(long, default_value_t = 12)]
        count: usize,
        #[arg(long, default_value_t = 4)]
        cols: usize,
        /// width of each thumbnail in px
        #[arg(long, default_value_t = 480)]
        thumb: u32,
    },
    /// Live preview in the browser: scrub, play, step frames; reloads when the scene file changes
    Preview {
        scene: PathBuf,
        #[arg(long, default_value_t = 7878)]
        port: u16,
        /// render scale of the preview frames
        #[arg(long, default_value_t = 0.5)]
        scale: f64,
        /// compositor: auto | gpu | cpu
        #[arg(long, default_value = "auto")]
        gpu: String,
        /// don't open the browser
        #[arg(long)]
        no_open: bool,
    },
    /// Validate a scene and print its timeline
    Check { scene: PathBuf },
    /// Write a ready-to-render example scene (intro, quote, stats, lower-third, countdown, ...)
    New {
        /// starter name; omit to list them
        starter: Option<String>,
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// write it as a TypeScript module (default when -o ends in .ts)
        #[arg(long)]
        ts: bool,
    },
    /// Start a video project in TypeScript: components, springs, data, live preview (like Remotion)
    Init {
        /// folder to create
        #[arg(default_value = "my-video")]
        dir: PathBuf,
        /// only refresh the flits/ SDK of the project in the current folder (or `dir`)
        #[arg(long)]
        update: bool,
    },
    /// Print the scene format reference
    Spec,
    /// Print the JSON Schema of the scene format (for editors, validators, code generators and LLM agents)
    Schema {
        /// write to this file instead of stdout
        #[arg(short, long)]
        out: Option<PathBuf>,
    },
    /// Measure text: reads a JSON array of {text, font, size, maxWidth, lineHeight, balance, letterSpacing, weight}
    /// on stdin, prints [{width, height, lines}] (for scripts that place things next to text)
    Measure,

    // ------------------------------------------------ ready-made videos
    /// Montage from photos/clips (folders or files), optionally cut on the beat of music
    Slideshow {
        /// photos, videos or folders
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        #[arg(short, long)]
        out: Option<PathBuf>,
        #[arg(long)]
        music: Option<PathBuf>,
        /// 16:9 | 9:16 | 1:1 | 4:5
        #[arg(long, default_value = "16:9")]
        format: String,
        /// seconds per photo
        #[arg(long, default_value_t = 3.5)]
        per: f64,
        /// longest stretch of a video clip (seconds)
        #[arg(long, default_value_t = 6.0)]
        clip_max: f64,
        /// kenburns | cards | mixed
        #[arg(long, default_value = "mixed")]
        style: String,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        subtitle: Option<String>,
        /// closing card text
        #[arg(long)]
        outro: Option<String>,
        /// cut on the beat of the music
        #[arg(long)]
        beats: bool,
        /// don't stretch photo durations to the music length
        #[arg(long)]
        no_fit: bool,
        /// keep the sound of video clips under the music
        #[arg(long)]
        keep_audio: bool,
        #[arg(long, default_value = "#f26328")]
        accent: String,
        /// name | date
        #[arg(long, default_value = "name")]
        sort: String,
        #[command(flatten)]
        r: RenderArgs,
    },
    /// Burn animated word-by-word captions into a video (transcribes automatically)
    Captions {
        video: PathBuf,
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// named look: tiktok | karaoke-box | bounce | minimal | neon | highlighter | subtitle | word-pop-big
        #[arg(long)]
        preset: Option<String>,
        /// karaoke | pop | word | line | bounce (default: pop, or the preset's)
        #[arg(long)]
        style: Option<String>,
        /// active word colour (default: #ffd60a, or the preset's)
        #[arg(long)]
        highlight: Option<String>,
        #[arg(long)]
        uppercase: bool,
        /// vertical position as fraction of the height
        #[arg(long, default_value_t = 0.72)]
        position: f64,
        /// existing transcript (.json/.srt/.vtt) instead of transcribing
        #[arg(long)]
        transcript: Option<PathBuf>,
        #[command(flatten)]
        w: WhisperArgs,
        #[command(flatten)]
        r: RenderArgs,
    },
    /// Podcast (audio) to video: cover, live waveform, animated captions, progress bar
    Podcast {
        audio: PathBuf,
        #[arg(short, long)]
        out: Option<PathBuf>,
        #[arg(long)]
        cover: Option<PathBuf>,
        #[arg(long, default_value = "")]
        title: String,
        #[arg(long)]
        subtitle: Option<String>,
        /// 16:9 | 9:16 | 1:1 | 4:5
        #[arg(long, default_value = "16:9")]
        format: String,
        /// clip start (seconds or mm:ss)
        #[arg(long, value_parser = parse_time, default_value = "0")]
        from: f64,
        /// clip end (seconds or mm:ss)
        #[arg(long, value_parser = parse_time)]
        to: Option<f64>,
        #[arg(long, default_value = "#f26328")]
        accent: String,
        /// captions style: karaoke | pop | word | line | bounce
        #[arg(long)]
        style: Option<String>,
        #[arg(long)]
        transcript: Option<PathBuf>,
        #[command(flatten)]
        w: WhisperArgs,
        #[command(flatten)]
        r: RenderArgs,
    },
    /// Raw footage to a vertical reel in one go: transcribe (once), keep the part(s) you pick, cut
    /// silences and filler words, reframe to 9:16, animated captions, optional hook title
    Reel {
        video: PathBuf,
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// start of the part to use (seconds or mm:ss)
        #[arg(long, value_parser = parse_time)]
        from: Option<f64>,
        /// end of the part to use (seconds or mm:ss)
        #[arg(long, value_parser = parse_time)]
        to: Option<f64>,
        /// several parts, played in this order: "1:12-1:40,2:05-2:31"
        #[arg(long, conflicts_with_all = ["from", "to"])]
        clips: Option<String>,
        /// 9:16 | 1:1 | 4:5 | 16:9
        #[arg(long, default_value = "9:16")]
        format: String,
        /// crop (fill the frame) | fit (whole picture, captions underneath)
        #[arg(long, default_value = "crop")]
        fit: String,
        /// what to keep in frame when cropping: auto (follow the people, shot by shot) or a fixed
        /// point, 0 = left edge .. 0.5 = centre .. 1 = right edge
        #[arg(long, default_value = "auto")]
        focus: String,
        /// hook text at the top for the first seconds
        #[arg(long)]
        title: Option<String>,
        /// caption look: tiktok | karaoke-box | bounce | minimal | neon | highlighter | subtitle | word-pop-big
        #[arg(long)]
        preset: Option<String>,
        /// active word colour
        #[arg(long)]
        highlight: Option<String>,
        /// captions position as a fraction of the height
        #[arg(long)]
        position: Option<f64>,
        /// also remove filler words (uh, ehm, ...)
        #[arg(long)]
        fillers: bool,
        /// also remove repeated words and stutters
        #[arg(long)]
        repeats: bool,
        /// keep silences as they are
        #[arg(long)]
        no_silence: bool,
        /// pause left in place of a removed silence (seconds)
        #[arg(long, default_value_t = 0.2)]
        keep: f64,
        /// existing transcript (.json/.srt/.vtt) instead of transcribing
        #[arg(long)]
        transcript: Option<PathBuf>,
        #[command(flatten)]
        w: WhisperArgs,
        #[command(flatten)]
        r: RenderArgs,
    },
    /// Remove silences, filler words and repeats from audio or video, edit it as text (--edit/--apply),
    /// or export the cut as a Final Cut Pro / Premiere / Resolve timeline (--export)
    Cut {
        input: PathBuf,
        /// rendered output (default: <input>-cut.<ext>); exports are named after it
        #[arg(short, long)]
        out: Option<PathBuf>,
        #[command(flatten)]
        c: CutArgs,
        /// existing transcript (.json/.srt/.vtt) instead of transcribing
        #[arg(long)]
        transcript: Option<PathBuf>,
        #[command(flatten)]
        w: WhisperArgs,
        #[command(flatten)]
        r: RenderArgs,
    },
    /// Transcribe audio/video to word-timed JSON + SRT + VTT + TXT (several files: one model load)
    #[command(after_help = "Speaker labels (--diarize) are optional and need a Hugging Face token:\n  \
        1. create a read token at https://huggingface.co/settings/tokens\n  \
        2. accept the conditions of https://huggingface.co/pyannote/speaker-diarization-community-1\n  \
        3. set HF_TOKEN; the first run installs pyannote.audio + PyTorch through uv (large download)")]
    Transcribe {
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        /// output JSON (one input only; default: <input>.words.json)
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// folder for the outputs (default: next to each input)
        #[arg(long)]
        outdir: Option<PathBuf>,
        /// write one flat word list ({"words": [...]}) instead of segments with their words
        #[arg(long, conflicts_with = "segments_only")]
        words_only: bool,
        /// segments without word timestamps (faster; captions then spread words over each segment)
        #[arg(long)]
        segments_only: bool,
        #[command(flatten)]
        w: WhisperArgs,
    },

    // ------------------------------------------------ tools
    /// Show duration, size, frame rate, colour and streams of a media file
    Info { file: PathBuf },
    /// Check ffmpeg, encoders, GPU, fonts, transcription and the remote server
    Doctor,
    /// Run a server other machines can offload transcription and rendering to
    Serve {
        #[arg(long, default_value_t = 7777)]
        port: u16,
        /// shared secret (default: $FLITS_TOKEN or a new random one)
        #[arg(long)]
        token: Option<String>,
    },
    /// Set (or show) the GPU server this machine offloads to
    Remote {
        /// e.g. http://laptop:7777 ; "off" to disable
        url: Option<String>,
        #[arg(long)]
        token: Option<String>,
    },
    /// Time frame rendering per backend (developer tool)
    #[command(hide = true)]
    Bench {
        scene: PathBuf,
        #[arg(long, default_value_t = 0.0)]
        at: f64,
        #[arg(long, default_value_t = 60)]
        frames: i64,
    },
}

pub fn parse_time(s: &str) -> Result<f64, String> {
    let mut t = 0.0;
    for p in s.trim().split(':') {
        t = t * 60.0 + p.trim().parse::<f64>().map_err(|_| format!("bad time '{s}' (use seconds or mm:ss)"))?;
    }
    Ok(t)
}

/// Load + prepare a scene file, printing warnings.
pub fn load(scene_path: &Path, scale: f64) -> Result<Prepared> {
    let t = Instant::now();
    let (scene, warnings) = Scene::load(scene_path)?;
    for w in &warnings {
        eprintln!("warning: {w}");
    }
    let p = Prepared::build(&scene, scale)?;
    eprintln!("flits: prepared scene in {:.2}s", t.elapsed().as_secs_f64());
    Ok(p)
}

/// `dir/name.mp4` -> `dir/name<suffix>`
pub fn with_suffix(p: &Path, suffix: &str) -> PathBuf {
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("out");
    p.with_file_name(format!("{stem}{suffix}"))
}

/// Write a generated scene next to the output (for later tweaking) and render it.
pub fn write_scene_and_render(scene: serde_json::Value, out: &Path, r: &RenderArgs) -> Result<()> {
    let scene_path = with_suffix(out, ".scene.json");
    std::fs::write(&scene_path, serde_json::to_string_pretty(&scene)?)?;
    eprintln!("flits: scene written to {} (edit it and run `flits render` to tweak)", scene_path.display());
    let p = load(&scene_path, r.scale)?;
    flits::render_prepared(&p, out, &r.options()).map(|_| ())
}

pub fn main() {
    if let Err(e) = run(Cli::parse()) {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<()> {
    if let Some(p) = &cli.props {
        flits::scene::script::set_props(flits::scene::script::parse_props(p)?);
    }
    match cli.cmd {
        Cmd::Render { scene, out, r, remote } => make::render(&scene, out, &r, remote),
        Cmd::Still { scene, at, out, scale } => make::still(&scene, &at, &out, scale),
        Cmd::Sheet { scene, out, count, cols, thumb } => make::sheet(&scene, &out, count, cols, thumb),
        Cmd::Preview { scene, port, scale, gpu, no_open } => tools::preview(&scene, port, scale, &gpu, !no_open),
        Cmd::Check { scene } => tools::check(&scene),
        Cmd::New { starter, out, ts } => make::new(starter, out, ts),
        Cmd::Init { dir, update } => make::init(&dir, update),
        Cmd::Measure => tools::measure(),
        Cmd::Spec => {
            print!("{}", include_str!("../../SPEC.md"));
            Ok(())
        }
        Cmd::Schema { out } => {
            let schema = flits::scene::schema::scene_schema_pretty();
            match out {
                Some(path) => std::fs::write(&path, schema)?,
                None => print!("{schema}"),
            }
            Ok(())
        }
        Cmd::Slideshow {
            inputs,
            out,
            music,
            format,
            per,
            clip_max,
            style,
            title,
            subtitle,
            outro,
            beats,
            no_fit,
            keep_audio,
            accent,
            sort,
            r,
        } => {
            let o = flits::templates::slideshow::SlideshowOpts {
                inputs,
                music,
                format,
                per,
                clip_max,
                style,
                title,
                subtitle,
                outro,
                beats,
                fit_music: !no_fit,
                keep_audio,
                accent,
                sort,
            };
            make::slideshow(o, out, &r)
        }
        Cmd::Captions { video, out, preset, style, highlight, uppercase, position, transcript, w, r } => {
            let look = make::CaptionLook { preset, style, highlight, uppercase, position };
            make::captions(&video, out, look, transcript, &w, &r)
        }
        Cmd::Podcast { audio, out, cover, title, subtitle, format, from, to, accent, style, transcript, w, r } => {
            make::podcast(&audio, out, cover, title, subtitle, format, from, to, accent, style, transcript, &w, &r)
        }
        Cmd::Cut { input, out, c, transcript, w, r } => make::cut(&input, out, &c, transcript, &w, &r),
        Cmd::Reel {
            video,
            out,
            from,
            to,
            clips,
            format,
            fit,
            focus,
            title,
            preset,
            highlight,
            position,
            fillers,
            repeats,
            no_silence,
            keep,
            transcript,
            w,
            r,
        } => {
            let windows = match clips {
                Some(c) => c
                    .split(',')
                    .map(|part| {
                        let (a, b) = part.split_once('-').ok_or_else(|| anyhow!("bad clip '{part}': use start-end, e.g. 1:12-1:40"))?;
                        Ok((parse_time(a).map_err(|e| anyhow!(e))?, parse_time(b).map_err(|e| anyhow!(e))?))
                    })
                    .collect::<Result<Vec<_>>>()?,
                None if from.is_some() || to.is_some() => vec![(from.unwrap_or(0.0), to.unwrap_or(f64::INFINITY))],
                None => vec![],
            };
            let focus = match focus.as_str() {
                "auto" => None,
                f => Some(f.parse::<f64>().map_err(|_| anyhow!("--focus: auto or a number 0..1"))?),
            };
            let a = make::ReelArgs { windows, format, fit, focus, title, preset, highlight, position, fillers, repeats, no_silence, keep };
            make::reel(&video, out, &a, transcript, &w, &r)
        }
        Cmd::Transcribe { inputs, out, outdir, words_only, segments_only, w } => {
            make::transcribe(&inputs, out, outdir, words_only, segments_only, &w)
        }
        Cmd::Info { file } => tools::info(&file),
        Cmd::Doctor => tools::doctor(),
        Cmd::Serve { port, token } => tools::serve(port, token),
        Cmd::Remote { url, token } => tools::remote(url, token),
        Cmd::Bench { scene, at, frames } => tools::bench(&scene, at, frames),
    }
}
