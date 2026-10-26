# flits scene format

A scene is one JSON file. flits renders it to MP4 (H.264/HEVC/AV1 + AAC):

    flits render scene.json -o out.mp4        # full render (GPU compositor + hardware encoder when available)
    flits render scene.json --scale 0.5       # fast half-resolution preview
    flits preview scene.json                  # live preview in the browser (scrub, play); reloads on save
    flits sheet scene.json -o sheet.png       # 12 frames in one image: check the whole video at a glance
                                              # (--at 1 --at 2.5: these times; --from 2 --to 3 --count 8: a range)
    flits still scene.json --at 12.5 -o f.png # single frame(s); prints the written paths, one per line
    flits sheet clip.mov -o sheet.png         # sheet and still also take a plain video or image: a scene of its
                                              # display size, fps and length with one full-frame layer
    flits check scene.json                    # validate (typos are reported), lint the layout, print the timeline
    flits check scene.json --json             # the same as one JSON object (ok, size, warnings, issues, layers)
    flits render video.ts --props '{"t": 1}'  # TypeScript/JavaScript scenes (see below); `flits init` starts a project
    flits new                                 # list example scenes; `flits new intro -o intro.json`
    flits schema                              # JSON Schema of this format (also schema/scene.schema.json)
                                              # (intro, quote, stats, lower-third, countdown, youtube-intro, podcast-quote,
                                              #  product-promo, news-headline, end-screen, kinetic-typography)

Units: pixels (canvas coordinates, origin top-left) and seconds. Relative file
paths resolve against the scene file's folder.

**JSON Schema.** `flits schema` prints a JSON Schema (draft 2020-12) generated from the
same structs flits parses, with a description for every property and enums for layer
types, eases, transitions, captions styles, `fit`, `align`, `wrap` and `reveal`. Point a
scene at it for editor completion and validation:

```jsonc
{ "$schema": "https://raw.githubusercontent.com/ArjandenHartog/flits/main/schema/scene.schema.json",
  "width": 1920, "height": 1080, "layers": [] }
```

`"$schema"` (at the top level) and `"comment"` (on any object) are ignored by flits and
allowed by the schema; every other unknown key is a typo (a warning in `flits check`, an
error in the schema). The schema is stricter than flits in two places: enum values are
lowercase only (flits ignores case), and it cannot see files, fonts or media lengths —
`flits check` remains the final word.

**Layout lint.** `flits check` also samples the timeline (transforms only, no pixels) and warns about text or counters
that run off the canvas for 0.5 s or more (entrances, exits and text inside a clipping group or wipe are fine), text that
overlaps other text for 0.3 s or more (not when the upper one sits on its own `background` or on an opaque rect drawn
between them), two texts at the same position (same anchor point) on screen together for less than 0.5 s — subtitles
whose end runs a frame past the next one's start, even on their own backgrounds (not with an `out`/`in` transition or
animated opacity: a crossfade) — and text visible for less than ~0.1 s + 0.2 s per word. Each warning names the layer
and the time range: `warning: 1.00-3.00s: text "Big headline" runs 140 px past the right edge (move it in, make it
smaller or set maxWidth)`, `warning: 13.81-13.86s: layers 12 and 13 (text) overlap on screen for 0.05 s at 13.81 s: ...`.
With `--json` they are in `issues` (`kind`: `offscreen` | `overlap` | `flash` | `brief`, `layer`, `other`, `from`, `to`,
`message`) next to `warnings` (typos), the canvas, `duration`, `frames`, `audio` and the `layers` tree (`type`,
`label`, `start`, `end`, `width`, `height`, `children`). A scene that fails to load prints `{"ok": false, "error": ...}`
and exits with an error.

## Scene

```jsonc
{
  "width": 1920, "height": 1080, "fps": 30,
  "duration": 60,                      // optional: defaults to the end of the last layer
  "background": "#101014",
  "fonts": { "brand": "fonts/Rubik-Black.ttf" },   // aliases usable in "font"
  "fontFallback": ["Noto Color Emoji"],  // optional: fonts for characters a layer's font lacks
  "loudness": -14,                     // optional: normalize the final mix (LUFS; -14 social, -16 podcast)
  "motionBlur": true,                  // optional: motion blur for moving layers (or { "samples": 12, "shutter": 0.5 })
  "audio": [ { "src": "music.mp3", "volume": 0.6, "fadeIn": 1, "fadeOut": 3, "duck": true } ],
  "layers": [ ... ]                    // drawn in order: first = bottom
}
```

