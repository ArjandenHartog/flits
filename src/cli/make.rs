//! Commands that produce media.

use super::{load, with_suffix, write_scene_and_render, CutArgs, RenderArgs, WhisperArgs};
use anyhow::{anyhow, bail, Context, Result};
use flits::compose::captions::load_transcript_full;
use flits::templates::{self, cut, edit, export, fillers, slideshow, starters};
use flits::{media, render, transcribe, Scene};
use std::path::{Path, PathBuf};
use std::time::Instant;

pub fn render(scene: &Path, out: Option<PathBuf>, r: &RenderArgs, remote: bool) -> Result<()> {
    let out = out.unwrap_or_else(|| scene.with_extension("mp4"));
    if remote {
        let rm = flits::remote::configured()
            .ok_or_else(|| anyhow!("no GPU server configured: run `flits remote http://host:7777 --token ...`"))?;
        rm.health().map_err(|e| anyhow!("GPU server {} not reachable: {e}", rm.url))?;
        if flits::scene::script::is_script(scene) {
            // the server gets plain JSON: run the script here (media paths stay relative to its folder)
            let value = flits::scene::script::load(scene)?;
            Scene::from_value(value.clone(), scene.parent().map(Path::to_path_buf).unwrap_or_default())?;
            let json = scene.with_file_name(format!(".flits-{}.scene.json", std::process::id()));
            std::fs::write(&json, serde_json::to_string(&value)?)?;
            let res = rm.render(&json, &out, &format!("scale={}&quality={}", r.scale, r.quality));
            let _ = std::fs::remove_file(&json);
            return res;
        }
        return rm.render(scene, &out, &format!("scale={}&quality={}", r.scale, r.quality));
    }
    let p = load(scene, r.scale)?;
    flits::render_prepared(&p, &out, &r.options()).map(|_| ())
}

/// Create the folder an output file goes into.
fn out_dir(out: &Path) -> Result<()> {
    match out.parent().filter(|p| !p.as_os_str().is_empty()) {
        Some(d) => std::fs::create_dir_all(d).with_context(|| format!("creating {}", d.display())),
        None => Ok(()),
    }
}

pub fn still(scene: &Path, at: &[f64], out: &Path, scale: f64) -> Result<()> {
    out_dir(out)?;
    let p = load(scene, scale)?;
    for f in render::stills(&p, flits::pick_gpu("auto")?, at, out)? {
        println!("{}", f.display());
    }
    Ok(())
}

/// Which frames `flits sheet` shows.
pub struct SheetPick {
    pub count: usize,
    pub at: Vec<f64>,
    pub from: Option<f64>,
    pub to: Option<f64>,
}

pub fn sheet(scene: &Path, out: &Path, pick: &SheetPick, cols: usize, thumb: u32) -> Result<()> {
    out_dir(out)?;
    // load once (a TypeScript scene runs its script once), then prepare at thumbnail size
    let (s, warnings) = Scene::load(scene)?;
    for w in &warnings {
        eprintln!("warning: {w}");
    }
    let p = flits::Prepared::build(&s, thumb as f64 / s.width as f64)?;
    for w in &p.warnings {
        eprintln!("warning: {w}");
    }
    let frames = render::sheet_frames(&p, pick.count, &pick.at, pick.from, pick.to);
    render::sheet(&p, flits::pick_gpu("auto")?, &frames, cols, out)?;
    println!("{}", out.display());
    Ok(())
}

