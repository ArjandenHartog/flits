# Bijdragen aan flits / Contributing

Fijn dat je wilt helpen! Issues en pull requests zijn welkom, in het Nederlands of Engels.
*Issues and pull requests are welcome in Dutch or English.*

## Ontwikkelomgeving

1. [Rust](https://rustup.rs) (stable) en `ffmpeg`/`ffprobe` op je PATH.
2. Optioneel: een GPU met Vulkan/D3D12/Metal (anders draait alles op de CPU-backend en slaan
   de GPU-tests zichzelf over), en [`uv`](https://docs.astral.sh/uv/) voor transcriptie.
3. Bouwen en testen:

```sh
cargo build --profile fast        # snelle iteratie (geen LTO)
cargo test --profile fast         # unit-, pariteits- en pipelinetests
cargo clippy --all-targets -- -D warnings
cargo fmt
flits doctor                      # wat werkt er op jouw machine
```

`cargo build --release` maakt de geoptimaliseerde binary (fat LTO, duurt een paar minuten).

## Waar zit wat

Zie het overzicht onder **Structuur** in de [README](README.md). De belangrijkste stroom:

```
scene.json --(scene/)--> Scene --(compose/)--> Prepared (alles statisch voor-gerasterd)
   per frame: compose/draw.rs --> paint::Op --> paint/gpu.rs (wgpu) of paint/cpu.rs (tiny-skia)
   render/: chunks -> workers -> encoders (media/encode.rs) -> samenvoegen + audiomix (media/audio.rs)
```

## Regels van het huis

- **CPU en GPU tekenen hetzelfde.** Elke nieuwe visuele feature werkt op beide backends;
  `tests/parity.rs` vergelijkt de pixels. Breid die test uit als je iets nieuws tekent.
- **Snelheid is een feature.** Meet voor en na met `examples/benchmark.py` en
  `FLITS_PROFILE=1 flits render bench/scene.json --encoder null`. Zet de cijfers in je PR.
- **Statisch werk één keer.** Alles wat niet per frame verandert, hoort in `compose/` voorbereid
  te worden, niet in de per-frame-code.
- **Het scene-formaat is de API.** Nieuwe properties documenteer je in [SPEC.md](SPEC.md);
  onbekende velden worden automatisch als waarschuwing gemeld (`serde_ignored`).
- Geen nieuwe runtime-afhankelijkheden zonder goede reden; ffmpeg is de enige externe tool
  voor renderen.
- Code en commentaar in het Engels, gebruikersteksten (CLI-uitvoer, docs) mogen Nederlands zijn.

## Pull requests

1. Fork en maak een branch (`feature/…` of `fix/…`).
2. Houd PR's klein en gericht; één onderwerp per PR.
3. Zorg dat `cargo fmt`, `cargo clippy -- -D warnings` en `cargo test` slagen (CI controleert dit
   op Linux en Windows).
4. Beschrijf wat je veranderde, waarom, en hoe je het getest hebt (bij visuele wijzigingen graag
   een `flits sheet`-plaatje).

## Ideeën om mee te beginnen

- Meer overgangen (bijv. zoom-blur, slide-push) en starters (`flits new`).
- Een `path`/`shape`-laag (lijnen, pijlen, voortgangscirkels) zonder SVG-bestand.
- Hardware-decoding voor videoclips waar het formaat dat toelaat (NVDEC/VAAPI).
- Linux/macOS: testen met VAAPI/QSV en Metal en de bevindingen documenteren.
- Snellere CPU-backend voor machines zonder GPU.