**Emoji and other scripts.** Characters a layer's font lacks are drawn with the first font that has them:
the scene's `fontFallback` (names, files or aliases), then the system's fallback fonts (Windows: Segoe UI Emoji,
Segoe UI Symbol, Nirmala UI, Microsoft YaHei, Yu Gothic, Malgun Gothic, ...; macOS: Apple Color Emoji, PingFang,
Hiragino; Linux: DejaVu Sans, Noto Sans, Noto Color Emoji, Noto Sans CJK, ...), then any installed font that has
the character. Emoji are drawn in colour (COLR v0/v1 and CBDT/sbix bitmap fonts) and shaped, so sequences such as
👍🏽, 🇳🇱, 1️⃣ and 👨‍👩‍👧 come out as one emoji. Hebrew, Arabic and the Indic scripts are shaped too; a text whose first
letter is right-to-left is laid out right to left. Colour glyphs keep their own colours (no `color`/`stroke`), the
text shadow applies to them. A character that no font has (it would be drawn as a box) is reported once per layer by
`flits check`, `render`, `still`, `sheet` and `preview`, e.g.
`warning: layers.3 (text): no font has a glyph for U+26A0 '⚠' (font: seguibl.ttf); it is drawn as a box: add a font
that has it to "fontFallback"` (text, counter and captions layers; joiners, variation selectors and other characters
that draw nothing on their own are not reported).

Audio track: `src`, `start` (scene time), `trim` (skip seconds of the source),
`duration`, `volume` (number or keyframes, see Animation), `fadeIn`, `fadeOut`, `loop`,
`duck` (`true` or a level like `0.2`: automatically lowers this track while other audio —
voice, video sound — plays). Audio from video layers is mixed in automatically
(`"volume": 0` on a video mutes it).

## Common layer properties

| key | meaning |
|---|---|
| `type` | `rect`, `image`, `video`, `text`, `counter`, `captions`, `waveform`, `shape`, `group` |
| `start`, `end` / `duration` | lifetime in seconds (relative to the parent group) |
| `x`, `y` | position of the anchor point (default: centre of the parent) |
| `width`, `height` | box size (default: parent size; text sizes itself; image/video with one side given keep their aspect ratio) |
| `anchor` | `[0.5,0.5]` = centre (default), `[0,0]` = top-left, `[1,1]` = bottom-right |
| `scale`, `scaleX`, `scaleY`, `rotation` (degrees), `opacity` | animatable |
| `in`, `out` | enter/exit transition (see below) |
| `radius` | corner radius (radius ≥ half the size = circle/pill) |
| `border` | `{ "width": 14, "color": "#fff" }` (drawn outside the box) |
| `shadow` | `{ "color": "rgba(0,0,0,0.5)", "blur": 24, "x": 0, "y": 12 }` (text: text shadow; others: box shadow) |
| `filter` | image/video colour grading: `{ "brightness": 0.05, "contrast": 1.1, "saturation": 1.2, "warmth": 0.3, "grayscale": false }` |

### Animation

Any animatable number can be:

* a constant: `"opacity": 0.8`
* from/to over the whole layer: `"zoom": { "from": 1.0, "to": 1.15, "ease": "linear" }` (optional `start`, `end`)
* keyframes `[time, value, ease]`: `"y": [[0, 1200], [0.6, 540, "back-out"], ["end-0.5", 540], ["end", -200, "in"]]`
  (time in seconds from layer start, or `"end"`/`"end-0.5"`; the ease applies to the segment ending at that key)

Eases: `linear`, `in`, `out`, `in-out` (default), `smooth`, `back-in`, `back-out`, `elastic`, `bounce`, `spring`, `hold`,
`expo-in`, `expo-out`, `expo-in-out`, `quint-out`, `quint-in-out`, `circ-out`, or a CSS curve `"cubic-bezier(0.16, 1, 0.3, 1)"`.
`expo-out` is the snappy "lands and settles" curve of most motion design.

### Motion blur

`"motionBlur": true` on the scene blurs whatever moves, like a camera shutter: every layer that travels more than a
pixel during the open shutter is drawn at several moments around the frame time and averaged. The number of samples
follows the distance moved (one per 2 px, up to `samples`, default 8), so still layers cost nothing and fast ones get
smooth trails. `{ "samples": 12, "shutter": 0.5 }`: `shutter` is the fraction of the frame interval the shutter is
open (0.5 = a 180° film shutter; 1 = the whole frame). Movement counts from position, scale, rotation, transitions
and text reveals; a moving group is sampled as a whole. `"motionBlur": false` on a layer exempts it and its children
(e.g. a timecode that must stay crisp).

### Transitions (`in` / `out`)

`"in": "fade"`, `"in": "pop 0.4"`, or `{ "type": "slide-up", "duration": 0.6, "ease": "out", "distance": 200 }`.
As `out`, every transition plays mirrored (it leaves the way it came, continuing the motion).

