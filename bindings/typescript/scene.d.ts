/**
 * TypeScript types for the flits scene format.
 *
 * Mirrors schema/scene.schema.json (generated from the Rust structs with `flits schema`);
 * tests/schema.rs checks that every property and enum value of the schema appears here.
 * Units are pixels (origin top-left) and seconds; relative paths resolve against the scene's folder.
 */

// ------------------------------------------------------------------ values

/**
 * Colour: "#rgb", "#rgba", "#rrggbb", "#rrggbbaa", "rgb(r,g,b)", "rgba(r,g,b,a)" (r,g,b 0..255, a 0..1)
 * or a name: "white", "black", "transparent", "red", "green", "blue", "yellow", "orange", "purple", "pink", "gray", "grey".
 */
export type Color = string;

/** Easing curve. Canonical names first; the rest are aliases. */
export type Ease =
  | "linear"
  | "in"
  | "out"
  | "in-out"
  | "smooth"
  | "back-in"
  | "back-out"
  | "elastic"
  | "bounce"
  | "spring"
  | "hold"
  | "ease-in"
  | "easein"
  | "ease-out"
  | "easeout"
  | "ease"
  | "inout"
  | "ease-in-out"
  | "easeinout"
  | "sine"
  | "sine-in-out"
  | "anticipate"
  | "back"
  | "overshoot"
  | "step";

/** Keyframe time: seconds from the layer start, or "end" / "end-0.5" (relative to the layer end). */
export type KeyTime = number | "end" | `end-${number}` | `end+${number}` | `${number}`;

/** `[time, value]` or `[time, value, ease]`; the ease applies to the segment ending at this key (default "in-out"). */
export type Keyframe = [time: KeyTime, value: number] | [time: KeyTime, value: number, ease: Ease];

/** Animate from one value to another over the layer (or between `start` and `end`). */
export interface FromTo {
  /** value at `start` */
  from: number;
  /** value at `end` */
  to: number;
  /** easing curve (default "linear") */
  ease?: Ease;
  /** when the animation starts (default 0 = layer start) */
  start?: KeyTime;
  /** when the animation ends (default "end" = layer end) */
  end?: KeyTime;
  comment?: string;
}

/** An animatable number: a constant, keyframes `[[time, value, ease?], ...]` or `{ from, to, ease, start, end }`. */
export type Animated = number | Keyframe[] | FromTo;

export type TransitionType =
  | "fade"
  | "slide-up"
  | "slide-down"
  | "slide-left"
  | "slide-right"
  | "slide-fade"
  | "zoom-in"
  | "zoom-out"
  | "zoom-fade"
  | "zoom-blur"
  | "pop"
  | "wipe-left"
  | "wipe-right"
  | "wipe-up"
  | "wipe-down"
  | "push-left"
  | "push-right"
  | "push-up"
  | "push-down"
  | "rotate-in"
  | "flip-x"
  | "flip-y"
  | "iris"
  // aliases: rise = slide-fade, zoom = zoom-in, wipe = wipe-right, push = push-right, rotate = rotate-in, flip = flip-x, circle = iris
  | "up"
  | "down"
  | "left"
  | "right"
  | "rise"
  | "zoom"
  | "wipe"
  | "push"
  | "rotate"
  | "flip"
  | "circle";

/** Enter/exit transition as an object. */
export interface TransitionSpec {
  /** transition type (default "fade") */
  type?: TransitionType;
  /** seconds (default 0.5; pop 0.45; iris and push 0.6) */
  duration?: number;
  /** easing curve (default "out"; pop "back-out"; wipes, push and iris "in-out") */
  ease?: Ease;
  /** slide distance in px (default 120; slide-fade 40) */
  distance?: number;
  /** rotate-in: degrees turned during the transition (default 90) */
  angle?: number;
  comment?: string;
}

/** Enter (`in`) or exit (`out`) transition: "fade", "pop 0.4" (type + seconds) or a {@link TransitionSpec}. */
export type Transition = TransitionType | `${TransitionType} ${number}` | `${TransitionType} ${number}s` | TransitionSpec;

/** Ducking: `true` (duck to 0.25) or the volume level (0..1) to duck to while other audio plays. */
export type Duck = boolean | number;

