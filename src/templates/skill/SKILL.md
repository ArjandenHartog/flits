---
name: flits-video
description: Make, edit and render videos with flits (GPU video renderer; videos written in TypeScript or JSON). Use whenever the user wants a video, animation or motion graphics - launch/promo/product videos, explainers, social clips (reels, shorts, TikTok), kinetic typography, title cards, lower thirds, animated charts and counters, animated captions/subtitles, intros/outros, slideshows, podcast clips - or wants an existing flits video (src/video.ts, *.scene.json) changed, reviewed or rendered to MP4.
---

# Making videos with flits

flits renders a video described in TypeScript (or JSON) on the GPU, without a browser: a 30 s 1080p video
renders in seconds. That changes how you work: **render stills constantly, look at them, fix, repeat.** You
cannot see motion, but you can read PNGs - so the review loop below is how you get a polished result.

Files: `api.md` (API cheat sheet), `example.ts` (a complete, reviewed 15 s launch video to copy patterns from).
`flits spec` prints the full scene format; `flits schema` the JSON Schema.

## The loop

1. **Format and fps.** Ask or infer: `formats.youtube` (1920x1080), `formats.reel`/`tiktok`/`short` (1080x1920),
   `formats.square` (1080x1080), `formats.portrait` (1080x1350). 30 fps by default; **60 fps for motion-heavy
   pieces** (kinetic type, fast moves) - it doubles render time, which is still seconds.
2. **Storyboard before code.** Write the acts with times, one idea per beat, e.g. for 15 s:
   `0-3 hook (title) | 3-7 problem | 7-11 product + 3 features | 11-15 CTA + logo`. Give each beat a hold long
   enough to read (~0.3 s per word, minimum ~1.2 s for a headline) plus ~0.6 s for its entrance.
   Put the storyboard as a comment at the top of `src/video.ts`.
3. **Write `src/video.ts`.** One component per beat (a function returning a `Group`), timed with `At`/`Sequence`.
   Keep design tokens (colours, fonts, margins, eases) in constants at the top.
4. **`flits check src/video.ts`** - schema errors, typos (unknown keys are warnings), missing files, and layout
   lint: text running off the canvas, text overlapping text, text on screen too briefly to read. Fix every
   warning (or know why it is intended). `--json` gives the same as one JSON object (timeline + issues).
5. **Look at stills.** Pick moments from your storyboard: each beat at rest *and* mid-motion.
   ```sh
   flits still src/video.ts --at 0.4 --at 1.5 --at 3.2 --at 7.5 -o out/s.png   # prints out/s-0.40s.png ...
   flits sheet src/video.ts -o out/sheet.png                    # 12 frames over the whole video, time-labelled
   flits sheet src/video.ts --from 2.8 --to 3.6 --count 8 -o out/cut.png      # one transition, frame by frame
   flits sheet src/video.ts --at 0.5 --at 3 --at 4.4 --at 6.5 -o out/beats.png # your storyboard moments in one image
   ```
   **Read the PNGs** (open them with your file-reading tool) and review against the checklist below.
6. **Fix and repeat** 4-5 until a full sheet looks like a finished piece. Iterating is cheap (a still takes
   ~1 s); do several rounds, it is what separates a polished video from a first draft.
7. **Render**: `flits render src/video.ts -o out/video.mp4` (prints time and fps). For a quick look at the
   real motion use `--scale 0.5`. Report the output path and render time.

### Review checklist (for every still)

- **Clipping**: nothing important cut by the frame edge; nothing cut by a group `clip` you didn't intend.
- **Overlap**: text never collides with other text or busy imagery; outgoing and incoming beats don't pile up
  (check the sheet across each cut).
- **Alignment**: elements on a shared grid/margin; left edges line up; centred things really centred.
- **Hierarchy**: one obvious focal point per frame; the headline is clearly the biggest thing.
- **Contrast**: text readable on its background (add a scrim/gradient behind text on photos).
- **Timing**: mid-motion frames look intentional (no half-faded mush at a hold point); each beat is fully
  arrived before it starts leaving; the last frame is a clean end card, not a frame of an exit.
- **Density**: at most ~7 words on screen at once in a social video; cut copy rather than shrink it.

## Motion design craft

This is what makes it look professional instead of "PowerPoint".

**Easing.** Never `linear` for movement (only for constant drifts and progress bars). Arrivals: `expo-out` (fast
in, long settle) or `quint-out`; exits: `expo-in` (accelerates away), and shorter than the entrance
(~0.35 s vs ~0.6 s). Moves from A to B on screen: `expo-in-out`. `back-out` only for small playful pops;
`elastic`/`bounce` almost never. Custom curves: `"cubic-bezier(0.16, 1, 0.3, 1)"`.

**Timing.** Stagger related elements by 60-120 ms (`start: i * 0.08`), never all at once. Entrances 0.5-0.8 s,
exits 0.3-0.4 s. Holds long enough to read: ~0.3 s per word after the text has landed. Leave 0.2-0.4 s of
"air" between one beat leaving and the next arriving, or overlap them deliberately (a transition), never
half-way. End on a held end card (1.5-2 s).