| type | effect |
|---|---|
| `fade` | opacity |
| `slide-up`, `slide-down`, `slide-left`, `slide-right` | moves `distance` px (default 120) while fading |
| `slide-fade` | a soft rise: `distance` (default 40) px up with the opacity following the ease; nice for text |
| `zoom-in`, `zoom-out` | scales from 85% / 115% while fading |
| `zoom-fade` | scales from 125% (exit: to 125%) while crossfading, like flying through the layer |
| `zoom-blur` | rushes in from 130% with a radial (zoom) blur that settles as it stops (exit: blurs out to 130%) |
| `pop` | scales up from 0 with overshoot (default ease `back-out`) |
| `push-left`, `push-right`, `push-up`, `push-down` | slides in from fully outside the **parent box** (moves exactly one parent width/height, no fade). An `out` and an `in` of the same length line up seamlessly: in a `sequence` with `"overlap"` equal to the duration, the next layer pushes the previous one out |
| `rotate-in` | turns from `-angle` degrees (default 90; exit: to `+angle`) while scaling up from 50% and fading |
| `flip-x`, `flip-y` | card flip: `scaleX` (`scaleY`) grows from 0 |
| `wipe-left`, `wipe-right`, `wipe-up`, `wipe-down` | reveal from one edge (clip) |
| `iris` | circular reveal growing from the centre until it covers the corners (exit: shrinks to the centre) |

Defaults: duration 0.5 s (`pop` 0.45, `push-*` and `iris` 0.6), ease `out` (`wipe-*`, `push-*` and `iris`: `in-out`, a
symmetric curve). Crossfade = overlap two layers and give the top one `"in": "fade"`. Transitions only change transform,
opacity and clip, so they work identically on the GPU and CPU renderers.

## Layer types

**rect** — `color` or `gradient`:
`{ "type": "linear", "angle": 180, "stops": [[0, "#000000"], [1, "#00000000"]] }` or
`{ "type": "radial", "colors": ["#f26328", "#120a1c"] }` (angle like CSS: 180 = top to bottom).

**image** — `src` (jpg/png/webp/gif/bmp; HEIC/AVIF via ffmpeg), `fit`: `cover` (default) | `contain` | `fill`,
`zoom` + `panX`/`panY` (−1..1) for Ken Burns inside the box, `blur` (px, cheap: done once), `filter`.
EXIF rotation is applied automatically; images are downscaled once to the size they are shown at.

**video** — like image plus `trim` (start offset in the source), `speed` (0.5 = slow motion, 2 = fast),
`volume` (default 1, 0 = mute, or keyframes), `fadeIn`/`fadeOut` (audio), `loop`, `filter`.
Default duration = the (re-timed) clip length. iPhone HDR (HLG/PQ, 10-bit) is tone-mapped on the GPU.
When `zoom`/`panX`/`panY` don't change over the layer, only the part of the picture that shows is decoded
(16:9 footage in a 9:16 frame: a third of it). To jump between framings, use one layer per framing
(contiguous `trim`s in a `sequence` group) rather than `hold` keyframes: they share one decoder.

**text** — `text` (`\n` for line breaks), `font` (installed font name like `"Segoe UI Black"`, a .ttf/.otf path or a
`fonts` alias; default Segoe UI Bold), `weight` (for variable fonts), `variations` (any axes of a variable font:
4-letter tag -> value, e.g. `{"wdth": 75, "opsz": 36, "slnt": -8}`; applied to outlines, advances, shaping and colour
glyphs; `weight` wins over a `"wght"` here; axes the font lacks are ignored with a warning; fallback fonts keep their
default instance; also accepted by `flits measure` and on counter and captions layers), `size` (64), `color`, `align`
(`left`/`center`/`right`), `maxWidth` (wraps), `lineHeight` (1.15), `letterSpacing`, `uppercase`,
`stroke` `{width,color}`, `background` `{ "color": "#000a", "padding": [24,12], "radius": 16, "mode": "box"|"lines" }`,
`reveal`: `"words"` | `"lines"` | `"chars"` | `"typewriter"` with `revealDuration` (seconds over which the pieces appear)
and `revealStyle`: `"rise"` (default: up a third of its height while fading in), `"mask"` (each piece slides up out of
its own line box, clipped, on an expo-out curve: the classic kinetic-type reveal), `"fade"`, or `"blur"` (rises while
a vertical smear sharpens).

**counter** — an animated number: `value` (animatable, e.g. `{"from": 0, "to": 12840, "ease": "out", "end": 2.5}`),
`decimals`, `prefix`, `suffix`, `separator` (thousands, default `","` → `1,234.5`; `"."` gives `1.234,5`; `"none"` gives `1234,5`),
plus the text styling keys. Digits are tabular, so the number doesn't jitter.