/** Padding in px: one number or [horizontal, vertical]. */
export type Padding = number | [x: number, y: number];

export type LayerType = "rect" | "image" | "video" | "text" | "counter" | "captions" | "waveform" | "shape" | "group";
/** rect (default), ellipse, circle, line, arrow, polygon, star, path (SVG path data in `d`) */
export type ShapeKind = "rect" | "ellipse" | "circle" | "line" | "arrow" | "polygon" | "star" | "path";
/** round (default), butt, square */
export type LineCap = "round" | "butt" | "square";
/** cover (default, crop), contain (letterbox), fill (stretch; alias "stretch") */
export type Fit = "cover" | "contain" | "fill" | "stretch";
/** left, center (default), right (aliases start, middle, end) */
export type Align = "left" | "center" | "right" | "start" | "middle" | "end";
export type Reveal = "words" | "lines" | "chars" | "typewriter" | "none";
/** balance: even line lengths; greedy (default) */
export type Wrap = "balance" | "greedy";
/** karaoke (default), pop, bounce, word, line; aliases highlight, tiktok, jump, single, plain, subtitle */
export type CaptionStyle = "karaoke" | "pop" | "bounce" | "word" | "line" | "highlight" | "tiktok" | "jump" | "single" | "plain" | "subtitle";
/** Ready-made caption looks (see SPEC.md); layer properties override the preset. */
export type CaptionPreset = "tiktok" | "karaoke-box" | "bounce" | "minimal" | "neon" | "highlighter" | "subtitle" | "word-pop-big";
/** pill (default; glides from word to word) or marker (highlighter stroke); aliases box, highlighter */
export type HighlightBox = "pill" | "marker" | "box" | "highlighter";
/** box (default): one box around the text block; lines: one pill per line */
export type TextBgMode = "box" | "lines";
export type GradientType = "linear" | "radial";

// ------------------------------------------------------------------ objects

/** Border drawn outside the box. */
export interface Border {
  /** px */
  width: number;
  color: Color;
  comment?: string;
}

/** Drop shadow (text: text shadow; other layers: box shadow). */
export interface Shadow {
  /** default "rgba(0,0,0,0.5)" */
  color?: Color;
  /** blur radius in px (default 24) */
  blur?: number;
  /** horizontal offset in px (default 0) */
  x?: number;
  /** vertical offset in px (default 12) */
  y?: number;
  comment?: string;
}

/** Text outline. */
export interface StrokeSpec {
  /** px */
  width: number;
  color: Color;
  comment?: string;
}

/** Box (or one pill per line) behind text. */
export interface TextBg {
  color: Color;
  padding?: Padding;
  /** corner radius in px (default: a quarter of the font size) */
  radius?: number;
  mode?: TextBgMode;
  comment?: string;
}

/** Colour grading for image and video layers. */
export interface Filter {
  /** added brightness, e.g. 0.05 (default 0) */
  brightness?: number;
  /** contrast factor (default 1) */
  contrast?: number;
  /** saturation factor (default 1, 0 = gray) */
  saturation?: number;
  /** warm (positive) or cool (negative) tint, e.g. 0.3 */
  warmth?: number;
  grayscale?: boolean;
  comment?: string;
}

/** Gradient fill for rect layers: `stops` ([offset 0..1, colour] pairs) or evenly spaced `colors`. */
export interface Gradient {
  /** default "linear" */
  type?: GradientType;
  /** CSS-style angle in degrees: 180 = top to bottom (default), 90 = left to right */
  angle?: number;
  stops?: [offset: number, color: Color][];
  colors?: Color[];
  comment?: string;
}

/** An audio file in the soundtrack. */
export interface AudioTrack {
  /** audio (or video) file */
  src: string;
  /** scene time in seconds where the track starts */
  start?: number;
  /** seconds to skip at the start of the source */
  trim?: number;
  /** seconds to play (default: until the source ends) */
  duration?: number;
  /** constant or keyframed (seconds from the track start) */
  volume?: Animated;
  fadeIn?: number;
  fadeOut?: number;
  /** repeat the source to fill `duration` (or the scene) */
  loop?: boolean;
  /** duck under the other audio (voice, video sound) */
  duck?: Duck;
  comment?: string;
}

