# flits scene format

A scene is one JSON file. flits renders it to MP4 (H.264/HEVC/AV1 + AAC):

    flits render scene.json -o out.mp4        # full render (GPU compositor + hardware encoder when available)
    flits render scene.json --scale 0.5       # fast half-resolution preview
    flits sheet scene.json -o sheet.png       # 12 frames in one image: check the whole video at a glance
    flits still scene.json --at 12.5 -o f.png # single frame(s)
    flits check scene.json                    # validate (typos are reported) + print the resolved timeline
    flits new                                 # list example scenes; `flits new intro -o intro.json`
                                              # (intro, quote, stats, lower-third, countdown, youtube-intro, podcast-quote,
                                              #  product-promo, news-headline, end-screen, kinetic-typography)

Units: pixels (canvas coordinates, origin top-left) and seconds. Relative file
paths resolve against the scene file's folder.

## Scene

```jsonc
{
  "width": 1920, "height": 1080, "fps": 30,
  "duration": 60,                      // optional: defaults to the end of the last layer
  "background": "#101014",
  "fonts": { "brand": "fonts/Rubik-Black.ttf" },   // aliases usable in "font"
  "loudness": -14,                     // optional: normalize the final mix (LUFS; -14 social, -16 podcast)
  "audio": [ { "src": "music.mp3", "volume": 0.6, "fadeIn": 1, "fadeOut": 3, "duck": true } ],
  "layers": [ ... ]                    // drawn in order: first = bottom
}
```

Audio track: `src`, `start` (scene time), `trim` (skip seconds of the source),
`duration`, `volume` (number or keyframes, see Animation), `fadeIn`, `fadeOut`, `loop`,
`duck` (`true` or a level like `0.2`: automatically lowers this track while other audio —
voice, video sound — plays). Audio from video layers is mixed in automatically
(`"volume": 0` on a video mutes it).

## Common layer properties

| key | meaning |
|---|---|
| `type` | `rect`, `image`, `video`, `text`, `counter`, `captions`, `waveform`, `group` |
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

Eases: `linear`, `in`, `out`, `in-out` (default), `smooth`, `back-in`, `back-out`, `elastic`, `bounce`, `spring`, `hold`.

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

**text** — `text` (`\n` for line breaks), `font` (installed font name like `"Segoe UI Black"`, a .ttf/.otf path or a
`fonts` alias; default Segoe UI Bold), `weight` (for variable fonts), `size` (64), `color`, `align`
(`left`/`center`/`right`), `maxWidth` (wraps), `lineHeight` (1.15), `letterSpacing`, `uppercase`,
`stroke` `{width,color}`, `background` `{ "color": "#000a", "padding": [24,12], "radius": 16, "mode": "box"|"lines" }`,
`reveal`: `"words"` | `"lines"` | `"chars"` | `"typewriter"` with `revealDuration` (seconds over which the pieces appear).

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
`size`, `font` (default Segoe UI Black), `color`, `stroke`, `shadow`, `uppercase`, `lineHeight`, `wrap: "balance"`,
`width` (wrap width).

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

## Ready-made commands

    flits slideshow fotos/ --music liedje.mp3 --beats --title "Zomer 2026" --format 9:16
    flits captions clip.mp4 --preset tiktok          # burn animated captions into a video (or --style pop)
    flits podcast aflevering.mp3 --cover cover.jpg --title "Titel" --format 9:16 --from 12:30 --to 13:30
    flits cut opname.mp4 --fillers                   # remove silences (and uh/uhm) from audio or video
    flits transcribe opname.m4a --lang nl            # -> .words.json + .srt + .vtt + .txt

`slideshow`, `captions`, `podcast` and `cut` (video) also write `<out>.scene.json`; edit it and run
`flits render` to tweak.