**captions** — animated word-by-word subtitles from a transcript (`flits transcribe` output, or any
faster-whisper / WhisperX JSON / .srt / .vtt): `src`, `trim` (transcript time that maps to the layer start),
`style`: `karaoke` (active word highlighted) | `pop` (words pop in as spoken, TikTok-style) | `bounce` (words drop in
with a bounce as spoken) | `word` (one big word at a time) | `line` (plain),
`highlight` (active word colour, default yellow), `colors` (list of active-word colours, cycled word by word),
`activeScale` (size of the active word: default 1.12 for `pop`, 1 otherwise),
`highlightBg` (box behind the active word) with `highlightBox`: `pill` (default; glides from word to word) | `marker`
(a highlighter stroke that wipes in), `background` (box behind the whole chunk, same keys as for text:
`{ "color", "padding", "radius", "mode": "box" | "lines" }`, `lines` = one box per line),
`maxWords` per chunk, `maxLines` per chunk (default 2; a chunk ends before it would wrap further),
`speakerColors` (`{ "SPEAKER_00": "#ffffff", "SPEAKER_01": "#ffd60a" }`: word colour per diarization speaker; a
`speaker` field on words or segments is read from WhisperX-style transcripts, and a new speaker starts a new chunk),
`size`, `font` (default Segoe UI Black), `color`, `stroke`, `shadow` (a fully transparent colour turns the default
shadow off), `uppercase`, `lineHeight`, `wrap: "balance"`, `width` (wrap width).

Grouping and timing of the chunks (all optional; the defaults are the behaviour without them):

- `maxChars` per chunk (spaces included; default 9 per word of `maxWords`, at least 12). A word ending in `.`, `?`,
  `!` or `:` always ends a chunk, as do a pause over 0.6 s and a new speaker.
- `breaks`: `greedy` (default; a full chunk breaks as late as fits) | `phrase`: a full chunk breaks after its last
  comma, otherwise never after a `noBreakAfter` word, and a lone trailing word joins the chunk before it (up to 8
  characters over `maxChars`). `noBreakAfter` (case-insensitive word list, e.g. `["de", "het", "een", "van", "met"]`)
  and `language` (`nl` | `en` | `auto` = the transcript's language: a built-in list of articles, prepositions and
  conjunctions) imply `phrase`.
- `minDuration` (s, e.g. 0.9): a chunk spoken faster than this merges with a neighbour (not across a sentence end, a
  pause over 0.2 s or a new speaker, and only when the result fits `maxLines`); also the shortest time a chunk stays up.
- `lead` (s a chunk appears before its first word, default 0), `hold` (s it stays after its last word, default 0.25),
  `gapClose` (a gap to the next chunk shorter than this is closed, default 0.6), `appear` (s of fade/scale-in,
  default 0.12; 0 = at once). Chunks never overlap: each ends at the latest when the next one appears.

`flits srt scene.json -o out.srt` writes the chunks of all captions layers as SRT, timed exactly as rendered.
One subtitle at a time on a coloured box per line, the way per-subtitle text layers would draw it:

```json
{ "type": "captions", "src": "talk.words.json", "style": "line", "y": 1450, "width": 880, "size": 56,
  "lineHeight": 1.38, "wrap": "balance", "maxChars": 32, "language": "nl", "minDuration": 0.9, "lead": 0.05,
  "appear": 0, "shadow": { "color": "#00000000" },
  "background": { "color": "#f07d00", "padding": [22, 6], "radius": 14, "mode": "lines" } }
```

`preset` picks a ready-made look; any property set on the layer overrides the preset's value (objects such as
`stroke` merge key by key), and outline/shadow sizes follow the layer's own `size`:

| preset | look |
|---|---|
| `tiktok` | bold uppercase words that pop in, yellow active word, thick black outline |
| `karaoke-box` | white words, the active word on a purple pill that glides along |
| `bounce` | uppercase words that drop in with a bounce, active word in cycling colours |
| `minimal` | clean white sentence-case lines (balanced) with a soft shadow |
| `neon` | glowing cyan uppercase words, the active word lights up pink |
| `highlighter` | white words, the active word turns dark on a yellow marker stroke |
| `subtitle` | classic two-line subtitles on semi-transparent boxes per line (set `y` near the bottom) |
| `word-pop-big` | one huge word at a time, colours cycling |

```json
{ "type": "captions", "src": "talk.words.json", "preset": "tiktok", "y": 1400, "highlight": "#4ade80" }
```

**waveform** — live audio spectrum bars: `src` (audio/video file), `trim`, `bars` (48), `color`, `mirror`
(bars grow from the centre line), `gap` (0..1 of a slot), `gain`, `radius`.

