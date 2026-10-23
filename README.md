# flits ⚡

[![ci](https://github.com/ArjandenHartog/flits/actions/workflows/ci.yml/badge.svg)](https://github.com/ArjandenHartog/flits/actions/workflows/ci.yml)
[![license: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

**Blazing-fast video and podcast rendering with built-in transcription.** Rust library + CLI.

Describe a video as JSON (layers, keyframe animation, transitions, word-by-word captions, audio
waveforms, counters) and flits composites it on the GPU, encodes it in parallel with hardware
encoders and joins the pieces without re-encoding. On top of that: Whisper transcription on the
GPU, beat-synced slideshows, silence and filler-word removal, podcast videos and animated captions.

## Speed (laptop: i7-11800H + RTX 3050 Ti)

| task | before | flits |
|---|---|---|
| 10,000 frames 1080p montage: 104 shots of photos, 26 HDR clips, text, shadows, music | ~30 min (Remotion) | **33–45 s** |
| the same scene without video clips | | **10 s** (~1,000 fps) |
| transcribing 1 h 33 min of Dutch speech (large-v3-turbo) | ~30 min (CPU) | **3.5 min** |
| removing silences from a 68 min podcast | | **28 s** |
| 2.5 min beat-synced 9:16 slideshow (18 photos/clips) | | **20 s** |

Measure it yourself with synthetic media (only ffmpeg needed):

```sh
python examples/benchmark.py && flits render bench/scene.json -o bench/out.mp4
FLITS_PROFILE=1 flits render bench/scene.json --encoder null   # renderer only, per-stage timings
```

## Commands

```sh
# scenes
flits render scene.json -o video.mp4          # render (--scale 0.5 = quick preview, --speed fast, --codec hevc)
flits sheet scene.json -o sheet.png           # contact sheet: check the whole video in one image
flits still scene.json --at 12.5 -o frame.png
flits check scene.json                        # validate (typos are reported) + print the timeline
flits new [intro|quote|stats|lower-third|countdown]   # starter scenes
flits spec                                    # full scene format reference

# ready-made
flits slideshow photos/ --music song.mp3 --beats --title "Summer 2026" --format 9:16
flits captions clip.mp4 --style pop           # burn in animated word-by-word captions
flits podcast episode.mp3 --cover cover.jpg --title "Episode 12" --format 9:16 --from 12:30 --to 13:30
flits cut recording.mp4 --fillers --repeats   # remove silences, uh/um (nl/en/de/fr/es/...) and stutters
flits cut recording.mp4 --export fcpxml,premiere,edl   # Final Cut Pro / Premiere / Resolve timeline instead
flits cut recording.mp4 --edit                # edit as text: delete lines/words in recording.edit.txt,
flits cut recording.mp4 --apply recording.edit.txt     #   then cut exactly that (word-accurate)
flits transcribe recording.m4a --lang en      # -> .words.json + .srt + .vtt + .txt
flits transcribe recording.m4a --initial-prompt "Arjan, flits" --vocab terms.txt   # names and jargon
flits transcribe interview.m4a --diarize      # speaker labels (optional pyannote, needs HF_TOKEN)

# tools
flits doctor                                  # what works on this machine (GPU, encoders, whisper, ...)
flits info file.mp4
flits measure < texts.json                    # text metrics for scripts that lay things out
flits serve --token secret                    # make this (GPU) machine a render/transcription server
flits remote http://gpu-box:7777 --token secret   # on another machine: offload work to it
```

As a library:

```rust
let stats = flits::render_file("scene.json".as_ref(), "out.mp4".as_ref(), &flits::Options::default())?;
```

The scene format is documented in [SPEC.md](SPEC.md) (`flits spec`), as are the cut options (edit files,
timeline exports, filler languages) and transcription options (prompts, vocabulary, diarization).

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

## Project layout

```
src/
  lib.rs          public API (Options, render_file, render_json, pick_gpu)
  cli/            the flits CLI (make.rs: commands that produce media, tools.rs: doctor/info/serve/...)
  scene/          the JSON format: structs, animation/easing/transitions, colours
  compose/        scene -> tree of prepared nodes (static content pre-rasterized);
                  draw.rs walks the tree per frame; captions, waveform
  paint/          drawing interface (Op/Painter) with CPU (tiny-skia) and GPU (wgpu + WGSL) backends
  render/         pipeline (chunks, workers, encoders, joining), FrameRenderer, stills/sheets, profiler
  media/          everything ffmpeg: probing, video decoding, encoders, audio mix, colour spaces, beat detection
  text/           font lookup/loading, layout (balanced wrapping, soft hyphens), glyphs -> paths
  gfx.rs          sprites, blur, shadows, rounded shapes
  transcribe/     faster-whisper via uv (CUDA when available), SRT/VTT/TXT
  remote/         `flits serve` and offloading to a GPU machine
  templates/      podcast, captions, slideshow, cut (+ fillers, edit files, editor exports), starters
tests/            CPU = GPU parity, end-to-end renders, audio, cut/edit/export
examples/         reproducible benchmark
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
