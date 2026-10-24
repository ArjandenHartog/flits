# flits API cheat sheet

Everything else: `flits spec` (full reference), `flits schema` (JSON Schema), the types in `flits/*.d.ts`.

## Project and commands

```sh
flits init my-video                     # project: src/video.ts, flits/ SDK (#flits), this skill, CLAUDE.md
flits check src/video.ts [--json]       # validate + layout lint (offscreen / overlapping / too-brief text) + timeline
flits still src/video.ts --at 1.2 --at 3.5 -o out/s.png      # PNGs (out/s-1.20s.png ...), paths printed one per line
flits sheet src/video.ts -o out/sheet.png [--count 12 --cols 4 --thumb 480]   # time-labelled contact sheet
flits sheet src/video.ts --from 2 --to 3 --count 8 -o out/t.png   # frames of one range (ends included)
flits sheet src/video.ts --at 1 --at 2.5 --at 4 -o out/t.png       # exactly these times
flits render src/video.ts -o out/video.mp4 [--scale 0.5] [--props '{"title":"Hi"}']
flits preview src/video.ts              # browser preview for humans (scrub/play, hot reload)
echo '[{"text":"Hi","font":"Segoe UI Black","size":120}]' | flits measure   # -> width, height, lines
flits new                               # list starter scenes (flits new kinetic-typography -o src/k.ts)
flits init --update                     # refresh flits/ SDK and this skill in an existing project
```

## Module shape

```ts
import { Composition, At, Sequence, Group, Text, Rect, Shape, Image, Video, Counter, Captions, Audio,
         defineVideo, formats, interpolate, spring, easing, random } from "#flits";

export default defineVideo(({ title = "Hello" }: { title?: string }) =>
  Composition({ ...formats.youtube, fps: 60, duration: 15, background: "#0b0b0f", motionBlur: true,
    loudness: -14, audio: [{ src: "assets/music.mp3", fadeOut: 2 }],
    fonts: { display: "../fonts/Inter-Black.ttf" },
    children: [ At({ time: 0, duration: 3, children: [ /* beat 1 */ ] }), /* ... */ ] }));
```

Components return plain layer objects; nest arrays, `null`/`false` children are dropped. `Sequence({ overlap,
children })` plays children back to back (each needs `duration`). `At({ time, duration, children })` = a
full-canvas group starting at `time`. Formats: `youtube` 1920x1080, `youtube4k`, `hd` 1280x720, `reel`/`short`/
`tiktok`/`story` 1080x1920, `square` 1080x1080, `portrait` 1080x1350.

## Common layer props

`start`, `duration` | `end` (s, relative to the parent) - `x`, `y` (anchor position in the parent box; default
centre) - `anchor` `[0.5, 0.5]` - `width`, `height` - `scale`, `scaleX`, `scaleY`, `rotation` (deg), `opacity` -
`in`, `out` (transitions) - `radius`, `border {width,color}`, `shadow {color,blur,x,y}` - `clip` (groups) -
`motionBlur: false` (opt out) - `id`, `comment`.

**Animatable** (`x y scale scaleX scaleY rotation opacity zoom panX panY value progress volume`):

```ts
x: 960                                                    // constant
x: { from: -200, to: 960, ease: "expo-out", start: 0, end: 0.8 }   // from/to (times relative to the layer)
y: [[0, 1200], [0.7, 540, "expo-out"], ["end-0.4", 540], ["end", 400, "expo-in"]]   // keyframes [t, v, ease]
scale: (t, { duration }) => spring({ t, damping: 12 })     // function of layer time, sampled per frame
```

## Layer types

| component | key props |
|---|---|
| `Text` | `text`, `font`, `weight`, `size`, `color`, `align`, `maxWidth`, `wrap: "balance"`, `lineHeight`, `letterSpacing`, `uppercase`, `stroke {width,color}`, `background {color,padding,radius,mode}`, `reveal` (`words`/`lines`/`chars`/`typewriter`), `revealStyle` (`rise`/`mask`/`fade`/`blur`), `revealDuration` |
| `Counter` | `value` (animatable), `decimals`, `prefix`, `suffix`, `separator` + text styling |
| `Rect` | `color` or `gradient: { type: "linear", angle: 180, stops: [[0, "#000"], [1, "#0000"]] }` / `{ type: "radial", colors: [...] }`, `radius` |
| `Shape` | `shape`: `rect` `ellipse` `circle` `line` `arrow` `polygon` `star` `path` (`d`), `points`, `color`/`gradient`, `stroke`, `progress` 0..1 (draw-on), `dash`, `lineCap`, `headSize` |
| `Image` | `src` (png/jpg/webp/svg/...), `fit` (`cover`/`contain`/`fill`), `zoom`, `panX`/`panY` (Ken Burns), `blur`, `filter` |
| `Video` | image props + `trim`, `speed`, `volume`, `loop`, `fadeIn`/`fadeOut` |
| `Captions` | `src` (transcript from `flits transcribe`), `preset` (`tiktok` `karaoke-box` `bounce` `minimal` `neon` `highlighter` `subtitle` `word-pop-big`), `style`, `highlight` |
| `Waveform` | `src`, `bars`, `color`, `mirror` |
| `Group` | `children`, `clip`, `sequence`, `overlap` |
| `Audio` | `src`, `start`, `trim`, `duration`, `volume`, `fadeIn`, `fadeOut`, `loop`, `duck` |

## Eases

`expo-out` (arrivals) - `expo-in` (exits) - `expo-in-out` (A to B) - `quint-out`, `quint-in-out`, `circ-out` -
`out`, `in`, `in-out` (cubic; default `in-out`), `smooth` - `back-out` (small pops), `back-in` - `spring`,
`elastic`, `bounce` (sparingly) - `linear` (drifts, progress bars) - `hold` (jump) -
`"cubic-bezier(0.16, 1, 0.3, 1)"`. In code: `easing["expo-out"](x)`, `cubicBezier(...)`, `interpolate(t, [0, 1],
[0, 100], { ease: "expo-out" })`.

## Transitions (`in` / `out`)

String `"slide-fade 0.6"` or `{ type: "slide-up", duration: 0.6, ease: "expo-out", distance: 80 }`. As `out`
they play mirrored at the end of the layer.

`fade` - `slide-up|down|left|right` (`distance`, default 120) - `slide-fade` (40 px rise; great for text) -
`zoom-in`, `zoom-out`, `zoom-fade`, `zoom-blur` (radial blur rush) - `pop` - `push-left|right|up|down` (whole parent
width; pair with `Sequence({ overlap })` of the same length) - `wipe-left|right|up|down` - `iris` - `rotate-in` -
`flip-x`, `flip-y`.

## Scene

`width`, `height`, `fps`, `duration`, `background`, `fonts` (aliases -> files), `fontFallback`, `motionBlur`
(`true` | `{ samples: 12, shutter: 0.5 }`), `loudness` (LUFS, -14 social), `audio` (tracks).