**shape** — vector shapes without an SVG file, filled with `color` or `gradient` and/or outlined with
`stroke` `{width,color}`. `shape`: `rect` (default; `radius` rounds it), `ellipse` (fills the box), `circle`,
`line` and `arrow` (from the left to the right middle of the box, or through `points` `[[x,y], ...]` in the layer
box; stroked in `color`, default 8 px, unless `stroke` is set), `polygon` (`sides`, default 6, or `points`), `star`
(`sides` points, default 5; `innerRadius` 0.5), `path` (SVG path data in `d`, in layer-box units: `"M 0 80 Q 100 0
200 80"`). Closed shapes are filled (white by default) unless only `stroke` is given. `lineCap`: `round` (default) |
`butt` | `square`; `dash`: `[on, off]` in px; `headSize`: arrow head length (default 4× the stroke width).
`progress` (0..1, animatable) draws the outline on, starting at the top of circles/ellipses and running clockwise —
a progress ring is a circle with a stroke and `"progress": {"from": 0, "to": 1}`; an arrow's head follows the drawn
end. Shapes are rasterized once (per `progress` value), so they cost no more per frame than an image.

```json
{ "type": "shape", "shape": "arrow", "points": [[0, 120], [180, 20], [400, 60]], "width": 400, "height": 140,
  "stroke": { "width": 10, "color": "#ffd60a" }, "progress": { "from": 0, "to": 1, "end": 0.8, "ease": "out" } }
```

**group** — `children`: layers positioned/timed relative to the group. Group transforms, opacity and
transitions apply to all children. `"clip": true` hides everything outside the group's box.
`"sequence": true` plays children one after another (each needs a `duration`, clips use their length);
`"overlap": 0.5` overlaps consecutive children (use with `"in": "fade"` for crossfades).

## Example

```json
{
  "width": 1080, "height": 1920, "fps": 30, "loudness": -14,
  "audio": [{ "src": "music.mp3", "volume": 0.8, "duck": 0.25, "fadeOut": 2 }],
  "layers": [
    { "type": "image", "src": "bg.jpg", "blur": 40, "zoom": { "from": 1.1, "to": 1.25 } },
    { "type": "rect", "color": "rgba(0,0,0,0.35)" },
    { "type": "group", "sequence": true, "overlap": 0.4, "children": [
      { "type": "image", "src": "a.jpg", "duration": 4, "height": 1100, "radius": 24,
        "border": { "width": 12, "color": "#fff" }, "shadow": { "blur": 40, "y": 24 },
        "rotation": [[0, -6], [0.7, -2, "back-out"]], "in": "pop", "filter": { "warmth": 0.3 } },
      { "type": "video", "src": "clip.mp4", "duration": 5, "height": 1100, "radius": 24, "speed": 0.5, "in": "fade" }
    ]},
    { "type": "text", "text": "Zomertrip 2026", "y": 260, "size": 96, "font": "Segoe UI Black",
      "shadow": { "blur": 20, "y": 6 }, "reveal": "chars" },
    { "type": "captions", "src": "voice.words.json", "y": 1500, "style": "pop", "size": 84,
      "highlight": "#ffd60a", "stroke": { "width": 8, "color": "#000" } }
  ]
}
```

## TypeScript and JavaScript scenes

`flits render`, `preview`, `still`, `sheet` and `check` also take a `.ts`, `.mts`, `.tsx`, `.js`, `.mjs` or `.jsx` file.
flits runs it with Bun, Deno or Node 22.6+ (whichever is installed; `FLITS_JS` picks one; JSX needs Bun or Deno) from
the nearest folder with a package.json or tsconfig.json, and renders the scene its default export describes:

* a scene object (exactly this format), or
* a function `(props) => scene` (may be async), called with `--props` (inline JSON or a .json file; default `{}`).

On the way, animatable properties (`x`, `y`, `scale`, `scaleX`, `scaleY`, `rotation`, `opacity`, `zoom`, `panX`,
`panY`, `value`, `progress`, `volume`) may be functions `(t, { frame, fps, duration, progress }) => number`, with `t`
in seconds since the layer started; they are sampled once per frame into linear keyframes, so the layer (or its group,
or the scene) needs a known duration. `layers`/`children` may be nested arrays with `null`/`false` entries (dropped),
and `Audio` elements among them move to `audio`. Relative media paths resolve against the scene file's folder; the
script's `console.log` output goes to stderr. `flits preview` reloads when any source or .json file in the scene's
folder changes.

`flits init <dir>` creates a project with the SDK in `flits/` (imported as `#flits`; `flits init --update` refreshes
it): components (`Composition`, `Text`, `Image`, `Video`, `Rect`, `Shape`, `Counter`, `Captions`, `Waveform`, `Group`,
`Sequence`, `Series`, `At`, `Audio`, `Fragment`), `formats` (youtube, reel, square, ...), `interpolate`, `spring`,
`easing`, `random`, `fromTo`, `keyframes`, `defineVideo`, the scene types, and a JSX runtime. `flits new <starter> -o
x.ts` writes a starter as a TypeScript module.

