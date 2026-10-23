# flits scene format

A scene is one JSON file. flits renders it to MP4 (H.264/HEVC/AV1 + AAC):

    flits render scene.json -o out.mp4        # full render (GPU compositor + hardware encoder when available)
    flits render scene.json --scale 0.5       # fast half-resolution preview
    flits preview scene.json                  # live preview in the browser (scrub, play); reloads on save
    flits sheet scene.json -o sheet.png       # 12 frames in one image: check the whole video at a glance
    flits still scene.json --at 12.5 -o f.png # single frame(s)
    flits check scene.json                    # validate (typos are reported) + print the resolved timeline
    flits new                                 # list example scenes; `flits new intro -o intro.json`

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
Types: `fade`, `slide-up`, `slide-down`, `slide-left`, `slide-right`, `zoom-in`, `zoom-out`, `pop`,
`wipe-left`, `wipe-right`, `wipe-up`, `wipe-down`. Crossfade = overlap two layers and give the top one `"in": "fade"`.

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
faster-whisper JSON / .srt / .vtt): `src`, `trim` (transcript time that maps to the layer start),
`style`: `karaoke` (active word highlighted) | `pop` (words pop in as spoken, TikTok-style) | `word` (one big word at a time) | `line` (plain),
`highlight` (color, default yellow), `highlightBg` (pill behind the active word), `maxWords` per chunk,
`size`, `font` (default Segoe UI Black), `color`, `stroke`, `shadow`, `uppercase`, `width` (wrap width).
With a diarized transcript (`flits transcribe --diarize`) a new chunk starts at every speaker change, and
`speakerColors` (e.g. `["#ffffff", "#4cc9f0"]`) gives each speaker its own text colour, in order of first appearance.

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
    flits captions clip.mp4 --style pop              # burn animated captions into a video
    flits podcast aflevering.mp3 --cover cover.jpg --title "Titel" --format 9:16 --from 12:30 --to 13:30
    flits cut opname.mp4 --fillers                   # remove silences (and uh/uhm) from audio or video
    flits transcribe opname.m4a --lang nl            # -> .words.json + .srt + .vtt + .txt

`slideshow`, `captions`, `podcast` and `cut` (video) also write `<out>.scene.json`; edit it and run
`flits render` to tweak.

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

`--initial-prompt`, `--vocab`, `--diarize` also work on `captions`, `podcast` and `cut` (whenever they transcribe).
The JSON has `language`, `duration` and `segments` (`start`, `end`, `text`, `words`: `word`, `start`, `end`, `prob`),
with `--diarize` plus `speaker` per segment and word (`"SPEAKER_00"`, ...) and a `speakers` list. SRT/TXT then
name the speaker when it changes, VTT uses `<v SPEAKER_00>` voice tags.

**Diarization** uses [pyannote.audio](https://github.com/pyannote/pyannote-audio), installed through uv only when
`--diarize` is used (with PyTorch: a large first download). It needs a Hugging Face token: create a read token at
huggingface.co/settings/tokens, accept the conditions of `pyannote/speaker-diarization-community-1` (pyannote.audio 4;
`pyannote/speaker-diarization-3.1` + `pyannote/segmentation-3.0` for 3.x) and set `HF_TOKEN`. `FLITS_DIARIZE_MODEL`
picks another pipeline. With `--python`, install `pyannote.audio` into that Python yourself.