/**
 * All layer properties. flits accepts every property on every layer type (and ignores the ones that don't
 * apply); the per-type interfaces below add the discriminant and the required properties.
 */
export interface LayerProps {
  type: LayerType;
  /** optional name (for your own bookkeeping) */
  id?: string;
  /** start time in seconds (relative to the parent group) */
  start?: number;
  /** end time in seconds (alternative to `duration`) */
  end?: number;
  /** lifetime in seconds (videos default to their (re-timed) length) */
  duration?: number;

  /** x of the anchor point in px (default: centre of the parent) */
  x?: Animated;
  /** y of the anchor point in px (default: centre of the parent) */
  y?: Animated;
  /** box width in px (default: parent width; text sizes itself; image/video keep their aspect ratio when only one side is given) */
  width?: number;
  /** box height in px */
  height?: number;
  /** [0.5, 0.5] = centre (default), [0, 0] = top-left, [1, 1] = bottom-right */
  anchor?: [number, number];
  scale?: Animated;
  scaleX?: Animated;
  scaleY?: Animated;
  /** degrees, clockwise */
  rotation?: Animated;
  /** 0..1 */
  opacity?: Animated;
  /** image/video: zoom inside the box (Ken Burns), 1 = fit */
  zoom?: Animated;
  /** image/video: horizontal pan inside the box, -1..1 */
  panX?: Animated;
  /** image/video: vertical pan inside the box, -1..1 */
  panY?: Animated;
  /** enter transition */
  in?: Transition;
  /** exit transition */
  out?: Transition;
  /** corner radius in px (>= half the size gives a circle/pill) */
  radius?: number;
  border?: Border;
  shadow?: Shadow;
  /** colour grading (image, video) */
  filter?: Filter;
  /** group: clip children to the group's box */
  clip?: boolean;
  /** free-form note; ignored by flits */
  comment?: string;

  /** image/video: media file; captions: transcript (.words.json/.srt/.vtt); waveform: audio or video file */
  src?: string;
  fit?: Fit;
  /** image: blur radius in px (done once, cheap) */
  blur?: number;
  /** video/captions/waveform: seconds to skip at the start of the source */
  trim?: number;
  /** video: audio volume (default 1, 0 = mute) */
  volume?: Animated;
  /** video: loop the clip */
  loop?: boolean;
  /** video: playback speed (0.5 = slow motion, 2 = double speed) */
  speed?: number;
  /** video: audio fade-in in seconds */
  fadeIn?: number;
  /** video: audio fade-out in seconds */
  fadeOut?: number;

  /** rect: fill colour; text/counter/captions: text colour (default white) */
  color?: Color;
  /** rect: gradient fill (instead of `color`) */
  gradient?: Gradient;

  /** text: the text ("\n" for line breaks; "&shy;" or U+00AD for soft hyphens) */
  text?: string;
  /** installed font name ("Segoe UI Black"), a .ttf/.otf path or a `fonts` alias */
  font?: string;
  /** font size in px (text 64, counter 96) */
  size?: number;
  /** weight for variable fonts / font families (e.g. 900) */
  weight?: number;
  align?: Align;
  /** wrap text at this width in px */
  maxWidth?: number;
  /** multiple of the font size (default 1.15) */
  lineHeight?: number;
  /** extra space between letters in px */
  letterSpacing?: number;
  stroke?: StrokeSpec;
  background?: TextBg;
  uppercase?: boolean;
  reveal?: Reveal;
  wrap?: Wrap;
  /** seconds over which the `reveal` pieces appear */
  revealDuration?: number;

  /** captions: named look; properties set on the layer override it */
  preset?: CaptionPreset;
  /** captions style */
  style?: CaptionStyle;
  /** captions: colour of the active word (default yellow) */
  highlight?: Color;
  /** captions: box colour behind the active word (shape: `highlightBox`) */
  highlightBg?: Color;
  /** captions: shape of the `highlightBg` box */
  highlightBox?: HighlightBox;
  /** captions: most words per chunk */
  maxWords?: number;
  /** captions: most lines per chunk (default 2) */
  maxLines?: number;
  /** captions: active-word colours, cycled word by word (instead of `highlight`) */
  colors?: Color[];
  /** captions: size of the active word relative to the others (default 1.12 for pop, 1 otherwise) */
  activeScale?: number;
  /** captions: word colour per diarization speaker label, e.g. { SPEAKER_00: "#ffffff" } */
  speakerColors?: Record<string, Color>;

