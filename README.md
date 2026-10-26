# flits ⚡

[![ci](https://github.com/ArjandenHartog/flits/actions/workflows/ci.yml/badge.svg)](https://github.com/ArjandenHartog/flits/actions/workflows/ci.yml)
[![license: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

**Blazing-fast video and podcast rendering with built-in transcription.** Rust library + CLI.

![flits demo: a title flying in with zoom blur, a progress ring and counter, an arrow drawing itself, emoji captions](docs/demo.gif)

<sub>Rendered by flits from [docs/demo.json](docs/demo.json) in 2.2 s (no GPU).</sub>

Describe a video as JSON (layers, keyframe animation with expo and cubic-bezier curves, real motion blur, masked text
reveals, 22 transitions, word-by-word captions with
ready-made looks, colour emoji in any language, audio waveforms, counters) and flits composites it on the GPU,
encodes it in parallel with hardware encoders and joins the pieces without re-encoding. On top of that: Whisper
transcription on the GPU, beat-synced slideshows, silence and filler-word removal, podcast videos and animated captions.

## Install

* **Binaries** for Linux (x86_64, glibc 2.35+), Windows (x86_64) and macOS (Apple silicon) are attached to every
  [release](https://github.com/ArjandenHartog/flits/releases): unpack and put `flits` on your PATH.
* **From source:** `cargo install --git https://github.com/ArjandenHartog/flits --locked` (Rust 1.88+).
* **Docker:** `docker build -t flits .` (see [below](#use-from-typescript--python--docker)).

flits needs `ffmpeg`/`ffprobe` on PATH; `flits doctor` shows what works on your machine (GPU, encoders, fonts,
transcription).

## Write videos in TypeScript

Like Remotion, a video can be code: components are functions, so you compose them, loop over data, use
conditions and share your own building blocks — with full type checking and autocompletion. flits runs the
module and renders what it exports, on the GPU, without a browser.

```sh
flits init my-video                              # TypeScript project: example video, a component, JSX example, typed SDK
flits preview my-video/src/video.ts              # live preview; save the file and it updates
flits render my-video/src/video.ts -o video.mp4 --props '{"title": "Hello"}'
```

```ts
import { Composition, Sequence, Text, Shape, Counter, Group, formats, spring, interpolate, defineVideo } from "#flits";
import stats from "./data.json" with { type: "json" };

function Stat({ label, value }: { label: string; value: number }) {
  return Group({ duration: 2, in: "zoom-blur", children: [
    Shape({ shape: "circle", width: 360, height: 360, stroke: { width: 24, color: "#f26328" }, progress: { from: 0, to: 1 } }),
    Counter({ value: { from: 0, to: value, ease: "out" }, size: 96 }),
    Text({ text: label, y: 800, size: 56 }),
  ] });
}

export default defineVideo(({ title = "Our year" }: { title?: string }) =>
  Composition({ ...formats.youtube, fps: 30, duration: 2 + stats.length * 2, children: [
    Sequence({ children: [
      Text({ text: title, size: 120, duration: 2,
             scale: (t) => spring({ t }),                       // any maths, sampled every frame
             opacity: (t) => interpolate(t, [0, 0.3], [0, 1]) }),
      ...stats.map((s) => Stat(s)),
    ] }),
  ] }),
);
```

* **Components**: `Composition`, `Text`, `Image`, `Video`, `Rect`, `Shape`, `Counter`, `Captions`, `Waveform`,
  `Group`, `Sequence`/`Series`, `At`, `Audio` — each takes the props of its layer type (typed, so typos are compile
  errors). They also work as JSX (`<Text size={80}>Hello</Text>`) with Bun or Deno.
* **Motion**: every animatable prop takes a number, keyframes, `{ from, to }` or a function `(t, { frame, fps,
  duration, progress }) => number`; flits samples functions once per frame. `interpolate()`, `spring()`, `easing`
  and a seeded `random()` work like Remotion's.
* **Props**: export a function and pass `--props` (inline JSON or a .json file) to render variants from one template.
* **Runtimes**: Bun, Deno or Node 22.6+ (whichever is installed, or `FLITS_JS`); only `.ts`/`.js` scenes need one.
  JSON scenes, the schema and everything else stay as they are — a TypeScript scene compiles to the same JSON.

### Motion design components

For the look of a polished brand film without writing keyframes, the SDK has a library of motion components
([`motion.js`](bindings/typescript/motion.js), typed and documented in `motion.d.ts`). Each is one call with good
defaults: `at` is when it comes in, `exit` when it leaves, and colours, fonts and timings come from one theme.

```ts
import { Composition, Act, Hud, KineticTitle, HighlightRule, LineChart, StatCounter, EndCard, Whip,
         setTheme, themeScene, timeline } from "#flits";

setTheme({ colors: { accent: "#ff4a2b" } });  // restyles everything; own fonts: fontFiles: { brand: "fonts/X.ttf" }, fonts: { display: "brand" }
const acts = timeline({ claim: 5, method: 5, end: 4 });                    // { claim: { at, end, duration }, ..., total }

export default Composition({ width: 1920, height: 1080, fps: 30, duration: acts.total, motionBlur: true, ...themeScene(),
  children: [
    Hud({ duration: acts.total, title: "Product film", chapters: [[0.5, "01 Claim"], [acts.method.at, "02 Method"]] }),
    Act({ ...acts.claim, children: KineticTitle({ text: "Motion design,\nwritten as code.", eyebrow: "New", rule: true, at: 0.5 }) }),
    Act({ ...acts.method, exit: "none", children: Whip({ exit: 4, children: [
      HighlightRule({ text: "Write it. Render it. Ship it.", x: 160, y: 400, size: 104,
                      marks: [["Write it.", 0.8], ["Render it.", 1.6], ["Ship it.", 2.4]] }),
    ] }) }),
    EndCard({ at: acts.end.at, duration: 4, title: "Make it move.", url: "example.com", fadeOut: true }),
  ] });
```

| group | components |
|---|---|
| text | `MaskLine` (a line sliding up out of its own mask), `KineticTitle` (multi-line, staggered, eyebrow + accent rule, exit), `Eyebrow`/`Label` (tracked mono capitals), `Typewriter` (with a cursor that follows), `HighlightRule` (an accent rule gliding under words), `Quote` |
| structure | `Rule` (drawing hairline), `Outline` (stroke-drawn box or corner brackets), `Tag` (outlined chip), `Grid`, `Hud` (rules, tick ruler, playhead, timecode, chapters), `LowerThird`, `EndCard`, `Backdrop` |
| data | `StatCounter`, `BarChart`, `LineChart` (draws itself, a dot riding the tip), `ProgressRing`, `Checklist`, `ProgressBars` |
| timing | `Act` (a chapter with an exit), `timeline()`, `Stagger`, `Whip` (whips out sideways; pair with `motionBlur`) |
| theme | `setTheme`, `createTheme`, `getTheme`, `themeScene` (scene `background` + `fonts`), `measureText` |

Rules under words, cursors and chips are sized from real text metrics: the SDK asks the running flits
(`flits measure`), so they line up exactly with the rendered glyphs. A complete 33 s film made only of these
components: [`bindings/typescript/example-motion.ts`](bindings/typescript/example-motion.ts).

## Making videos with Claude Code

flits is built to be driven by coding agents: an agent can't watch a video, but it can render any frame in about a
second and look at the PNG. The `flits-video` skill teaches Claude Code (and other agents that read `SKILL.md`
skills) that loop, plus the motion design rules that make a video look produced rather than generated.

```sh
flits init launch-video          # a project with the skill (.claude/skills/flits-video) and a CLAUDE.md
cd launch-video && claude        # then ask, e.g.:
#   "Make a 30 s 1080p launch video for Acme Notes: hook, three features, CTA. 60 fps, dark, one orange accent."
flits skill install              # add the skill to an existing project (--global: for all your projects)
flits skill print                # or read it / paste it into another agent
```

The loop the skill follows: pick the format and fps, storyboard the beats with timings, write `src/video.ts`, then

```sh
flits check src/video.ts --json                                    # errors, typos and layout lint: text off the
                                                                   # canvas, text over text, text too brief to read
flits still src/video.ts --at 0.4 --at 3.2 --at 7.5 -o out/s.png   # mid-motion and at-rest frames (prints the paths)
flits sheet src/video.ts --from 2.8 --to 3.6 --count 8 -o out/t.png   # one transition, every tile time-labelled
flits render src/video.ts -o out/video.mp4                         # seconds, not minutes
```

reading every PNG for clipping, overlap, alignment, contrast and timing, and fixing until a full contact sheet looks
finished. A 15 s 1080p60 launch video made this way (`flits skill print` includes it as `example.ts`) renders in
4.1 s on a laptop RTX 3050 Ti.

## Speed (laptop: i7-11800H + RTX 3050 Ti)

| task | before | flits |
|---|---|---|
| 10,000 frames 1080p montage: 104 shots of photos, 26 HDR clips, text, shadows, music | ~30 min (Remotion) | **33–45 s** |
| the same scene without video clips | | **10 s** (~1,000 fps) |
| transcribing 1 h 33 min of Dutch speech (large-v3-turbo) | ~30 min (CPU) | **3.5 min** |
| removing silences from a 68 min podcast | | **28 s** |
| 2.5 min beat-synced 9:16 slideshow (18 photos/clips) | | **20 s** |

### flits vs Remotion vs editly

The same 1080p scene (generated by `examples/benchmark.py`: photos with Ken Burns, cards with border, shadow and
keyframes over blurred backgrounds, phone clips, titles, crossfades, music) rendered by each tool, wall time of the
whole CLI run, every output checked with ffprobe. Harness, settings and fairness notes: [bench/](bench/README.md).

Intel Core i5-7500 (4 cores) with its integrated **Intel HD 630**, Ubuntu 24.04; 1800 frames (60 s) 1080p30; flits
in its Docker image with `/dev/dri` (Vulkan on the iGPU + VAAPI); every tool at low priority on a shared server
(`--nice`), one run each, all outputs verified:

| tool | settings | wall time | fps | vs flits |
|---|---|---:|---:|---:|
| **flits** | iGPU compositor + VAAPI encoder | **21.0 s** | **86** | 1× |
| flits | iGPU compositor + x264 medium CRF 18 (Remotion's/editly's encoder settings) | 58.4 s | 31 | 2.8× |
| flits | no GPU at all: CPU compositor + x264 medium CRF 18 | 83.6 s | 22 | 4.0× |
| Remotion 4.0.538 | `--concurrency=4`, x264 CRF 18 | 199.6 s | 9 | 9.5× |
| Remotion 4.0.538 | default concurrency | 219.2 s | 8 | 10.4× |
| editly 0.14.2 | x264 medium CRF 18; draws less (no card borders, shadows, blur, pan, slide-ins) | 384.2 s | 5 | 18.3× |

With the same encoder settings flits is 3.4× faster than Remotion here, and still 2.4× faster with no GPU at all.
On the RTX laptop above the 10,000-frame montage takes 33–45 s.

Reproduce (only ffmpeg needed for flits; Remotion and editly are optional):

```sh
cargo build --release
python bench/run.py --frames 10000                # all installed tools; --tools flits for flits only
FLITS_PROFILE=1 flits render bench/media/scene.json --encoder null   # renderer only, per-stage timings
```

## Commands

```sh
# scenes
flits render scene.json -o video.mp4          # render (--scale 0.5 = quick preview, --speed fast, --codec hevc)
flits preview scene.json                      # live preview in the browser, reloads when you save the scene
flits sheet scene.json -o sheet.png           # contact sheet: check the whole video in one image (tiles labelled
                                              # with their time; --at 1 --at 2.5, or --from 2 --to 3 --count 8)
flits still scene.json --at 12.5 -o frame.png
flits sheet clip.mov -o sheet.png             # sheet/still also take a plain video or image (shown full frame,
                                              # rotation and HDR handled like a video layer)
flits check scene.json                        # validate (typos are reported), lint the layout, print the timeline
                                              # (--json: one object with warnings, layout issues and the layer tree)
flits skill install                           # the flits-video skill for Claude Code (--global for all projects)
flits init my-video                           # TypeScript project (components, springs, data; see below)
flits new kinetic-typography                  # starter scenes (-o x.ts for TypeScript): intro, quote, stats, lower-third, countdown, youtube-intro,
                                              # podcast-quote, product-promo, news-headline, end-screen, kinetic-typography
flits spec                                    # full scene format reference
flits schema                                  # JSON Schema of the scene format (also in schema/scene.schema.json)

# ready-made
flits slideshow photos/ --music song.mp3 --beats --title "Summer 2026" --format 9:16
flits reel interview.mp4 --from 1:12 --to 2:10 --fillers --title "Hook"   # raw footage -> 9:16 reel in one go:
                                              # transcribe once, cut silences/uh's, follow the faces shot by shot,
                                              # captions, hook title, one render (1 min reel from 1080p: ~30 s)
flits reel interview.mp4 --pick               # rank the parts that work as a reel (whole sentences, a hook, a clean end)
flits reel interview.mp4 --auto 45 --candidates 3 --logo logo.png --end "Follow for more"   # render the 3 best, end card
flits captions clip.mp4 --preset tiktok       # burn in animated word-by-word captions; presets: tiktok, karaoke-box,
                                              # bounce, minimal, neon, highlighter, subtitle, word-pop-big
flits podcast episode.mp3 --cover cover.jpg --title "Episode 12" --format 9:16 --from 12:30 --to 13:30
flits cut recording.mp4 --fillers --repeats   # remove silences, uh/um (nl/en/de/fr/es/...) and stutters
flits cut recording.mp4 --export fcpxml,premiere,edl   # Final Cut Pro / Premiere / Resolve timeline instead
flits cut recording.mp4 --edit                # edit as text: delete lines/words in recording.edit.txt,
flits cut recording.mp4 --apply recording.edit.txt     #   then cut exactly that (word-accurate)
flits transcribe recording.m4a --lang en      # -> .words.json + .srt + .vtt + .txt
flits warm                                    # load whisper now: the next transcription starts at once
flits transcribe recording.m4a --initial-prompt "Arjan, flits" --vocab terms.txt   # names and jargon
flits transcribe interview.m4a --diarize      # speaker labels (optional pyannote, needs HF_TOKEN)

# tools
flits doctor                                  # what works on this machine (GPU, encoders, whisper, ...)
flits info file.mp4
flits info flyby.wav --peaks                  # loudness over time: the loudest moment (RMS, --window 0.25 s),
                                              # the loudest sample and dBFS per second (--json for scripts)
flits measure < texts.json                    # text metrics for scripts that lay things out
flits serve --token secret                    # make this (GPU) machine a render/transcription server
flits remote http://gpu-box:7777 --token secret   # on another machine: offload work to it
```

`flits preview` serves a page on `http://127.0.0.1:7878/` (`--port`, `--scale 0.5`, `--gpu cpu`,
`--no-open`) with a scrubber, play/pause in real time, frame stepping (arrow keys), a thumbnail strip and
a problems panel. Frames are rendered on demand on the GPU; decoders stay open between frames, so playing
and scrubbing through video clips is fast. The scene file and every file it references (media, transcripts,
fonts) are watched: save and the page updates; errors appear in the page while the last good version stays
on screen.

As a library:

```rust
let stats = flits::render_file("scene.json".as_ref(), "out.mp4".as_ref(), &flits::Options::default())?;
```

The scene format is documented in [SPEC.md](SPEC.md) (`flits spec`), as are the cut options (edit files,
timeline exports, filler languages) and transcription options (prompts, vocabulary, diarization).

## Use from TypeScript / Python / Docker

A scene is plain JSON, so anything that can write JSON and start a process can drive flits. The format has a
**JSON Schema** ([`schema/scene.schema.json`](schema/scene.schema.json), or `flits schema`), generated from the Rust
structs and tested against them, with descriptions and enums for every property. Use it for editor completion
(`"$schema": "https://raw.githubusercontent.com/ArjandenHartog/flits/main/schema/scene.schema.json"` at the top of a
scene), for validation in CI, or as the output format for an LLM (structured output / tool parameters). `flits check`
remains the authority: it also catches what a schema can't (missing files, fonts, video length).

**TypeScript** ([`bindings/typescript`](bindings/typescript), dependency-free, Node ≥ 18, Bun, Deno): the SDK of
[TypeScript videos](#write-videos-in-typescript) — complete types for the scene format, components, motion helpers —
plus a small wrapper around the CLI to render, check and transcribe from your own code.

```ts
import { check, reel, render, type Scene } from "flits-video"; // npm i flits-video, or a relative import of bindings/typescript/index.js

const scene: Scene = {
  width: 1080, height: 1920, duration: 4,
  layers: [{ type: "text", text: "Hello", size: 120, in: "pop", out: "fade 0.4" }],
};
const { ok, warnings, error } = await check(scene);
await render(scene, "hello.mp4", { scale: 0.5 });  // also: still, sheet, transcribe, reel, schema
const [best] = await reel("talk.mp4", null, { pick: true });             // ranked parts: from, to, score, first, reasons
await reel("talk.mp4", "reel.mp4", { from: best.from, to: best.to, end: "Follow for more" });
```

**Python** ([`bindings/python`](bindings/python), `pip install flits-video`, no dependencies on 3.11+):
`TypedDict`s for the scene format and the same wrapper (`render`, `still`, `sheet`, `check`, `transcribe`, `reel`, `schema`);
animatable properties may be Python callables `f(t, ctx)`, sampled per frame like in TypeScript.
See [`examples/hello.py`](bindings/python/examples/hello.py).

```python
import flits

scene = {"width": 1080, "height": 1920, "duration": 4,
         "layers": [{"type": "text", "text": "Hello", "size": 120, "in": "pop"}]}
print(flits.check(scene))                     # CheckResult(ok=True, warnings=[], ...)
flits.render(scene, "hello.mp4", scale=0.5, echo=True)
words_json = flits.transcribe("talk.m4a", lang="en")[0]
best = flits.reel("talk.mp4", pick=True)[0]   # ranked parts (flits.types.ReelCandidate)
flits.reel("talk.mp4", "reel.mp4", from_=best["from"], to=best["to"], fillers=True, end="Follow for more")
```

Both wrappers find the CLI via `$FLITS_BIN` or `PATH`. A scene object is written to a temporary file in `baseDir` /
`base_dir` (default: the current directory), so relative media paths resolve against that folder.

**Docker** (Linux image with ffmpeg, fonts, Vulkan and VAAPI drivers; `flits` is the entrypoint):

```sh
docker build -t flits .
docker run --rm -v "$PWD:/work" flits render scene.json -o out.mp4                  # CPU compositor + x264
docker run --rm --gpus all -v "$PWD:/work" flits render scene.json -o out.mp4       # NVIDIA: Vulkan + NVENC
docker run --rm --device /dev/dri -v "$PWD:/work" flits render scene.json -o out.mp4 # Intel/AMD: Vulkan + VAAPI
docker run --rm -v "$PWD:/work" -v flits-cache:/cache flits transcribe talk.m4a --lang en
```

The NVIDIA variant needs the NVIDIA Container Toolkit on the host. Add `--user "$(id -u):$(id -g)"` to get output
files owned by you (with `--device /dev/dri` also `--group-add "$(stat -c %g /dev/dri/renderD128)"`); the `/cache`
volume keeps whisper models between runs.

## GPUs and encoders

`flits doctor` reports the compositor GPU and the encoders that work. `--encoder auto` (default) tries NVENC, then
VAAPI (Linux), then QSV, then x264; `--gpu cpu` / `--encoder cpu` force the software paths (same pixels).

| platform | compositor | encoder | notes |
|---|---|---|---|
| Windows | D3D12 / Vulkan | NVENC, QSV | QSV via the Intel driver (Media SDK / oneVPL) |
| Linux, NVIDIA | Vulkan | NVENC | proprietary driver; in Docker via the NVIDIA Container Toolkit |
| Linux, Intel / AMD | Vulkan (Mesa) | VAAPI | your user needs the `render` group (`sudo usermod -aG render $USER`, log in again); `FLITS_VAAPI_DEVICE` picks a render node (default: the first one you can open) |
| Linux, Intel Tiger Lake+ | Vulkan (Mesa) | QSV or VAAPI | QSV needs Intel's oneVPL GPU runtime (`libmfx-gen1`); older Intel GPUs (Broadwell..Ice Lake) have VAAPI only |
| macOS | Metal | x264 | (VideoToolbox is not wired up yet) |

Measured on Linux with an Intel HD 630 (Kaby Lake, Mesa 25 + iHD 24 in the Docker image): Vulkan compositor +
`h264_vaapi` 110 fps vs 71 fps with x264 and 30 fps with no GPU at all (1080p montage, 300 frames); `h264_qsv`
does not start on Kaby Lake (no oneVPL runtime for that generation), which `flits render` now explains.
VAAPI uses constant QP = `--quality` + 4, about the size and quality of x264 at that CRF.

## Why it is fast

* **Static work happens once.** Photos are decoded, EXIF-rotated and downscaled to their on-screen
  size once; blur, shadows, text, gradients and SVGs are pre-rasterized. Per frame only transforms remain.
* **GPU compositing (wgpu: Vulkan / D3D12 / Metal).** Every layer is one instanced quad; rounded
  corners and anti-aliasing come from a signed-distance function in the shader. Video arrives as YUV
  planes; colour conversion and HDR→SDR tone mapping happen in the shader, and a compute shader produces
  the YUV 4:2:0 frames for the encoder. A dedicated poll thread keeps workers from blocking each other.
* **Hardware video decoding** (NVDEC, VAAPI) for HEVC/AV1/VP9/H.264 sources, including rotated phone
  footage, with automatic software fallback; decoders run ahead in their own threads and are reused across
  cuts of the same clip.
* **Parallel encoding.** The timeline is split into chunks that workers pull (work stealing); each worker
  has its own hardware encoder (NVENC, QSV, VAAPI) or x264. Segments are joined without re-encoding while
  the soundtrack (ducking, loudness normalization) is mixed in parallel.
* **No GPU? Same pixels.** A CPU rasterizer (tiny-skia) produces matching frames (tested: < 0.2/255 mean difference).

## Text

Any installed font by name, a .ttf/.otf/.ttc file or a scene alias; variable font weights; balanced wrapping, soft
hyphens, outlines, shadows, background pills, word/line/character reveals. Characters a font lacks fall back to the
scene's `fontFallback`, then the system's fallback fonts, then any installed font that has them, so mixed scripts
just work. **Emoji are drawn in colour** (COLR v0/v1 and CBDT/sbix fonts: Segoe UI Emoji, Noto Color Emoji, Apple
Color Emoji, Twemoji) and shaped, so 👍🏽 🇳🇱 1️⃣ 👨‍👩‍👧 come out as one emoji; Hebrew, Arabic and Indic scripts are
shaped too, and right-to-left paragraphs run right to left.

## Project layout

```
src/
  lib.rs          public API (Options, render_file, render_json, pick_gpu)
  cli/            the flits CLI (make.rs: commands that produce media, tools.rs: doctor/info/serve/...)
  scene/          the JSON format: structs, animation/easing/transitions, colours, JSON Schema;
                  script.rs runs TypeScript/JavaScript scenes (Bun, Deno or Node)
  compose/        scene -> tree of prepared nodes (static content pre-rasterized);
                  draw.rs walks the tree per frame; captions, waveform
  paint/          drawing interface (Op/Painter) with CPU (tiny-skia) and GPU (wgpu + WGSL) backends
  render/         pipeline (chunks, workers, encoders, joining), FrameRenderer, stills/sheets, profiler
  media/          everything ffmpeg: probing, video decoding, encoders, audio mix, colour spaces, beat detection
  text/           font lookup/loading and fallback, layout (balanced wrapping, soft hyphens, RTL),
                  glyphs -> paths, colour emoji (COLR/CBDT/sbix), shaping of fallback runs (rustybuzz)
  gfx.rs          sprites, blur, shadows, rounded shapes
  transcribe/     faster-whisper via uv (CUDA when available), SRT/VTT/TXT
  remote/         `flits serve` and offloading to a GPU machine
  preview/        `flits preview`: live browser preview with hot reload (page in index.html)
  templates/      podcast, captions, slideshow, cut (+ fillers, edit files, editor exports), starters,
                  project/ (the `flits init` TypeScript project)
tests/            CPU = GPU parity, end-to-end renders, audio, preview, cut/edit/export, schema
examples/         benchmark scene generator
bench/            flits vs Remotion vs editly harness (run.py, Remotion interpreter, editly converter)
schema/           scene.schema.json (generated by `flits schema`, checked by tests/schema.rs)
bindings/         TypeScript SDK (components, motion helpers, types, JSX runtime, CLI wrapper), Python package
Dockerfile        Linux image: flits + ffmpeg + fonts + Vulkan and VAAPI drivers
```

## Requirements

* `ffmpeg` / `ffprobe` on PATH
* for transcription: [`uv`](https://docs.astral.sh/uv/) (installs faster-whisper into a cache automatically)
  or `--python` pointing at a Python with faster-whisper. CUDA is used when an NVIDIA GPU is present;
  machines without one can offload to another machine (`flits serve` / `flits remote`).
* optional, for `--diarize`: a Hugging Face token in `HF_TOKEN` (pyannote.audio is installed through uv on first use)

## Building and testing

```sh
cargo build --release      # binary: target/release/flits
cargo test --release       # unit, parity and pipeline tests (GPU tests skip themselves without a GPU)
```

## Contributing

Contributions are welcome — see [CONTRIBUTING.md](CONTRIBUTING.md) for the setup, the house rules
(CPU = GPU, measure speed) and ideas to start with.

## License

[MIT](LICENSE)
