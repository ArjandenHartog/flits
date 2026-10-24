# Benchmark: flits vs Remotion vs editly

`bench/run.py` renders one scene with each tool, times the whole CLI invocation, checks every
output with ffprobe (frame count, size) and writes `bench/results.json` plus a markdown table.

    python bench/run.py                       # 1080p, 1800 frames (60 s), every installed tool
    python bench/run.py --frames 10000        # the README's long montage
    python bench/run.py --frames 300 --tools flits,remotion --compare-at 1,5   # quick check + side-by-side stills

Only needs Python 3.9+, ffmpeg/ffprobe, and whichever tools you want to compare. Tools that are not
installed are skipped with the reason.

## The scene

`examples/benchmark.py` generates the media with ffmpeg (24 photos of 12 MP, six 1080x1920 phone clips
with sound, music) and a 1080p montage: full-screen photos with Ken Burns zoom and pan under a
gradient, "cards" (photo or clip) with border, rounded corners, drop shadow, rotation/scale keyframes
and a slide-in over a blurred copy of the photo, titles with shadow and slide-in, crossfades between
shots, a progress bar and a music track. Media files are reused between runs; only the scene changes
with `--frames`.

## What each tool renders

| tool | how | settings |
|---|---|---|
| flits | `flits render scene.json` | `default`: GPU compositor + hardware encoder (NVENC, VAAPI or QSV) at `--speed balanced`; `x264-crf18`: GPU compositor, x264 preset medium CRF 18; `cpu-only`: CPU compositor + x264 medium CRF 18 (no GPU at all) |
| Remotion | `remotion/`: `<FlitsScene>` interprets the same scene.json with React/CSS, the way you would write it by hand in Remotion | h264 (x264 CRF 18), JPEG frames; default concurrency and `--concurrency=<logical CPUs>` |
| editly | `editly/convert.mjs` converts the scene to an editly spec | x264 preset medium CRF 18 |

Fairness notes:

* **Same output check.** Every result must have the scene's frame count and resolution, otherwise it is
  reported as failed. Use `--compare-at 2,30` to put the same moments of all outputs side by side.
* **editly does less work.** Its model (clips with full-frame layers) cannot express everything; the
  converter drops card borders/shadows/radius/rotation, image blur and pan, gradient overlays, the
  progress bar and slide-in transitions, and lists what it dropped. Its time is a lower bound.
* **Remotion draws everything** except what the scene doesn't use (captions, counters, waveforms).
  Its `OffthreadVideo` decodes the clips with its own ffmpeg; Chrome Headless Shell renders the frames.
* **Warm caches.** The media are read once before timing; Remotion gets an untimed 1-frame warm-up
  render (downloads the browser, fills the webpack cache).
* Wall time includes everything the CLI does: probing media, decoding, rendering, encoding, audio, muxing.

## Setup

**flits:** `cargo build --release` (the runner uses `target/release/flits`, else `flits` on PATH, or `--flits`).

**Remotion:** `cd bench/remotion && npm ci` (Node 18+).

**editly:** its native modules (`gl`, `canvas`) need a Node version with prebuilt binaries or a C++ toolchain.
Either `cd bench/editly && npm ci` with a suitable Node (pass it with `--editly-node` or
`FLITS_BENCH_EDITLY_NODE`), or use the Docker image (Node 18, headless GL under Xvfb):

    docker build -t flits-bench-editly bench/editly
    python bench/run.py --editly docker       # default "auto": native when it loads, else docker

**GPU only reachable from a container** (e.g. a Linux server where your user is not in the `render`
group): build the flits image from the repository's `Dockerfile` and run flits in it with the host's
`/dev/dri` (Vulkan compositor + VAAPI encoder on Intel/AMD). Container start-up (~1 s) is included in
the time.

    docker build -t flits .
    python bench/run.py --flits-docker flits

## Output

* `bench/out/`: the videos, `logs/` per configuration, `compare-<t>s.jpg` stills;
* `bench/results.json`: machine (CPU, cores, RAM, GPU, OS), versions, commands, all timings and the
  ffprobe check per result;
* the markdown table on stdout (as used in the README).

`--runs 3` repeats every configuration and reports the median.