  /** shape: what to draw */
  shape?: ShapeKind;
  /** shape: points [[x, y], ...] in the layer box (line, arrow: a polyline; polygon: its corners) */
  points?: [x: number, y: number][];
  /** shape "path": SVG path data in layer-box units, e.g. "M 0 100 C 50 0 150 0 200 100" */
  d?: string;
  /** shape: corners of a regular polygon (default 6) or points of a star (default 5) */
  sides?: number;
  /** shape "star": inner radius as a fraction of the outer one (default 0.5) */
  innerRadius?: number;
  /** shape: ends of lines and dashes */
  lineCap?: LineCap;
  /** shape: dash pattern of the stroke in px, e.g. [20, 10] */
  dash?: number[];
  /** shape: how much of the outline is drawn, 0..1 (animate it to draw the shape on; default 1) */
  progress?: Animated;
  /** shape "arrow": length of the arrow head in px (default 4 x the stroke width) */
  headSize?: number;

  /** waveform: number of bars (default 48) */
  bars?: number;
  /** waveform: bars grow from the centre line */
  mirror?: boolean;
  /** waveform: gap between bars as a fraction of a slot (0..1) */
  gap?: number;
  /** waveform: amplification */
  gain?: number;

  /** counter: the number, e.g. { from: 0, to: 12840, ease: "out", end: 2.5 } */
  value?: Animated;
  /** counter: digits after the decimal mark */
  decimals?: number;
  prefix?: string;
  suffix?: string;
  /** counter: thousands separator: "," (default, 1,234.5), "." (1.234,5) or "none" */
  separator?: string;

  /** group: child layers (positioned and timed relative to the group) */
  children?: Layer[];
  /** group: play children one after another (each needs a duration; clips use their length) */
  sequence?: boolean;
  /** group with `sequence`: seconds consecutive children overlap */
  overlap?: number;
}

export interface RectLayer extends LayerProps {
  type: "rect";
}
export interface ImageLayer extends LayerProps {
  type: "image";
  src: string;
}
export interface VideoLayer extends LayerProps {
  type: "video";
  src: string;
}
export interface TextLayer extends LayerProps {
  type: "text";
  text: string;
}
export interface CounterLayer extends LayerProps {
  type: "counter";
  value: Animated;
}
export interface CaptionsLayer extends LayerProps {
  type: "captions";
  src: string;
}
export interface WaveformLayer extends LayerProps {
  type: "waveform";
  src: string;
}
export interface GroupLayer extends LayerProps {
  type: "group";
  children?: Layer[];
}

export interface ShapeLayer extends LayerProps {
  type: "shape";
}
export type Layer = RectLayer | ImageLayer | VideoLayer | TextLayer | CounterLayer | CaptionsLayer | WaveformLayer | ShapeLayer | GroupLayer;

/** A flits scene: one video described as layers on a canvas. */
export interface Scene {
  /** URL or path of the JSON Schema (editor completion); ignored by flits */
  "$schema"?: string;
  /** canvas width in px */
  width: number;
  /** canvas height in px */
  height: number;
  /** frames per second (default 30) */
  fps?: number;
  /** seconds (default: the end of the last layer) */
  duration?: number;
  /** canvas colour behind all layers (default black) */
  background?: Color;
  /** font aliases usable in a layer's `font`, e.g. { brand: "fonts/Rubik-Black.ttf" } */
  fonts?: Record<string, string>;
  /** fonts for characters a layer's font lacks (emoji, other scripts), tried before the system's fallback fonts */
  fontFallback?: string[];
  /** soundtrack: music, voice-over, effects (audio of video layers is mixed in automatically) */
  audio?: AudioTrack[];
  /** normalize the final mix to this loudness (LUFS, e.g. -14 for social, -16 for podcasts) */
  loudness?: number;
  /** drawn in order: the first is at the bottom */
  layers?: Layer[];
  comment?: string;
}