pub fn new(starter: Option<String>, out: Option<PathBuf>, ts: bool) -> Result<()> {
    let Some(name) = starter else {
        println!("starters (flits new <name> -o scene.json, or -o scene.ts for TypeScript):");
        let width = starters::STARTERS.iter().map(|(n, _)| n.len()).max().unwrap_or(0);
        for (n, d) in starters::STARTERS {
            println!("  {n:<width$}  {d}");
        }
        return Ok(());
    };
    let scene = starters::starter(&name)?;
    let ts = ts || out.as_ref().is_some_and(|o| flits::scene::script::is_script(o));
    let out = out.unwrap_or_else(|| PathBuf::from(format!("{name}.{}", if ts { "ts" } else { "json" })));
    if out.exists() {
        bail!("{} already exists", out.display());
    }
    let json = serde_json::to_string_pretty(&scene)?;
    let text = if !ts {
        json
    } else if templates::project::project_root(out.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."))).is_some() {
        // inside a `flits init` project: typed through the SDK
        format!("// flits starter '{name}'. Preview: flits preview {0}\nimport {{ defineScene }} from \"#flits\";\n\nexport default defineScene({json});\n", out.display())
    } else {
        format!("// flits starter '{name}'. Preview: flits preview {0} (`flits init` sets up typed TypeScript projects)\nexport default {json};\n", out.display())
    };
    std::fs::write(&out, text)?;
    println!("{}  (render with: flits render {})", out.display(), out.display());
    Ok(())
}

