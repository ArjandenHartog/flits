# flits ⚡

Bliksemsnelle video- en podcastrenderer met ingebouwde transcriptie. Rust-bibliotheek + CLI.

Je beschrijft een video als JSON (lagen, animaties, overgangen, ondertitels, audiogolf) en flits
rendert hem op de GPU, codeert parallel met de hardware-encoder en plakt de stukken zonder
hercoderen aan elkaar.

| | Remotion (headless Chrome) | flits |
|---|---|---|
| 10.000 frames 1080p (foto's, videoclips, tekst, schaduwen, muziek) | ~30 min | **65 s** |

## Waarom zo snel

* **Alles wat statisch is, wordt één keer gerasterd**: foto's worden eenmalig gedecodeerd, EXIF-gedraaid en
  verkleind tot de grootte waarop ze getoond worden; blur, schaduwen, tekst en verlopen worden vooraf berekend.
  Per frame blijven alleen transformaties over.
* **GPU-compositing (wgpu: Vulkan/D3D12/Metal)**: elke laag is één quad; afgeronde hoeken en anti-aliasing via
  een SDF in de shader. RGB→YUV 4:2:0 gebeurt in een compute-shader, dus de CPU raakt bijna geen pixel aan.
  Zonder GPU valt flits terug op een CPU-rasterizer (tiny-skia).
* **Parallel encoden**: de tijdlijn wordt in stukken geknipt, elk stuk krijgt een eigen worker + encoder
  (NVENC, Intel QSV/VAAPI of x264); daarna `concat` zonder hercoderen. Audio wordt tegelijk gemixt.
* **Video-decoding** via ffmpeg, direct op doelresolutie; iPhone-HDR wordt naar SDR getonemapt.

## Gebruik

```sh
flits render scene.json -o video.mp4          # renderen
flits render scene.json --scale 0.5           # snelle preview op halve resolutie
flits sheet scene.json -o sheet.png           # contactsheet: hele video in één plaatje checken
flits still scene.json --at 12.5 -o frame.png
flits check scene.json                        # valideren + tijdlijn tonen
flits spec                                    # volledige beschrijving van het scene-formaat

flits transcribe opname.m4a --lang nl         # -> .words.json + .srt + .vtt + .txt
flits captions clip.mp4 --style pop           # geanimeerde woord-voor-woord ondertitels inbranden
flits podcast aflevering.mp3 --cover cover.jpg --title "Aflevering 12" --format 9:16 --from 12:30 --to 13:30
```

Als bibliotheek:

```rust
let stats = flits::render_file("scene.json".as_ref(), "out.mp4".as_ref(), &flits::Options::default())?;
```

Zie [SPEC.md](SPEC.md) voor het scene-formaat en `examples/` voor voorbeelden.

## Vereisten

* `ffmpeg`/`ffprobe` op PATH
* voor transcriptie: [`uv`](https://docs.astral.sh/uv/) (installeert faster-whisper automatisch in een cache)
  of `--python` naar een Python met faster-whisper. Met een NVIDIA-GPU wordt CUDA gebruikt.

## Bouwen

```sh
cargo build --release
```
