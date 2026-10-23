# Contributing to flits

Thanks for helping! Issues and pull requests are welcome.

## Development setup

1. [Rust](https://rustup.rs) (stable) and `ffmpeg`/`ffprobe` on your PATH.
2. Optional: a GPU with Vulkan/D3D12/Metal (otherwise everything runs on the CPU backend and the
   GPU tests skip themselves), and [`uv`](https://docs.astral.sh/uv/) for transcription.
3. Build and test:

```sh
cargo build --profile fast        # quick iteration (no LTO)
cargo test --profile fast         # unit, parity and pipeline tests
cargo clippy --all-targets -- -D warnings
cargo fmt
flits doctor                      # what works on your machine
```

`cargo build --release` produces the optimized binary (fat LTO, takes a few minutes).

## Where things live

See **Project layout** in the [README](README.md). The main flow:

```
scene.json --(scene/)--> Scene --(compose/)--> Prepared (everything static pre-rasterized)
   per frame: compose/draw.rs --> paint::Op --> paint/gpu.rs (wgpu) or paint/cpu.rs (tiny-skia)
   render/: chunks -> workers -> encoders (media/encode.rs) -> join + audio mix (media/audio.rs)
```

## House rules

- **CPU and GPU draw the same.** Every visual feature works on both backends; `tests/parity.rs`
  compares the pixels. Extend that test when you draw something new.
- **Speed is a feature.** Measure before and after with `examples/benchmark.py` and
  `FLITS_PROFILE=1 flits render bench/scene.json --encoder null`, and put the numbers in your PR.
- **Static work once.** Anything that doesn't change per frame belongs in `compose/` preparation,
  not in per-frame code.
- **The scene format is the API.** Document new properties in [SPEC.md](SPEC.md); unknown fields are
  reported as warnings automatically (`serde_ignored`). Give new fields a doc comment (it becomes the
  description in the JSON Schema), regenerate `schema/scene.schema.json` with
  `cargo run --profile fast -- schema -o schema/scene.schema.json` and add the field to
  `bindings/typescript/scene.d.ts` and `bindings/python/flits/types.py` — `tests/schema.rs` fails until you do.
- No new runtime dependencies without a good reason; ffmpeg is the only external tool for rendering.
- English for code, comments, docs and CLI output.

## Pull requests

1. Fork and create a branch (`feature/…` or `fix/…`).
2. Keep PRs small and focused; one topic per PR.
3. Make sure `cargo fmt`, `cargo clippy -- -D warnings` and `cargo test` pass (CI checks this on
   Linux and Windows).
4. Describe what changed, why, and how you tested it (for visual changes, attach a `flits sheet` image).

## Good first ideas

- More transitions (e.g. zoom-blur, slide-push) and starters (`flits new`).
- A `path`/`shape` layer (lines, arrows, progress rings) without an SVG file.
- Linux/macOS: test VAAPI/QSV/Metal paths and document the results.
- A faster CPU backend for machines without a GPU.
