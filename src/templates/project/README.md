# {{name}}

A video written in TypeScript with [flits](https://github.com/ArjandenHartog/flits).

```sh
flits preview src/video.ts                    # live preview in the browser; reloads when you save
flits render src/video.ts -o out/video.mp4    # render (GPU + hardware encoder when available)
flits render src/video.ts --props '{"title": "Hello"}'   # props for the exported function
flits sheet src/video.ts -o out/sheet.png     # 12 frames in one image
npm install && npm run typecheck              # optional: type-check with tsc
```

* `src/video.ts`: the video. Its default export is a scene, or a function of the props returning one.
* `src/components.ts`: your own components (functions returning layers).
* `src/hello.tsx`: the same in JSX (Bun or Deno).
* `CLAUDE.md` and `.claude/skills/flits-video/`: instructions for Claude Code (and other coding agents) to make,
  review and render videos here. Ask it for a video.
* `flits/`: the flits SDK (components, `interpolate`, `spring`, types), imported as `#flits`. Update it
  with `flits init --update`.

flits runs `.ts` files with Bun, Deno or Node 22.6+ (whichever is installed, or `FLITS_JS`). Relative
imports need their extension (`./components.ts`). Media paths are relative to the scene file you render
(`src/`). The scene format is documented in `flits spec`; animatable properties also accept `(t) => number`.
