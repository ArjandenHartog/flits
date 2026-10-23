# flits scene format

A scene is one JSON file. flits renders it to MP4 (H.264 + AAC):

    flits render scene.json -o out.mp4        # full render (GPU + hardware encoder when available)
    flits render scene.json --scale 0.5       # fast half-resolution preview
    flits sheet scene.json -o sheet.png       # 12 frames in one image: check the whole video at a glance
    flits still scene.json --at 12.5 -o f.png # single frame(s)
    flits check scene.json                    # validate + print the resolved timeline

Units: pixels (canvas coordinates, origin top-left) and seconds. Relative file
paths resolve against the scene file's folder.

## Scene

```json
{
  "width": 1920, "height": 1080, "fps": 30,
  "duration": 60,                      // optional: defaults to the end of the last layer
  "background": "#101014",
  "fonts": { "brand": "fonts/Rubik-Black.ttf" },   // aliases usable in "font"
  "audio": [ { "src": "music.mp3", "volume": 0.6, "fadeIn": 1, "fadeOut": 3 } ],
  "layers": [ ... ]                    // drawn in order: first = bottom
}
```

Audio track: `src`, `start` (scene time), `trim` (skip seconds of the source),
`duration`, `volume`, `fadeIn`, `fadeOut`, `loop`. Audio from video layers is
mixed in automatically (set `"volume": 0` on a video to mute it).

## Common layer properties

| key | meaning |
|---|---|
| `type` | `rect`, `image`, `video`, `text`, `captions`, `waveform`, `group` |
| `start`, `end` / `duration` | lifetime in seconds (relative to the parent group) |
| `x`, `y` | position of the anchor point (default: centre of the parent) |
| `width`, `height` | box size (default: parent size; text sizes itself; image/video with one side given keep their aspect ratio) |
| `anchor` | `[0.5,0.5]` = centre (default), `[0,0]` = top-left, `[1,1]` = bottom-right |
| `scale`, `scaleX`, `scaleY`, `rotation` (degrees), `opacity` | animatable |
| `in`, `out` | enter/exit transition (see below) |
| `radius` | corner radius |
| `border` | `{ "width": 14, "color": "#fff" }` (drawn outside the box) |
| `shadow` | `{ "color": "rgba(0,0,0,0.5)", "blur": 24, "x": 0, "y": 12 }` (text: text shadow; others: box shadow) |

### Animation

Any animatable number can be:

* a constant: `"opacity": 0.8`
* from/to over the whole layer: `"zoom": { "from": 1.0, "to": 1.15, "ease": "linear" }` (optional `start`, `end`)
* keyframes `[time, value, ease]`: `"y": [[0, 1200], [0.6, 540, "back-out"], ["end-0.5", 540], ["end", -200, "in"]]`
  (time in seconds from layer start, or `"end"`/`"end-0.5"`; the ease applies to the segment ending at that key)

Eases: `linear`, `in`, `out`, `in-out` (default), `smooth`, `back-in`, `back-out`, `elastic`, `bounce`, `spring`, `hold`.

### Transitions (`in` / `out`)

`"in": "fade"`, `"in": "pop 0.4"`, or `{ "type": "slide-up", "duration": 0.6, "ease": "out", "distance": 200 }`.
Types: `fade`, `slide-up`, `slide-down`, `slide-left`, `slide-right`, `zoom-in`, `zoom-out`, `pop`,
`wipe-left`, `wipe-right`, `wipe-up`, `wipe-down`. Crossfade = overlap two layers and give the top one `"in": "fade"`.

## Layer types

**rect** — `color` or `gradient`:
`{ "type": "linear", "angle": 180, "stops": [[0, "#000000"], [1, "#00000000"]] }` or
`{ "type": "radial", "colors": ["#f26328", "#120a1c"] }` (angle like CSS: 180 = top to bottom).