**Motion components** (`motion.js`, also exported from `#flits`) build whole motion-design elements from these layers.
Timing props are shared: `at` (seconds, relative to the parent, when it starts coming in) and `exit` (when it starts
leaving; omitted = stays until the parent ends); `x`/`y` are the top-left (or top-centre/right with `align`).
Text: `MaskLine`, `KineticTitle`, `Eyebrow`/`Label`, `Typewriter`, `HighlightRule`, `Quote`. Structure: `Rule`,
`Outline`, `Tag`, `Grid`, `Hud`, `LowerThird`, `EndCard`, `Backdrop`. Data: `StatCounter`, `BarChart`, `LineChart`,
`ProgressRing`, `Checklist`, `ProgressBars`. Timing: `Act` (a full-frame chapter from `at` for `duration` with an
`exit` of `rise`/`sink`/`fade`/`whip`/`none`), `timeline({ name: seconds, ... })` (acts end to end: `{ name: { at, end,
duration }, total }`), `Stagger` (shifts children's `start` by `each`), `Whip`. Colours (`background`, `surface`,
`text`, `muted`, `subtle`, `line`, `accent`, `positive`, `negative`), fonts (`display`, `body`, `mono`), weights,
`fontFiles`, motion defaults (`in` 0.9 s `expo-out`, `out` 0.55 s `expo-in`, `stagger` 0.12) and canvas size come from
the theme: `setTheme({...})` once, or a `theme` prop per component; `themeScene()` gives the scene's `background` and
`fonts`. Components that align to text (`HighlightRule`, `Typewriter`, `Tag`, `Eyebrow` markers, `KineticTitle` with
`maxWidth`) measure it with `flits measure` (flits sets `FLITS_BIN` for the scripts it runs; results are cached in the
temp folder; without flits the SDK estimates).

## Ready-made commands

    flits slideshow fotos/ --music liedje.mp3 --beats --title "Zomer 2026" --format 9:16
    flits captions clip.mp4 --preset tiktok          # burn animated captions into a video (or --style pop)
    flits podcast aflevering.mp3 --cover cover.jpg --title "Titel" --format 9:16 --from 12:30 --to 13:30
    flits cut opname.mp4 --fillers                   # remove silences (and uh/uhm) from audio or video
    flits transcribe opname.m4a --lang nl            # -> .words.json + .srt + .vtt + .txt

`slideshow`, `captions`, `podcast` and `cut` (video) also write `<out>.scene.json`; edit it and run
`flits render` to tweak.

## Reels: `flits reel`

    flits reel interview.mp4                              # whole recording -> 9:16 reel
    flits reel interview.mp4 --from 1:12 --to 2:10 --fillers --title "Waarom dit ertoe doet"
    flits reel interview.mp4 --clips "0:40-1:05,3:10-3:32"     # several parts, in this order
    flits reel interview.mp4 --focus 0.3 --format 4:5     # fixed framing, other format
    flits reel interview.mp4 --fit fit                    # whole picture, captions underneath
    flits reel interview.mp4 --logo logo.png --accent "#f39200" --progress --music bed.mp3   # branded
    flits reel interview.mp4 --pick                       # rank the parts that work as a reel, render nothing
    flits reel interview.mp4 --auto 45 --candidates 3     # render the 3 best parts: <out>-1.mp4, -2, -3
    flits reel interview.mp4 --from 1:12 --to 2:10 --logo logo.png --end "Volg ons"   # end card

One command from raw footage to a reel, in one render of just the reel's length: the transcript
(`<input>.words.json`, made once and reused), the chosen part(s) with silences (and with `--fillers` /
`--repeats` filler words and stutters) cut out, reframed to the format, captions timed to the cut
(`--preset`, `--highlight`, `--position`) and an optional hook `--title`. `--focus auto` (the default)
follows the people: shot changes are detected on every frame and faces a few times per second
(built-in SeetaFace model, no extra install), and every shot gets one steady framing that jumps
exactly on the cut (one video layer per framing); shots without a face are framed in the middle, and footage without any face (a
podcast cover, slides) is shown whole on a blurred backdrop. A two-shot whose people don't fit the
crop together follows the speaker: a hard cut to whoever talks, only on turns of 1.2 s or more (no
ping-pong on interjections). Who talks comes from the transcript's diarization labels (`--diarize`;
labels are matched to faces by mouth movement) or, without them, from the mouth movement itself
(read from a sharper decode of just that shot, with head movement compensated). Captions keep
clear of faces: per shot they stay at the default height, or move to the lowest position the app UI
leaves free (bottom ~15%) or near the top (below ~12%; not while the hook title shows) when the
caption block would cover a face and chin; a jump exactly on the cut. Text already burned into the
footage (subtitles of an earlier edit, lower thirds, name straps, slide text) is avoided the same way, with
two more slots right above and below it: it is found on the frames the face search decodes anyway (rows of
dense, high-contrast strokes of even width in a free-standing band 2..12% of the picture high, with edges
across as letters have; textures, railings, faces, pages of text and anything at the picture's edge don't
count) and only when it stays in its band for most of the shot or 3 s in a row, in the part the crop
shows. When no slot is clear of both, the captions cover the text rather than a face
(`FLITS_DEBUG_REFRAME=1` says so). An explicit `--position` keeps them fixed. Reels play at 30 fps at most (50 -> 25,
60 -> 30; `--fps` overrides). The hook title is black on a white box, readable on any footage. Jump cuts (removed pauses) alternate between
the framing and a 10% punch-in so they don't read as hiccups (`--no-punch`). Branding: `--logo` (png/svg,
top right on a soft shade, sized by height so wordmarks and marks look alike), `--accent` (progress
bar and active caption word), `--progress` (a bar along the bottom), `--music` (looped under the voice,
ducked while someone speaks, faded out). The scene is written next to the
output (`<out>.scene.json`) to tweak and re-render.