**Masked text reveals** look premium: `reveal: "words"` or `"lines"` + `revealStyle: "mask"` (each piece slides up
out of its own line box, clipped, on expo-out). `revealDuration` = time over which the pieces start (0.3-0.6 s
for a headline). `"blur"` is a softer alternative; `"chars"` for short words only.

**Motion blur.** `motionBlur: true` on the `Composition` (scene) makes fast moves read as motion instead of
strobing. Costs nothing for still layers. `motionBlur: false` on a layer keeps it crisp (timecodes, UI).

**Typography.** Big, tight headlines (110-180 px at 1080p, `letterSpacing` -2 to -4, `lineHeight` 1.0-1.05,
heavy weight) against small labels (22-30 px, uppercase, mono or wide `letterSpacing` 2-6, muted colour).
Two fonts at most. Use `wrap: "balance"` + `maxWidth` for multi-line text. Don't centre everything: a strong
left-aligned layout on a margin often looks more designed.

**Grid and margins.** Work on a grid: margins of ~6-8% of the width (120-160 px at 1920), content aligned to them.
Put anchors where you align: `anchor: [0, 0.5]` + `x: margin` for left-aligned text.

**Colour.** A restrained palette: background, one text colour, one muted colour, **one accent** used sparingly
(a word, a rule, a number). Dark near-black backgrounds (`#0b0b0f`) with a subtle radial gradient or grain look
richer than pure black. Check contrast in the stills.

**Transitions.** Choose one or two transition styles for the whole piece and use them consistently (e.g. every
beat leaves with `expo-in` up-and-fade, every beat arrives with a mask reveal). Built-ins (`in`/`out`):
`slide-fade`, `zoom-blur`, `push-*` (with `Sequence({ overlap })` for camera-like pushes), `wipe-*`, `iris`.

**Sound.** Music makes or breaks a promo. `audio: [{ src: "music.mp3", volume: 0.8, fadeIn: 0.5, fadeOut: 2 }]`
on the composition, `loudness: -14` (social/YouTube) to normalize; `duck: 0.25` lowers music under voice-over.
Cut beats on musical phrases when you know the tempo (a beat at 120 BPM = 0.5 s). Never ship music you don't
have rights to; ask the user for a track if none is given.

## Gotchas (learned the hard way)

- **Positions are the anchor point**, default `anchor: [0.5, 0.5]` (centre) in the parent's box. For left-aligned
  text use `anchor: [0, 0.5]` (or `[0, 0]`) *and* `align: "left"` for multi-line text.
- **Text sizes itself**; `width` doesn't wrap text, `maxWidth` does. Use `flits measure` to get exact text widths
  when laying things out next to text (e.g. an underline exactly under a word):
  `echo '[{"text":"Launch","font":"Segoe UI Black","size":140}]' | flits measure` -> `[{"width":..,"height":..}]`.
- **Groups default to the full parent size** with children positioned inside. A `clip: true` group clips to its own
  box: give it an explicit `width`/`height` and position, and remember children's coordinates are then relative
  to that box (centre = `width/2, height/2`).
- **Everything is rasterized once, at the largest size it reaches** (from `scale`, `zoom`, parent scale and
  transitions), then only transformed per frame. Consequences: SVGs and shapes stay sharp when they scale; text
  is oversampled up to 3x only, so a word that zooms from `scale: 0.2` to `8` goes soft - set the text at its
  biggest size and animate `scale` *down* to it instead; photos are never upscaled past their own pixels, so
  use sources at least as large as they appear (Ken Burns `zoom` included).
- **Fonts**: an installed font name (`"Segoe UI Black"`, `"Bahnschrift"`, `"Cascadia Mono"` on Windows), or ship a
  file: `fonts: { display: "../fonts/Inter-Black.ttf" }` on the composition and `font: "display"`. Files make the
  video render the same on every machine. Variable fonts take `weight`.
- **Relative paths resolve from the scene file's folder** (`src/` for `src/video.ts`): put media in `src/assets/`
  or write `"../assets/x.png"`.
- **Time is relative to the parent group.** A layer inside `At({ time: 3 })` with `start: 0.2` starts at 3.2 s.
  Children without `duration` last as long as their group.
- **Animation times are relative to the layer's own start.** `{ from, to, start: 0.5, end: 1.6 }` on a layer that
  itself has `start: 0.5` begins at 1.0 s in the group. When two layers must move in sync (a bar and the number
  riding on its tip) but start at different times, give each its own timing (see `Bar` in `example.ts`).
- **Counters and motion blur**: a counting number moves its digits every frame, so motion blur averages them into
  a ghosted smear. Give `Counter`s (and other changing text such as timecodes) `motionBlur: false`.