**image** — `src` (jpg/png/webp/gif/bmp; HEIC/AVIF via ffmpeg), `fit`: `cover` (default) | `contain` | `fill`,
`zoom` + `panX`/`panY` (−1..1) for Ken Burns inside the box, `blur` (px, cheap: done once).
EXIF rotation is applied automatically; images are downscaled once to the size they are shown at.

**video** — like image plus `trim` (start offset in the source), `volume` (default 1, 0 = mute),
`fadeIn`/`fadeOut` (audio), `loop`. Default duration = the clip length. iPhone HDR is tone-mapped.

**text** — `text` (`\n` for line breaks), `font` (installed font name like `"Segoe UI Black"`, a .ttf/.otf path or a
`fonts` alias; default Segoe UI Bold), `weight` (for variable fonts), `size` (64), `color`, `align`
(`left`/`center`/`right`), `maxWidth` (wraps), `lineHeight` (1.15), `letterSpacing`, `uppercase`,
`stroke` `{width,color}`, `background` `{ "color": "#000a", "padding": [24,12], "radius": 16, "mode": "box"|"lines" }`,
`reveal`: `"words"` | `"lines"` with `revealDuration` (seconds over which the pieces appear).

**captions** — animated word-by-word subtitles from a transcript (`flits transcribe` output, or any
faster-whisper JSON / .srt / .vtt): `src`, `trim` (transcript time that maps to the layer start),
`style`: `karaoke` (active word highlighted) | `pop` (words pop in as spoken, TikTok-style) | `word` (one big word at a time) | `line` (plain),
`highlight` (color, default yellow), `highlightBg` (pill behind the active word), `maxWords` per chunk,
`size`, `font` (default Segoe UI Black), `color`, `stroke`, `shadow`, `uppercase`, `width` (wrap width).
Place it with `x`/`y` (default: centre).

**waveform** — live audio spectrum bars: `src` (audio/video file), `trim`, `bars` (48), `color`, `mirror`
(bars grow from the centre line), `gap` (0..1 of a slot), `gain`, `radius`.

**group** — `children`: layers positioned/timed relative to the group. Group transforms, opacity and
transitions apply to all children. `"sequence": true` plays children one after another (each needs a
`duration`); `"overlap": 0.5` overlaps consecutive children (use with `"in": "fade"` for crossfades).

## Example

```json
{
  "width": 1080, "height": 1920, "fps": 30,
  "audio": [{ "src": "voice.mp3" }],
  "layers": [
    { "type": "image", "src": "bg.jpg", "blur": 40, "zoom": { "from": 1.1, "to": 1.25 } },
    { "type": "rect", "color": "rgba(0,0,0,0.35)" },
    { "type": "group", "sequence": true, "overlap": 0.4, "children": [
      { "type": "image", "src": "a.jpg", "duration": 4, "height": 1100, "radius": 24,
        "border": { "width": 12, "color": "#fff" }, "shadow": { "blur": 40, "y": 24 },
        "rotation": [[0, -6], [0.7, -2, "back-out"]], "in": "pop" },
      { "type": "video", "src": "clip.mp4", "duration": 5, "height": 1100, "radius": 24, "in": "fade" }
    ]},
    { "type": "text", "text": "Zomertrip 2026", "y": 260, "size": 96, "font": "Segoe UI Black",
      "shadow": { "blur": 20, "y": 6 }, "in": { "type": "slide-up", "duration": 0.7 } },
    { "type": "captions", "src": "voice.words.json", "y": 1500, "style": "pop", "size": 84,
      "highlight": "#ffd60a", "stroke": { "width": 8, "color": "#000" } }
  ]
}
```

## Ready-made commands

    flits transcribe opname.mp3 --lang nl          # -> opname.words.json + .srt + .vtt + .txt
    flits captions clip.mp4 --style pop            # burn animated captions into a video
    flits podcast aflevering.mp3 --cover cover.jpg --title "Titel" --format 9:16 --from 12:30 --to 13:30

`captions` and `podcast` also write `<out>.scene.json`; edit it and run `flits render` to tweak.