**Picking the part** (`--auto [SECONDS]`, default 45, 15..90; `--pick` / `--dry-run` only prints): the
transcript is split into sentences and every run of whole sentences of about the target length (counted with
long pauses shortened, as the reel cuts them) is scored with named reasons; a candidate never starts or ends
mid-sentence (a sentence start follows sentence punctuation and is not lower case; the end is a sentence end).
Points go to the **hook** (a question, a number, hook words like "the reason" / "het probleem", a contrast;
against: starting with a conjunction "and/but/so/en/maar/dus", a reply "ja/nou/yes/well", a back reference "dat
is/die gaan/that was", a verb or "dat zij ..." fragment, a greeting, answering a question that isn't in the clip;
a leading "Ja," / "Nou," / "So," is trimmed off), the **end** (a full stop followed by a pause, a conclusion "dus /
so / that's why / dan denk ik", a call to action "kom / volg / word lid / follow / join", an exclamation; against: a
trailing "...", a question, a next sentence that completes the thought "maar/dus ...", a thank-you/goodbye, a
one-word reaction), the **body** (against: an interviewer's new question halfway, short back-and-forth
reactions, a goodbye halfway, broken-off sentences, silences over 1.5 s, slow speech, filler words) and the
**length**. Structure works in any language; the cue lists cover English and Dutch. `--pick` prints the five best
non-overlapping parts with times, score, the first and last sentence, every reason and the `--from/--to` command
(`--json`: the same as JSON: `rank`, `from`, `to`, `length`, `score`, `first`, `last`, `reasons`). `--candidates N`
(1..5) renders the N best (`<out>-1.mp4`, ...). Several picks are several stories: they don't overlap, they share
few content words (a second part about the same subject is skipped), and in recordings over 10 minutes they lie
apart (3% of the length, up to 90 s). With diarization labels (`--diarize`), a part where one voice tells it
scores higher than a back-and-forth.

**End card** (`--end "Volg ons"`, `--end-logo [PATH]` (default: the `--logo`), `--end-duration` 2 s): after the
last clip the picture goes soft (a blurred still of the last frame, framed like the clip, drifting in on an
expo curve; without a still: the accent colour opening as an iris), the logo zooms in and the call to action
rises out of a mask word by word above an accent rule that draws itself. The voice ends with the last clip; the
`--music` carries on and fades out with the card; captions, the progress bar and the corner logo stop where the
card starts.

## Cutting: `flits cut`

    flits cut rec.mp4                                # shorten silences (--threshold -35 dB, --min 0.6 s, --keep 0.25 s)
    flits cut rec.mp4 --fillers --repeats            # also cut filler words and stutters ("I I I think", "th- the")
    flits cut rec.mp4 --fillers-list "uh,um,you know"   # your own list (phrases allowed)
    flits cut rec.mp4 --export fcpxml,premiere,edl,json # editor timelines instead of a render (--render: both)
    flits cut rec.mp4 --edit                         # -> rec.edit.txt: edit the recording as text...
    flits cut rec.mp4 --apply rec.edit.txt           # ...and cut what you deleted (word-accurate)
    flits cut rec.mp4 --dry-run                      # print the kept ranges

Words come from the transcript (`rec.words.json` next to the input, `--transcript`, or made automatically).
Removed words are cut from the middle of the pause before them to the middle of the pause after them, so
what remains sounds natural; silences are still shortened unless `--no-silence`.

**Filler words** follow the transcript language (`--lang`, else the `language` in the JSON): built-in lists for
`nl`, `en`, `de`, `fr`, `es`, `it`, `pt` (hesitations only: uh/um/ehm, äh/ähm, euh, eh/em, ...); other languages use a
combined list. When flits transcribes for `--fillers`/`--repeats` with an explicit `--lang`, it primes whisper to
write hesitations down (whisper tends to drop them). An existing transcript is reused as is.

**Repeats** (`--repeats`): an immediately repeated word or 2-3 word phrase loses its earlier take(s), and a cut-off
fragment ending in `-` that starts the next word goes too. Repeats across a sentence end or a pause over 1 s stay.

