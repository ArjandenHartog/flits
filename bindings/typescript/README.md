# flits TypeScript SDK

Write [flits](https://github.com/ArjandenHartog/flits) videos in TypeScript: components, motion helpers and complete
types for the scene format, plus a wrapper around the `flits` CLI. Dependency-free; Node 18+ (22.6+ to run `.ts`
directly), Bun or Deno.

The quickest start is a project made by the CLI, which vendors this SDK as `#flits`:

```sh
flits init my-video && flits preview my-video/src/video.ts
```

## Components

Functions that take the props of a layer type and return the layer; compose them like any function.

| component | layer |
|---|---|
| `Composition({ width, height, fps, duration, background, children })` | the scene (export it as default) |
| `Text({ text \| children, size, font, ... })` | text |
| `Image({ src, fit, zoom, ... })`, `Video({ src, trim, speed, volume, ... })` | media |
| `Rect`, `Shape({ shape: "circle" \| "arrow" \| "star" \| "path", progress, ... })` | boxes and vector shapes |
| `Counter({ value, suffix, ... })`, `Captions({ src, preset })`, `Waveform({ src })` | numbers, subtitles, audio bars |
| `Group({ children })`, `Sequence({ overlap, children })` (`Series`), `At({ time, children })` | timing and nesting |
| `Audio({ src, volume, duck })` | soundtrack (may sit among the layers) |

`children` may be nested arrays and contain `null`/`false` (`cond && Text(...)`). With
`"jsx": "react-jsx"` and `"jsxImportSource": "#flits"` (or this package) the same components work as JSX in Bun and
Deno: `<Sequence><Text duration={2}>Hi</Text></Sequence>`.

## Motion

Animatable props (`x`, `y`, `scale`, `rotation`, `opacity`, `zoom`, `value`, `progress`, `volume`, ...) take a number,
keyframes `[[t, value, ease?], ...]`, `{ from, to, ease, start, end }`, or a function of time that is sampled every frame:

```ts
Text({ text: "Hi", duration: 2,
       scale: (t) => spring({ t, damping: 12 }),
       x: (t, { progress }) => interpolate(progress, [0, 1], [200, 1720], { ease: "in-out" }) });
```

`interpolate(input, inRange, outRange, { ease, extrapolate })`, `spring({ t | frame, fps, stiffness, damping, mass,
delay })`, `easing.<name>(x)` (the scene format's curves), `random(seed)` (deterministic), `formats.youtube | reel |
square | ...`, `fromTo()`, `keyframes()`, `frames()`/`seconds()`.

## Motion design components

`motion.js` (exported from the package root too) turns the patterns of a polished brand film into one-call components
with designed defaults: `at` = when it comes in, `exit` = when it leaves, everything else from a theme set once with
`setTheme({ colors: { accent }, fonts: { display } })`.

* text: `MaskLine`, `KineticTitle`, `Eyebrow` (`Label`), `Typewriter`, `HighlightRule`, `Quote`
* structure: `Rule`, `Outline`, `Tag`, `Grid`, `Hud`, `LowerThird`, `EndCard`, `Backdrop`
* data: `StatCounter`, `BarChart`, `LineChart`, `ProgressRing`, `Checklist`, `ProgressBars`
* timing: `Act`, `timeline()`, `Stagger`, `Whip`
* theme: `setTheme`, `createTheme`, `getTheme`, `resetTheme`, `themeScene`, `defaultTheme`, `measureText`, `withAlpha`, `mix`

Every prop is documented in `motion.d.ts`; [`example-motion.ts`](example-motion.ts) is a complete film made with them.

## Parametrized videos

```ts
export default defineVideo(({ name }: { name: string }) => Composition({ ...formats.reel, duration: 3, children: Text({ text: `Hi ${name}` }) }));
```

`flits render video.ts --props '{"name": "Ada"}'` (or `--props props.json`).

## From your own code

`render(scene, "out.mp4", options)`, `still`, `sheet`, `check`, `transcribe`, `schema` run the CLI (`$FLITS_BIN` or
`flits` on PATH); `scene` is an object (compiled with `compile()` first) or a file path (`.json`, `.ts`, `.js`).

Tests: `node --test "test/*.test.js"`.