pub fn init(dir: &Path, update: bool) -> Result<()> {
    if update {
        let start = if dir == Path::new("my-video") && !dir.exists() { Path::new(".") } else { dir };
        let root = templates::project::project_root(start).ok_or_else(|| {
            anyhow!("no flits project here (a package.json with \"#flits\" in its imports); create one with `flits init <dir>`")
        })?;
        templates::project::write_sdk(&root)?;
        let skill = templates::skill::project_dir(&root);
        templates::skill::install(&skill)?;
        println!("updated {} and {} to flits {}", root.join("flits").display(), skill.display(), env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    templates::project::init(dir)?;
    let d = dir.display();
    println!("created a flits TypeScript project in {d}\n");
    println!("  flits preview {d}/src/video.ts                    live preview; edit src/video.ts and save");
    println!("  flits render {d}/src/video.ts -o {d}/out/video.mp4");
    println!("  cd {d} && npm install && npm run typecheck   optional: type-check with tsc\n");
    println!("Coding agents: CLAUDE.md and the flits-video skill (.claude/skills/flits-video) are included;");
    println!("  ask Claude Code in {d} for a video, e.g. \"make a 20 s launch video for ...\".\n");
    println!("{}", flits::scene::script::describe());
    Ok(())
}

/// `flits skill install [--global]` / `flits skill print`.
pub fn skill(print: bool, global: bool, dir: Option<PathBuf>) -> Result<()> {
    if print {
        print!("{}", templates::skill::print());
        return Ok(());
    }
    let target = match (global, dir) {
        (true, _) => templates::skill::global_dir()?,
        (false, Some(d)) => templates::skill::project_dir(&d),
        (false, None) => templates::skill::project_dir(Path::new(".")),
    };
    for f in templates::skill::install(&target)? {
        println!("{}", f.display());
    }
    eprintln!(
        "flits: installed the {} skill for Claude Code{}",
        templates::skill::NAME,
        if global { " (all projects)" } else { " (this project; --global for all projects)" }
    );
    Ok(())
}

pub fn slideshow(o: slideshow::SlideshowOpts, out: Option<PathBuf>, r: &RenderArgs) -> Result<()> {
    let out = out.unwrap_or_else(|| {
        let base =
            o.inputs.first().map(|p| if p.is_dir() { p.join("slideshow") } else { p.with_file_name("slideshow") }).unwrap_or_default();
        base.with_extension("mp4")
    });
    let scene = slideshow::slideshow(&o)?;
    write_scene_and_render(scene, &out, r)
}

/// How `flits captions` should look.
pub struct CaptionLook {
    pub preset: Option<String>,
    pub style: Option<String>,
    pub highlight: Option<String>,
    pub uppercase: bool,
    pub position: f64,
}

pub fn captions(
    video: &Path,
    out: Option<PathBuf>,
    look: CaptionLook,
    transcript: Option<PathBuf>,
    w: &WhisperArgs,
    r: &RenderArgs,
) -> Result<()> {
    let out = out.unwrap_or_else(|| with_suffix(video, "-captions.mp4"));
    if out == video {
        bail!("output would overwrite the input");
    }
    if let Some(p) = &look.preset {
        // fail before transcribing
        flits::scene::presets::caption_preset(p, (1920.0, 1080.0), None)?;
    }
    let tr = transcribe::ensure(video, transcript.as_deref(), &w.opts()?)?;
    let CaptionLook { preset, style, highlight, uppercase, position } = look;
    let opts = templates::CaptionOpts { video, transcript: &tr, preset, style, highlight, uppercase, position };
    let scene = templates::captioned_video(&opts)?;
    write_scene_and_render(scene, &out, r)
}

#[allow(clippy::too_many_arguments)]
pub fn podcast(
    audio: &Path,
    out: Option<PathBuf>,
    cover: Option<PathBuf>,
    title: String,
    subtitle: Option<String>,
    format: String,
    from: f64,
    to: Option<f64>,
    accent: String,
    style: Option<String>,
    transcript: Option<PathBuf>,
    w: &WhisperArgs,
    r: &RenderArgs,
) -> Result<()> {
    let out = out.unwrap_or_else(|| with_suffix(audio, "-video.mp4"));
    let tr = transcribe::ensure(audio, transcript.as_deref(), &w.opts()?)?;
    let title = if title.is_empty() { audio.file_stem().and_then(|s| s.to_str()).unwrap_or("Podcast").to_string() } else { title };
    let scene = templates::podcast(&templates::PodcastOpts {
        audio,
        transcript: &tr,
        cover: cover.as_deref(),
        title,
        subtitle,
        format,
        from,
        to,
        accent,
        style,
    })
    .context("building podcast scene")?;
    write_scene_and_render(scene, &out, r)
}

fn fmt_ts(t: f64) -> String {
    format!("{:02}:{:05.2}", (t / 60.0) as u64, t % 60.0)
}

pub fn cut(input: &Path, out: Option<PathBuf>, c: &CutArgs, transcript: Option<PathBuf>, w: &WhisperArgs, r: &RenderArgs) -> Result<()> {
    let info = media::probe(input)?;
    if !info.has_audio {
        bail!("{} has no audio to detect silences in", input.display());
    }
    let mut formats: Vec<export::Format> = c.export.iter().map(|f| export::Format::parse(f)).collect::<Result<_>>()?;
    // "fcpxml,resolve" is one file
    let mut seen = vec![];
    formats.retain(|f| {
        let new = !seen.contains(f);
        seen.push(*f);
        new
    });
    let custom_fillers = c.fillers_list.as_deref().map(fillers::parse_list);
    let want_fillers = c.fillers || custom_fillers.is_some();
    let edit_text = match &c.apply {
        Some(p) => Some(std::fs::read_to_string(p).with_context(|| format!("reading edit file {}", p.display()))?),
        None => None,
    };

    // the transcript: given, named in the edit file, existing next to the input, or made now
    let mut wo = w.opts()?;
    if (want_fillers || c.repeats) && wo.initial_prompt.is_none() {
        // make whisper write hesitations down instead of smoothing them away
        wo.initial_prompt = fillers::verbatim_prompt(&wo.language).map(str::to_string);
    }
    let transcript = transcript.or_else(|| edit_text.as_deref().and_then(edit::transcript_from_header));
    let tr_path = if want_fillers || c.repeats || c.edit || c.apply.is_some() {
        Some(transcribe::ensure(input, transcript.as_deref(), &wo)?)
    } else {
        None
    };
    let tr = match &tr_path {
        Some(p) => load_transcript_full(p)?,
        None => Default::default(),
    };

    if c.edit {
        let path = with_suffix(input, ".edit.txt");
        if path.exists() && !c.force {
            bail!(
                "{} already exists: edit it and run `flits cut {} --apply {}` (or --force to start over)",
                path.display(),
                input.display(),
                path.display()
            );
        }
        let doc = edit::EditDoc::new(&tr);
        if doc.lines.is_empty() {
            bail!("the transcript has no words to edit");
        }
        let tr_abs = PathBuf::from(templates::abs(tr_path.as_deref().unwrap()));
        std::fs::write(&path, doc.render(input, &tr_abs, &path))?;
        eprintln!(
            "flits: {} lines written. Delete lines or words (or wrap them in [[ ]]), save, then run:\n  flits cut {} --apply {}",
            doc.lines.len(),
            input.display(),
            path.display()
        );
        println!("{}", path.display());
        return Ok(());
    }

    let (words, preset) = match &edit_text {
        Some(text) => {
            let doc = edit::EditDoc::new(&tr);
            let a = edit::apply(&doc, text);
            for wn in &a.warnings {
                eprintln!("warning: {wn}");
            }
            eprintln!("flits: edit file keeps {} line(s), cuts {}", a.kept_lines, a.removed_lines);
            (doc.words, a.removed)
        }
        None => (tr.words(), vec![]),
    };
    let lang = if w.lang != "auto" { Some(w.lang.as_str()) } else { tr.language.as_deref() };
    let filler_list = match custom_fillers {
        Some(l) => l,
        None if want_fillers => fillers::for_language(lang),
        None => vec![],
    };
    if want_fillers {
        eprintln!("flits: filler words ({}): {}", lang.unwrap_or("unknown language"), filler_list.join(", "));
    }
    let o = cut::CutOpts {
        threshold_db: c.threshold,
        min_silence: c.min,
        keep: c.keep,
        silences: !c.no_silence,
        fillers: filler_list,
        repeats: c.repeats,
    };
    let removed = cut::removed_words(&words, &o, &preset).iter().filter(|r| **r).count();
    if removed > 0 {
        eprintln!("flits: cutting {removed} of {} words", words.len());
    }
    let ranges = cut::plan(input, info.duration, &o, &words, &preset)?;
    let kept: f64 = ranges.iter().map(|(s, e)| e - s).sum();
    eprintln!(
        "flits: keeping {} of {} ({} removed, {} cuts)",
        fmt_ts(kept),
        fmt_ts(info.duration),
        fmt_ts(info.duration - kept),
        ranges.len().saturating_sub(1)
    );
    if c.dry_run {
        for (s, e) in &ranges {
            println!("keep {} - {}  ({:.2}s)", fmt_ts(*s), fmt_ts(*e), e - s);
        }
        return Ok(());
    }
    let default_ext = if info.has_video { "mp4" } else { input.extension().and_then(|e| e.to_str()).unwrap_or("m4a") };
    let out = out.unwrap_or_else(|| with_suffix(input, &format!("-cut.{default_ext}")));
    if out == input {
        bail!("output would overwrite the input");
    }
    if !formats.is_empty() {
        let stem = input.file_stem().and_then(|s| s.to_str()).unwrap_or("cut");
        let tl = export::Timeline::new(export::Source::probe(input)?, &ranges, &format!("{stem} (cut)"));
        for f in &formats {
            let p = with_suffix(&out, f.suffix());
            std::fs::write(&p, export::render(&tl, *f)?)?;
            let hint = match f {
                export::Format::Fcpxml => "Final Cut Pro: File > Import > XML; DaVinci Resolve: File > Import > Timeline",
                export::Format::Premiere => "Premiere Pro: File > Import",
                export::Format::Edl => "DaVinci Resolve: File > Import > Timeline (add the source to the media pool first)",
                export::Format::Json => "cut list",
            };
            println!("{}", p.display());
            eprintln!("flits:   {hint}");
        }
        let fps = format!("{:.3}", tl.source.rate.fps());
        eprintln!(
            "flits: timeline of {} clip(s), {} at {} fps",
            tl.clips.len(),
            fmt_ts(tl.frames() as f64 / tl.source.rate.fps()),
            fps.trim_end_matches('0').trim_end_matches('.')
        );
        if !c.render {
            return Ok(());
        }
    }
    let t = Instant::now();
    if !info.has_video {
        cut::cut_audio(input, &ranges, &out)?;
        eprintln!("flits: done in {:.1}s -> {}", t.elapsed().as_secs_f64(), out.display());
        return Ok(());
    }
    // the cut soundtrack stays next to the output: the written scene refers to it
    let audio = with_suffix(&out, ".audio.m4a");
    cut::cut_audio(input, &ranges, &audio)?;
    let scene = cut::cut_video_scene(input, &ranges, &audio)?;
    write_scene_and_render(scene, &out, r)
}

pub fn transcribe(
    inputs: &[PathBuf],
    out: Option<PathBuf>,
    outdir: Option<PathBuf>,
    words_only: bool,
    segments_only: bool,
    w: &WhisperArgs,
) -> Result<()> {
    let mut o = w.opts()?;
    o.word_timestamps = !segments_only;
    if out.is_some() && inputs.len() > 1 {
        bail!("-o works with one input; use --outdir for several");
    }
    let jobs: Vec<(PathBuf, PathBuf)> = inputs
        .iter()
        .map(|i| {
            let o = match (&out, &outdir) {
                (Some(o), _) => o.clone(),
                (None, Some(d)) => d.join(transcribe::default_json(i).file_name().unwrap()),
                (None, None) => transcribe::default_json(i),
            };
            (i.clone(), o)
        })
        .collect();
    if let Some(d) = &outdir {
        std::fs::create_dir_all(d)?;
    }
    let t = Instant::now();
    transcribe::transcribe_many(&jobs, &o)?;
    if words_only {
        for (_, j) in &jobs {
            transcribe::to_words_only(j)?;
        }
    }
    eprintln!("flits: transcribed {} file(s) in {:.1}s", jobs.len(), t.elapsed().as_secs_f64());
    for (_, o) in &jobs {
        println!("{}", o.display());
    }
    Ok(())
}

#[derive(Clone)]
pub struct ReelArgs {
    pub windows: Vec<(f64, f64)>,
    pub format: String,
    pub fit: String,
    /// None: find the people in every shot
    pub focus: Option<f64>,
    pub title: Option<String>,
    pub preset: Option<String>,
    pub highlight: Option<String>,
    pub position: Option<f64>,
    pub fillers: bool,
    pub repeats: bool,
    pub no_silence: bool,
    pub keep: f64,
    pub fps: Option<f64>,
    pub punch: bool,
    pub music: Option<PathBuf>,
    pub logo: Option<PathBuf>,
    pub progress: bool,
    pub accent: Option<String>,
    /// end card: call to action
    pub end_text: Option<String>,
    /// end card: logo (an empty path = the `logo`)
    pub end_logo: Option<PathBuf>,
    pub end_duration: f64,
}

/// Who is where, shot by shot, over the parts of the footage the reel uses: (fraction of the
/// source width the crop shows, shots). Footage that isn't cropped sideways is analysed too: the
/// captions keep clear of the faces.
fn auto_shots(video: &Path, info: &media::MediaInfo, format: &str, windows: &[(f64, f64)]) -> Result<(f64, Vec<media::reframe::Shot>)> {
    let t = Instant::now();
    let (cw, ch) = templates::canvas_for(format)?;
    let visible = templates::reel::visible_fraction((cw as f64, ch as f64), (info.width as f64, info.height as f64));
    let mut all = vec![];
    for &(wa, wb) in windows {
        let (wa, wb) = (wa.max(0.0), wb.min(info.duration));
        if wb > wa {
            all.extend(media::reframe::analyse(video, wa, wb, info.fps)?);
        }
    }
    all.sort_by(|p, q| p.start.total_cmp(&q.start));
    eprintln!("flits: analysed {} shot(s) in {:.1}s", all.len(), t.elapsed().as_secs_f64());
    Ok((visible, all))
}

/// Framings for the analysed shots: two-shots whose people don't fit the crop together follow the
/// speaker (by the transcript's diarization labels when it has them, else by mouth movement).
fn auto_framings(shots: &[media::reframe::Shot], visible: f64, words: &[flits::compose::captions::Word]) -> Vec<media::reframe::Framing> {
    let f = media::reframe::framings(shots, visible, words);
    let extra = f.len().saturating_sub(shots.len());
    if extra > 0 && visible < 1.0 {
        eprintln!("flits: following the speaker in two-shots ({extra} extra cut(s))");
    }
    f
}

/// Raw footage -> reel: one transcript, one cut plan, one render of just the reel's length.
pub fn reel(video: &Path, out: Option<PathBuf>, a: &ReelArgs, transcript: Option<PathBuf>, w: &WhisperArgs, r: &RenderArgs) -> Result<()> {
    let t0 = Instant::now();
    let info = media::probe(video)?;
    if !info.has_video || !info.has_audio {
        bail!("{} needs both video and sound for a reel", video.display());
    }
    if let Some(p) = &a.preset {
        flits::scene::presets::caption_preset(p, (1080.0, 1920.0), None)?; // fail before transcribing
    }
    let out = out.unwrap_or_else(|| with_suffix(video, "-reel.mp4"));
    if out == video {
        bail!("output would overwrite the input");
    }
    let mut wo = w.opts()?;
    if (a.fillers || a.repeats) && wo.initial_prompt.is_none() {
        wo.initial_prompt = fillers::verbatim_prompt(&wo.language).map(str::to_string);
    }
    if transcript.is_none() && !transcribe::default_json(video).exists() {
        // start loading whisper now: it is ready by the time the audio is
        transcribe::prewarm(&wo);
    }
    let windows = if a.windows.is_empty() { vec![(0.0, info.duration)] } else { a.windows.clone() };
    // who is where needs only the picture: work it out while whisper listens
    let (tr_path, framings) = std::thread::scope(|s| {
        let framing_job = s.spawn(|| match a.focus {
            Some(_) => Ok(None),
            None => auto_shots(video, &info, &a.format, &windows).map(Some),
        });
        let tr = transcribe::ensure(video, transcript.as_deref(), &wo);
        (tr, framing_job.join().map_err(|_| anyhow!("reframing thread panicked")).and_then(|r| r))
    });
    let tr_path = tr_path?;
    let shots: Option<(f64, Vec<media::reframe::Shot>)> = framings?;
    let tr = load_transcript_full(&tr_path)?;
    let words = tr.words();
    let cropped = shots.as_ref().is_some_and(|s| s.0 < 1.0);
    let lang = if w.lang != "auto" { Some(w.lang.as_str()) } else { tr.language.as_deref() };
    let o = cut::CutOpts {
        threshold_db: -35.0,
        min_silence: 0.6,
        keep: a.keep,
        silences: !a.no_silence,
        fillers: if a.fillers { fillers::for_language(lang) } else { vec![] },
        repeats: a.repeats,
    };
    // next to the output: the written scene refers to them
    let audio = with_suffix(&out, ".audio.m4a");
    let words_path = with_suffix(&out, ".words.json");
    // the framing per shot (incl. reading mouths in two-shots) and the cut + soundtrack don't need
    // each other: side by side
    let (framings, ranges) = std::thread::scope(|s| {
        let framing_job = s.spawn(|| shots.map(|(visible, shots)| auto_framings(&shots, visible, &words)));
        let cut = || -> Result<Vec<(f64, f64)>> {
            let ranges = templates::reel::within(&cut::plan(video, info.duration, &o, &words, &[])?, &windows);
            let kept: f64 = ranges.iter().map(|(s, e)| e - s).sum();
            let chosen: f64 = windows.iter().map(|(s, e)| e.min(info.duration) - s).sum();
            eprintln!("flits: reel of {} from {} of footage ({} cuts)", fmt_ts(kept), fmt_ts(chosen), ranges.len().saturating_sub(1));
            cut::cut_audio(video, &ranges, &audio)?;
            Ok(ranges)
        };
        let ranges = cut();
        (framing_job.join().map_err(|_| anyhow!("reframing thread panicked")), ranges)
    });
    let (framings, ranges) = (framings?, ranges?);
    // no face anywhere (a static cover, slides, a landscape): don't crop it, show it whole
    let mut fit = a.fit.clone();
    if cropped && framings.as_ref().is_some_and(|f| !f.is_empty() && f.iter().all(|f| !f.face)) {
        eprintln!("flits: no faces found: showing the whole picture (--fit fit; --focus 0.5 crops anyway)");
        fit = "fit".into();
    }
    let backdrop = with_suffix(&out, ".backdrop.jpg");
    let backdrop = if fit == "fit" {
        let mid = ranges.first().map(|r| (r.0 + r.1) / 2.0).unwrap_or(0.0);
        let mut cmd = media::ffmpeg();
        cmd.args(["-y", "-ss", &format!("{mid:.3}")]).arg("-i").arg(video).args(["-frames:v", "1", "-vf", "scale=480:-2"]).arg(&backdrop);
        media::run(&mut cmd, "backdrop still").ok().map(|_| backdrop)
    } else {
        None
    };
    let end_still = with_suffix(&out, ".end.jpg");
    let end = end_card(video, a, &ranges, &end_still)?;
    templates::reel::write_words(&templates::reel::remap_words(&words, &ranges), tr.language.as_deref(), &words_path)?;
    let scene = templates::reel::reel(&templates::reel::ReelOpts {
        video,
        ranges: &ranges,
        audio: &audio,
        words: &words_path,
        format: a.format.clone(),
        focus: a.focus.unwrap_or(0.5),
        framings: framings.as_deref(),
        fit,
        backdrop: backdrop.as_deref(),
        fps: a.fps,
        punch: a.punch,
        music: a.music.as_deref(),
        logo: a.logo.as_deref(),
        progress: a.progress,
        accent: a.accent.clone(),
        title: a.title.clone(),
        preset: a.preset.clone(),
        highlight: a.highlight.clone(),
        position: a.position,
        end,
    })?;
    write_scene_and_render(scene, &out, r)?;
    eprintln!("flits: footage to reel in {:.1}s", t0.elapsed().as_secs_f64());
    println!("{}", out.display());
    Ok(())
}

/// The end card's logo and text from the arguments, with a still of the last frame (blurred on the card).
fn end_card<'a>(video: &Path, a: &'a ReelArgs, ranges: &[(f64, f64)], still: &'a Path) -> Result<Option<templates::reel::EndCard<'a>>> {
    let logo = match &a.end_logo {
        Some(p) if p.as_os_str().is_empty() => Some(
            a.logo.as_deref().ok_or_else(|| anyhow!("--end-logo without a path needs --logo (or name the logo: --end-logo logo.png)"))?,
        ),
        Some(p) => Some(p.as_path()),
        // a text card shows the brand's logo when there is one
        None if a.end_text.is_some() => a.logo.as_deref(),
        None => None,
    };
    let text = a.end_text.clone().filter(|t| !t.trim().is_empty());
    if logo.is_none() && text.is_none() {
        return Ok(None);
    }
    if !(0.5..=6.0).contains(&a.end_duration) {
        bail!("--end-duration: 0.5 to 6 seconds (1.5 to 2.5 reads well)");
    }
    let last = ranges.last().map(|r| (r.1 - 0.05).max(r.0)).unwrap_or(0.0);
    let mut cmd = media::ffmpeg();
    cmd.args(["-y", "-ss", &format!("{last:.3}")]).arg("-i").arg(video).args(["-frames:v", "1", "-vf", "scale=540:-2"]).arg(still);
    let still = media::run(&mut cmd, "end card still").ok().map(|_| still);
    Ok(Some(templates::reel::EndCard { text, logo, still, duration: a.end_duration }))
}

/// How `flits reel --auto` picks.
pub struct AutoArgs {
    /// target length (seconds)
    pub target: f64,
    /// reels to render
    pub candidates: usize,
    /// only print the candidates
    pub pick: bool,
    pub json: bool,
}

fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

/// `flits reel --auto` / `--pick`: rank the parts of the recording from its transcript, then print
/// them or render the best one(s) with `reel`.
pub fn reel_auto(
    video: &Path,
    out: Option<PathBuf>,
    a: &ReelArgs,
    auto: &AutoArgs,
    transcript: Option<PathBuf>,
    w: &WhisperArgs,
    r: &RenderArgs,
) -> Result<()> {
    if !(15.0..=90.0).contains(&auto.target) {
        bail!("--auto: a length of 15 to 90 seconds (30 to 75 works best for reels)");
    }
    if !(1..=5).contains(&auto.candidates) {
        bail!("--candidates: 1 to 5");
    }
    let info = media::probe(video)?;
    let mut wo = w.opts()?;
    if (a.fillers || a.repeats) && wo.initial_prompt.is_none() {
        wo.initial_prompt = fillers::verbatim_prompt(&wo.language).map(str::to_string);
    }
    // while whisper listens, find the shots and faces of the whole recording: whichever part gets
    // picked is then analysed already (and kept for the next reel from this footage)
    let transcribing = transcript.is_none() && !transcribe::default_json(video).exists();
    let tr_path = if transcribing && a.focus.is_none() && info.has_video && info.duration <= 20.0 * 60.0 {
        std::thread::scope(|s| {
            s.spawn(|| media::reframe::analyse_all(video, info.duration, info.fps));
            transcribe::ensure(video, transcript.as_deref(), &wo)
        })?
    } else {
        transcribe::ensure(video, transcript.as_deref(), &wo)?
    };
    let words = load_transcript_full(&tr_path)?.words();
    let n = if auto.pick { 5 } else { auto.candidates };
    let picks = templates::highlights::pick(&words, auto.target, n, info.duration);
    if picks.is_empty() {
        bail!("the transcript has no sentences to pick from");
    }
    if auto.json {
        let v: Vec<serde_json::Value> = picks
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let reasons: Vec<serde_json::Value> =
                    p.reasons.iter().map(|(pts, why)| serde_json::json!({"points": round2(*pts), "why": why})).collect();
                serde_json::json!({"rank": i + 1, "from": round2(p.from), "to": round2(p.to), "length": round2(p.length),
                    "score": round2(p.score), "first": p.first, "last": p.last, "reasons": reasons})
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&v)?);
        return Ok(());
    }
    for (i, p) in picks.iter().enumerate() {
        let why: String = p.reasons.iter().map(|(pts, why)| format!("    {pts:+.1}  {why}\n")).collect();
        let text = format!(
            "#{}  {} - {}  ~{:.0} s  score {:+.1}\n    \"{}\"\n    ... \"{}\"\n{why}    flits reel {} --from {:.2} --to {:.2}\n",
            i + 1,
            fmt_ts(p.from),
            fmt_ts(p.to),
            p.length,
            p.score,
            p.first,
            p.last,
            video.display(),
            p.from,
            p.to
        );
        if auto.pick {
            println!("{text}");
        } else {
            eprint!("flits: picked {text}");
        }
    }
    if auto.pick {
        return Ok(());
    }
    let out = out.unwrap_or_else(|| with_suffix(video, "-reel.mp4"));
    for (i, p) in picks.iter().enumerate() {
        let o = if picks.len() == 1 { out.clone() } else { with_suffix(&out, &format!("-{}.mp4", i + 1)) };
        let a = ReelArgs { windows: vec![(p.from, p.to)], ..a.clone() };
        reel(video, Some(o), &a, Some(tr_path.clone()), w, r)?;
    }
    Ok(())
}
