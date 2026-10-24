# {{name}}

A video project made with [flits](https://github.com/ArjandenHartog/flits): videos are TypeScript modules
rendered on the GPU (no browser). **For any video work here, use the `flits-video` skill**
(`.claude/skills/flits-video/`): it has the make-review-fix loop, motion design rules and the API.

## Conventions

- The video is `src/video.ts` (default export: `defineVideo(props => Composition(...))`). One component per
  beat in `src/` (functions returning layers); storyboard as a comment at the top of `src/video.ts`.
- The SDK is vendored in `flits/` and imported as `#flits`. Don't edit it; refresh it with `flits init --update`.
- Media in `src/assets/`, fonts in `fonts/` (referenced via `fonts` aliases). Paths are relative to the scene
  file (`src/`). Relative imports need their extension (`./title.ts`).
- Outputs go to `out/` (git-ignored): `out/video.mp4`, stills and sheets.

## Commands

```sh
flits check src/video.ts                         # validate + layout lint; fix every warning
flits still src/video.ts --at 1 --at 2.5 -o out/s.png   # frames to look at (read the PNGs)
flits sheet src/video.ts -o out/sheet.png        # the whole video in one time-labelled image
flits render src/video.ts -o out/video.mp4       # final render (seconds)
```