**Edit file** (`--edit` writes `<input>.edit.txt`, `--force` overwrites):

```text
# flits edit file for rec.mp4
# transcript: /abs/path/rec.words.json
[00:12.40] So this is the first sentence.
[00:15.02] {SPEAKER_01} And this is the answer.
```

One line per sentence (`[h:mm:ss.cc]` from an hour on; `{speaker}` only with a diarized transcript). Delete a line to
cut the sentence, delete words to cut only those words, or wrap parts in `[[ ]]` (may span lines). Lines are tied to
the recording by their time stamp (±1 s); within a line the remaining text is aligned to the original words, ignoring
case and punctuation, so a fixed typo keeps its word and added words are reported and ignored. A line without a time
stamp continues the line above it. Reordering is not supported. `--apply` combines with `--fillers`, `--repeats`,
`--export` and `--dry-run`.

**Exports** (`--export`, comma separated; files are named after `-o`/`<input>-cut`, every kept range is one clip that
refers to the original file, cut points snapped to source frames):

| format | file | open in |
|---|---|---|
| `fcpxml` (= `resolve`) | `.fcpxml` (FCPXML 1.9) | Final Cut Pro: File > Import > XML; DaVinci Resolve: File > Import > Timeline |
| `premiere` | `.xml` (FCP7 XML / xmeml 4) | Premiere Pro: File > Import (also Resolve) |
| `edl` | `.edl` (CMX3600) | DaVinci Resolve, Avid, ... (source must be in the media pool) |
| `json` | `.cuts.json` | scripts: exact `ranges` in seconds, `removed`, frame-snapped clips, `timebase` |

NTSC rates use exact rational timebases (23.976 = 24000/1001, 29.97 = 30000/1001, 59.94 = 60000/1001); a source
timecode (camera files starting at 01:00:00:00, drop-frame included) is respected; audio-only sources give an
audio-only timeline at 30 fps.

## Transcription: `flits transcribe`

    flits transcribe rec.m4a --lang en                         # -> rec.words.json + .srt + .vtt + .txt
    flits transcribe rec.m4a --initial-prompt "Arjan, flits, NVENC"   # prime names, jargon and spelling
    flits transcribe rec.m4a --vocab terms.txt                 # terms (one per line or comma separated) to favour
    flits transcribe rec.m4a --words-only                      # {"words": [...]} instead of segments
    flits transcribe rec.m4a --segments-only                   # no word timestamps (faster)
    flits transcribe rec.m4a --diarize [--speakers 2]          # speaker labels (optional, see below)

**Warm worker.** The first transcription starts a helper that keeps the model loaded (it stops by
itself after 10 idle minutes; `FLITS_WHISPER_IDLE` seconds, `FLITS_WHISPER_DAEMON=0` to run whisper
fresh every time). Starting Python and loading the model is most of the time for short clips: a 35 s
clip takes ~11 s the first time and under 2 s after that. Diarization always runs on its own.
`flits warm` starts the worker and loads the model ahead of time (e.g. while you pick the footage);
`flits reel` starts warming it by itself as soon as it starts, while it decodes the audio.

`--initial-prompt`, `--vocab`, `--diarize` also work on `captions`, `podcast` and `cut` (whenever they transcribe).
The JSON has `language`, `duration` and `segments` (`start`, `end`, `text`, `words`: `word`, `start`, `end`, `prob`),
with `--diarize` plus `speaker` per segment and word (`"SPEAKER_00"`, ...) and a `speakers` list. SRT/TXT then
name the speaker when it changes, VTT uses `<v SPEAKER_00>` voice tags.

**Diarization** uses [pyannote.audio](https://github.com/pyannote/pyannote-audio), installed through uv only when
`--diarize` is used (with PyTorch: a large first download). It needs a Hugging Face token: create a read token at
huggingface.co/settings/tokens, accept the conditions of `pyannote/speaker-diarization-community-1` (pyannote.audio 4;
`pyannote/speaker-diarization-3.1` + `pyannote/segmentation-3.0` for 3.x) and set `HF_TOKEN`. `FLITS_DIARIZE_MODEL`
picks another pipeline. With `--python`, install `pyannote.audio` into that Python yourself.

## From other languages

`bindings/typescript` (types + `render`/`still`/`sheet`/`check`/`transcribe`/`reel` wrapping the CLI; `reel`
takes typed options for every `flits reel` flag and returns the written videos, or with `pick` the ranked candidates) and
`bindings/python` (TypedDicts + the same functions; `flits.reel(video, out, from_=..., ...)`) build scenes as plain objects and run the
`flits` executable (`$FLITS_BIN` or PATH). The `Dockerfile` builds a Linux image with ffmpeg,
fonts and Vulkan drivers and `flits` as the entrypoint.