- **Mixed sizes on one line** (a 30 px mono index next to a 72 px headline) don't line up optically by centre
  anchors alone; check a full-size still and nudge by a few px.
- Contact sheet thumbnails are small: use them for timing and layout, then `flits still` at full size to judge
  type, alignment and blur.
- Function props `(t) => number` are sampled per frame: fine for maths, but the layer (or its group or the scene)
  needs a known duration.
- `in`/`out` transitions play inside the layer's lifetime (an `out: "fade 0.4"` fades during the last 0.4 s).
- A crossfade is two overlapping layers with the top one `in: "fade"`; a `Sequence({ overlap: 0.4 })` does that
  for consecutive beats.
- `flits check` warnings about unknown keys are typos: fix them, they are silently ignored otherwise.
- Relative imports in TypeScript need the extension: `import { Title } from "./title.ts"`.

<!-- flits-motion-library: start -->
## Motion components (import from `#flits`)

Prefer these over hand-built layers: they carry the craft rules above (expo curves, masks, staggers, real
text metrics) and restyle from one theme. Full props and JSDoc: `flits/motion.d.ts`. Shared props: `at`
(when it comes in), `exit` (when it starts leaving), `x`/`y` (top-left; top-centre/right with `align`),
`theme` (per-component override).

```ts
import { Composition, Act, Hud, KineticTitle, HighlightRule, StatCounter, EndCard, Whip, setTheme, themeScene, timeline } from "#flits";
setTheme({ colors: { accent: "#ff4a2b" } });               // once, at the top: colours, fonts (fontFiles), timings
const acts = timeline({ hook: 4, proof: 6, end: 4 });      // { hook: { at, end, duration }, ..., total }
export default Composition({ width: 1920, height: 1080, fps: 60, duration: acts.total, motionBlur: true, ...themeScene(),
  children: [
    Act({ ...acts.hook, exit: "rise", children: KineticTitle({ text: "One idea,\nper beat.", eyebrow: "Launch", rule: true, at: 0.4 }) }),
    Act({ ...acts.proof, children: [StatCounter({ value: 12840, label: "renders a day", at: 0.5 })] }),
    EndCard({ at: acts.end.at, duration: 4, title: "Make it move.", url: "example.com", fadeOut: true }),
  ] });
```

| beat | component |
|---|---|
| headline / claim | `KineticTitle` (multi-line, staggered mask reveal, eyebrow, accent rule, exit), `MaskLine` (one line) |
| label, eyebrow, readout | `Eyebrow` / `Label` (tracked mono capitals) |
| emphasis on words | `HighlightRule` (`marks: [[phrase, t], ...]`, rule glides from word to word; `variant: "marker"`) |
| typing, chat, code | `Typewriter` (cursor follows the text, then blinks) |
| quote | `Quote` (word reveal + attribution) |
| structure | `Rule`, `Outline` (drawn box or corner brackets), `Tag` (chip), `Grid`, `Backdrop` |
| persistent frame | `Hud` (rules, tick ruler, playhead, timecode, chapter names) |
| people | `LowerThird` (name + role) |
| numbers | `StatCounter`, `BarChart`, `LineChart` (draws itself), `ProgressRing`, `ProgressBars`, `Checklist` |
| acts and transitions | `Act` (`exit: "rise" \| "sink" \| "fade" \| "whip"`), `timeline()`, `Stagger`, `Whip` |
| closing | `EndCard` |

Use one transition family per video (e.g. every act rises; one `Whip` for the big turn). Counters and other
changing digits should not be motion blurred (`motionBlur: false` on them) or the digits smear.
<!-- flits-motion-library: end -->

## From raw footage: `flits reel`

For a reel/short from an interview or talk, don't build it by hand: one command transcribes (once; the
transcript is reused), cuts silences (`--fillers`: also uh/ehm), follows the faces shot by shot
(`--focus auto`, the default), adds captions and a hook title, and renders only the reel's length:

```sh
flits transcribe talk.mp4 --lang nl                 # first: read talk.words.txt (or .srt for times) to pick the best part
flits reel talk.mp4 --from 1:12 --to 2:10 --fillers --title "Hook in a few words" --preset tiktok
flits reel talk.mp4 --clips "0:40-1:05,3:10-3:32"   # several parts, in this order
flits still talk-reel.scene.json --at 2 --at 20 -o out/r.png   # check the framing and captions
```

Footage without faces (a static podcast cover, slides) is shown whole on a blurred backdrop
automatically; `--fit fit` forces that, `--focus 0.5` forces a crop. 50/60 fps footage becomes a
25/30 fps reel (`--fps` to override). Pick parts that stand alone: a clear first sentence (the hook), one idea, a natural end; 30-60 s. Then
review stills at a few moments (faces in frame, captions not covering faces). `talk-reel.scene.json` is a
normal scene: tweak it (positions, preset, title) and `flits render` it again.


## API in one screen

See `api.md` for the cheat sheet (layer types, props, eases, transitions). The rest: `flits spec`.
