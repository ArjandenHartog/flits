# flits ⚡

[![ci](https://github.com/ArjandenHartog/flits/actions/workflows/ci.yml/badge.svg)](https://github.com/ArjandenHartog/flits/actions/workflows/ci.yml)
[![license: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

> *Blazing-fast video and podcast renderer with built-in transcription: describe a video as JSON,
> flits composites it on the GPU and encodes it in parallel with hardware encoders. 10,000 frames
> of 1080p montage in well under a minute. Rust library + CLI. Docs are mostly Dutch; issues and
> PRs in English are welcome.*

Bliksemsnelle video- en podcastrenderer met ingebouwde transcriptie. Rust-bibliotheek + CLI.

Je beschrijft een video als JSON (lagen, animaties, overgangen, ondertitels, audiogolf, tellers)
en flits rendert hem op de GPU, codeert parallel met de hardware-encoder en plakt de stukken
zonder hercoderen aan elkaar. Daarnaast: transcriptie (Whisper op de GPU), slideshows op de beat,
stiltes en stopwoorden wegknippen, podcast-video's en geanimeerde ondertitels.

## Snelheid (laptop: i7-11800H + RTX 3050 Ti)

| taak | voorheen | flits |
|---|---|---|
| 10.000 frames 1080p, 104 shots met foto's, 26 HDR-videoclips, tekst, schaduwen, muziek | ~30 min (Remotion) | **33–45 s** |
| dezelfde scene zonder videoclips | | **10 s** (~1000 fps) |
| transcriptie 1 u 33 min (Nederlands, large-v3-turbo) | ~30 min (CPU) | **3,5 min** |
| stiltes uit een podcast van 68 min knippen | | **28 s** |
| slideshow 2,5 min (9:16, 18 foto's/clips, op de beat) | | **20 s** |

## Commando's

```sh
# scenes
flits render scene.json -o video.mp4          # renderen (--scale 0.5 = snelle preview, --speed fast, --codec hevc)
flits sheet scene.json -o sheet.png           # contactsheet: hele video in één plaatje checken
flits still scene.json --at 12.5 -o frame.png
flits check scene.json                        # valideren (typfouten worden gemeld) + tijdlijn
flits new [intro|quote|stats|lower-third|countdown]   # voorbeeldscène om mee te beginnen
flits spec                                    # volledige beschrijving van het scene-formaat

# kant-en-klaar
flits slideshow fotos/ --music liedje.mp3 --beats --title "Zomer 2026" --format 9:16
flits captions clip.mp4 --style pop           # geanimeerde woord-voor-woord ondertitels inbranden
flits podcast aflevering.mp3 --cover cover.jpg --title "Aflevering 12" --format 9:16 --from 12:30 --to 13:30
flits cut opname.mp4 --fillers                # stiltes (en uh/uhm/eh) eruit, audio of video
flits transcribe opname.m4a --lang nl         # -> .words.json + .srt + .vtt + .txt

# hulpmiddelen
flits doctor                                  # wat werkt er op deze pc (GPU, encoders, whisper, ...)
flits info bestand.mp4
flits serve --token geheim                    # deze (GPU-)pc als server voor andere pc's
flits remote http://laptop:7777 --token geheim   # op een andere pc: werk naar de GPU-pc sturen
```

Als bibliotheek:

```rust
let stats = flits::render_file("scene.json".as_ref(), "out.mp4".as_ref(), &flits::Options::default())?;
```

## Hoe het snel is

* **Statisch werk één keer**: foto's worden eenmalig gedecodeerd, EXIF-gedraaid en verkleind tot hun
  schermgrootte; blur, schaduwen, tekst en verlopen worden vooraf gerasterd. Per frame blijven alleen
  transformaties over.
* **GPU-compositing (wgpu: Vulkan/D3D12/Metal)**: elke laag is één quad; afgeronde hoeken en
  anti-aliasing via een SDF in de shader. Video komt als YUV-vlakken binnen: kleurconversie en
  HDR→SDR-tonemapping gebeuren in de shader. RGB→YUV 4:2:0 voor de encoder doet een compute-shader.
  Een aparte poll-thread voorkomt dat workers elkaar blokkeren op de GPU.
* **Parallel**: de tijdlijn wordt in chunks verdeeld die workers oppakken (work stealing); elke worker
  heeft een eigen hardware-encoder (NVENC, QSV, VAAPI) of x264. Segmenten worden zonder hercoderen
  samengevoegd; de audiomix (ducking, loudness) loopt tegelijk.
* **Video-decoding** loopt vooruit in een eigen thread en levert direct op doelresolutie.
* Zonder GPU valt flits terug op een CPU-rasterizer (tiny-skia) die exact dezelfde beelden maakt
  (getest: gemiddeld < 0,2/255 verschil).

`FLITS_PROFILE=1 flits render ...` laat zien waar de tijd heen gaat.

## Structuur

```
src/
  lib.rs          publieke API (Options, render_file, render_json, pick_gpu)
  cli/            de flits-CLI (make.rs: commando's die media maken, tools.rs: doctor/info/serve/...)
  scene/          het JSON-formaat: structs, animatie/easing/overgangen, kleuren
  compose/        scene -> boom van voorbereide nodes (alles statisch vooraf gerasterd);
                  draw.rs loopt per frame door de boom; captions, waveform
  paint/          tekeninterface (Op/Painter) met CPU-backend (tiny-skia) en GPU-backend (wgpu + WGSL)
  render/         pijplijn (chunks, workers, encoders, samenvoegen), FrameRenderer, stills/sheets, profiler
  media/          alles via ffmpeg: probe, video-decoder, encoders, audiomix, kleurruimtes, beat-detectie
  text/           fonts zoeken/laden, layout, glyphs -> paden
  gfx.rs          sprites, blur, schaduwen, afgeronde vormen
  transcribe/     faster-whisper via uv (CUDA als die er is), SRT/VTT/TXT
  remote/         `flits serve` en offloaden naar een GPU-pc
  templates/      podcast, captions, slideshow, cut, starters
tests/            pariteit CPU=GPU, end-to-end renders, audio
```

## Vereisten

* `ffmpeg`/`ffprobe` op PATH
* voor transcriptie: [`uv`](https://docs.astral.sh/uv/) (installeert faster-whisper automatisch in een cache)
  of `--python` naar een Python met faster-whisper. Met een NVIDIA-GPU wordt CUDA gebruikt; zonder GPU
  kan een andere pc het werk doen (`flits serve` / `flits remote`).

## Bouwen en testen

```sh
cargo build --release      # binary: target/release/flits
cargo test --release       # unit-, pariteits- en pipelinetests (GPU-tests slaan over zonder GPU)
```

Zelf meten: `python examples/benchmark.py && flits render bench/scene.json -o bench/out.mp4`
(synthetisch materiaal, alleen ffmpeg nodig).

## Meedoen

Bijdragen zijn welkom: zie [CONTRIBUTING.md](CONTRIBUTING.md) voor de opzet, de huisregels
(CPU = GPU, snelheid meten) en ideeën om mee te beginnen.

## Licentie

[MIT](LICENSE)
