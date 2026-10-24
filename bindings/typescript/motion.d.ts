// Motion design components for flits (motion.js): masked type, eyebrows, drawing rules and boxes, counters,
// charts, whip-outs and a theme. Every component returns plain layers.
//
// Shared conventions:
// * `at`   seconds (relative to the parent group) at which the element starts coming in (default 0);
// * `exit` seconds (relative to the parent) at which it starts leaving; omit to stay until the parent ends;
// * `x`, `y` are the element's top-left corner (top-centre / top-right with `align`), in parent pixels,
//   unless a component says otherwise (rings: the centre);
// * colours, fonts and timings come from the theme (`setTheme(...)` once, or `theme` per component).

import type { Color, Ease, GroupLayer, Layer, RectLayer, Scene } from "./scene.js";
import type { Children } from "./compile.js";

// ------------------------------------------------------------------ theme

/** Colour roles of a theme. */
export interface ThemeColors {
  /** canvas colour (`themeScene()` sets it as the scene background) */
  background: Color;
  /** panels and fills a step above the background */
  surface: Color;
  /** primary text */
  text: Color;
  /** secondary text: eyebrows, labels, values */
  muted: Color;
  /** tertiary marks: baselines, inactive boxes, ruler ticks */
  subtle: Color;
  /** hairlines and outlines */
  line: Color;
  /** the one loud colour: rules, markers, highlighted bars, cursors */
  accent: Color;
  /** checks and good news */
  positive: Color;
  /** crosses and bad news */
  negative: Color;
}

/** Font roles: installed font names, font files or `fontFiles` aliases. */
export interface ThemeFonts {
  /** headlines, titles, big numbers */
  display: string;
  /** running text, subtitles */
  body: string;
  /** eyebrows, labels, data, code */
  mono: string;
}

/** Weights per font role (pick static weights such as "Segoe UI Semibold" for families, or set variable axes). */
export interface ThemeWeights {
  display: number;
  body: number;
  mono: number;
}

/** Default timing of entrances, exits and moves. */
export interface ThemeMotion {
  /** seconds an element takes to come in (default 0.9) */
  in: number;
  /** seconds an element takes to leave (default 0.55) */
  out: number;
  /** entrance ease (default "expo-out": lands fast, settles slowly) */
  ease: Ease;
  /** exit ease (default "expo-in": leaves accelerating) */
  exitEase: Ease;
  /** ease of things moving from one place to another (default "expo-in-out") */
  move: Ease;
  /** seconds between consecutive lines/rows (default 0.12) */
  stagger: number;
}

/** Everything that styles the motion components. */
export interface Theme {
  colors: ThemeColors;
  fonts: ThemeFonts;
  weights: ThemeWeights;
  /** font files by alias, e.g. `{ sans: "fonts/Inter.ttf" }` (relative to the scene file); `themeScene()` passes them to the scene's `fonts` */
  fontFiles: Record<string, string>;
  motion: ThemeMotion;
  /** canvas width the full-frame components (Hud, Whip, Act, EndCard, Grid) assume (default 1920) */
  width: number;
  /** canvas height (default 1080) */
  height: number;
  /** page margin in px (default 120) */
  margin: number;
  /** folder that relative `fontFiles` resolve against when measuring text (default: the scene file's folder) */
  baseDir?: string;
}

/** A partial theme: any subset of the keys, merged onto the current theme key by key. */
export type ThemeOverrides = {
  [K in keyof Theme]?: Theme[K] extends object ? Partial<Theme[K]> : Theme[K];
};

/** The default look: ink ground, warm white text, one red accent, Segoe UI Semibold + Consolas (look-alikes elsewhere). */
export const defaultTheme: Readonly<Theme>;
/** A theme: `base` (default: `defaultTheme`) with `overrides` merged in key by key. */
export function createTheme(overrides?: ThemeOverrides, base?: Theme): Theme;
/** Set the theme every motion component uses from now on (merged onto the default). Returns it. */
export function setTheme(overrides?: ThemeOverrides): Theme;
/** The theme currently in use. */
export function getTheme(): Theme;
/** Restore the default theme. */
export function resetTheme(): Theme;
/** `{ background, fonts }` for a theme: spread into `Composition` so the scene matches it. */
export function themeScene(theme?: ThemeOverrides): Pick<Scene, "background" | "fonts">;

/** `"#rrggbb"` + alpha (0..1) -> `"rgba(r,g,b,a)"`; other colour strings are returned unchanged. */
export function withAlpha(color: Color, alpha: number): Color;
/** Mix two `#rrggbb` colours: `k` 0 gives `a`, 1 gives `b`. */
export function mix(a: Color, b: Color, k: number): Color;

// ------------------------------------------------------------------ measuring

/** What to measure (the same keys as a text layer). */
export interface MeasureRequest {
  text: string;
  /** px (default 64) */
  size?: number;
  /** font name, file or `theme.fontFiles` alias (default: the theme's body font) */
  font?: string;
  weight?: number;
  letterSpacing?: number;
  uppercase?: boolean;
  /** wrap width in px */
  maxWidth?: number;
  /** default 1.15 */
  lineHeight?: number;
  /** balanced line breaks (with maxWidth) */
  balance?: boolean;
}

/** Layout of a measured text, in px. */
export interface TextMetrics {
  width: number;
  height: number;
  /** the text of each line after wrapping */
  lines: string[];
  lineWidths: number[];
  /** true when flits could not be run and the size is an estimate */
  estimated?: boolean;
}

/**
 * Measure text exactly as flits lays it out (runs `flits measure`, cached in memory and in the temp folder).
 * Falls back to an estimate when flits is not available (or `FLITS_MEASURE=estimate`).
 */
export function measureText(request: MeasureRequest, options?: { theme?: ThemeOverrides }): TextMetrics;
export function measureText(requests: MeasureRequest[], options?: { theme?: ThemeOverrides }): TextMetrics[];

// ------------------------------------------------------------------ shared props

/** When an element comes and goes. */
export interface Timing {
  /** seconds (relative to the parent) the element starts coming in (default 0) */
  at?: number;
  /** seconds (relative to the parent) the element starts leaving; omit to stay until the parent ends */
  exit?: number;
  /** theme overrides for this element only */
  theme?: ThemeOverrides;
}

/** Where an element sits. */
export interface Placement {
  /** px from the parent's left: the left edge (or centre / right edge with `align`) */
  x?: number;
  /** px from the parent's top: the top edge */
  y?: number;
}

export type HAlign = "left" | "center" | "right";

/** Type styling shared by the text components. */
export interface TypeProps {
  /** font size in px */
  size?: number;
  /** font name, file or alias (default: the theme font for the component's role) */
  font?: string;
  /** font weight (default: the theme weight for the role) */
  weight?: number;
  /** text colour (default: theme text) */
  color?: Color;
  /** extra px between characters */
  letterSpacing?: number;
  uppercase?: boolean;
  /** which side `x` refers to (default "left") */
  align?: HAlign;
}

// ------------------------------------------------------------------ text

export interface MaskLineProps extends Timing, Placement, TypeProps {
  /** the line (no wrapping) */
  text: string;
  /** line height factor; the mask is this plus a little room for descenders (default 1.15) */
  lineHeight?: number;
  /** seconds to slide in (default theme.motion.in, 0.9) */
  inDuration?: number;
  /** seconds to slide out (default theme.motion.out, 0.55) */
  outDuration?: number;
  /** entrance ease (default "expo-out") */
  ease?: Ease;
  /** exit ease (default "expo-in") */
  exitEase?: Ease;
  /** "up" (default: rises in, leaves upward) or "down" */
  direction?: "up" | "down";
  /** font role used for the defaults: "display" (default), "body" or "mono" */
  role?: "display" | "body" | "mono";
  /** extra opacity (constant or animated) on the whole line */
  opacity?: GroupLayer["opacity"];
  /** `false`: never motion-blurred */
  motionBlur?: boolean;
}
/** One line of text sliding up out of its own mask, and leaving upward through it on `exit`. */
export function MaskLine(props: MaskLineProps): GroupLayer;

export interface KineticTitleProps extends Timing, Placement, TypeProps {
  /** the headline; "\n" separates lines (or give `lines`) */
  text?: string;
  /** the lines, as an array */
  lines?: string[];
  /** wrap `text` to this width (balanced, measured by flits) instead of splitting on "\n" */
  maxWidth?: number;
  /** distance between line tops as a factor of `size` (default 1.1) */
  leading?: number;
  /** seconds between line entrances (default theme.motion.stagger, 0.12) */
  stagger?: number;
  /** seconds between line exits (default 0.05) */
  exitStagger?: number;
  /** seconds per line entrance (default 0.9) */
  inDuration?: number;
  /** seconds per line exit (default 0.55) */
  outDuration?: number;
  /** an eyebrow label above the title */
  eyebrow?: string;
  /** an accent square in front of the eyebrow */
  marker?: boolean;
  /** an accent rule under the title: `true` or its length in px */
  rule?: boolean | number;
  /** line index drawn in the accent colour (-1: the last line) */
  accentLine?: number;
}
/** A multi-line headline: lines mask in one after another (with optional eyebrow and accent rule) and leave line by line. Default size 120. */
export function KineticTitle(props: KineticTitleProps): GroupLayer;

export interface EyebrowProps extends Timing, Placement, TypeProps {
  text: string;
  /** an accent square in front of the text */
  marker?: boolean;
  /** colour of the marker (default theme accent) */
  markerColor?: Color;
  /** seconds to slide in (default 0.7) */
  inDuration?: number;
  /** seconds to slide out (default 0.55) */
  outDuration?: number;
}
/** Small tracked monospaced capitals (eyebrow, label, readout), masked in; default 20 px, muted, uppercase. */
export function Eyebrow(props: EyebrowProps): GroupLayer;
/** Alias of `Eyebrow`. */
export const Label: typeof Eyebrow;

export interface TypewriterProps extends Timing, Placement, Omit<TypeProps, "align" | "uppercase"> {
  /** the text; "\n" starts a new line */
  text: string;
  /** seconds to type everything (default: 0.045 s per character) */
  duration?: number;
  /** seconds per character when `duration` is not given (default 0.045) */
  speed?: number;
  /** line height factor (default 1.15) */
  lineHeight?: number;
  /** `false`: no cursor */
  cursor?: boolean;
  /** cursor colour (default theme accent) */
  cursorColor?: Color;
  /** cursor width as a factor of `size` (default 0.09) */
  cursorWidth?: number;
  /** seconds the cursor blinks after typing (default 4) */
  blink?: number;
}
/** Text typed out character by character (mono by default, 34 px) with a cursor that follows the last character, then blinks. */
export function Typewriter(props: TypewriterProps): GroupLayer;

/** `[phrase, seconds after at]`, or `{ text, at }`; a phrase may be a word or several words of the line. */
export type HighlightMark = [phrase: string, at: number] | { text: string; at: number };

export interface HighlightRuleProps extends Timing, Placement, TypeProps {
  /** the line */
  text: string;
  /** where the rule goes, and when (seconds relative to `at`) */
  marks: HighlightMark[];
  /** "rule" (default: an accent underline) or "marker" (a translucent highlighter box behind the words) */
  variant?: "rule" | "marker";
  /** colour of the rule / marker (default theme accent) */
  color?: Color;
  /** colour of the text (default theme text) */
  textColor?: Color;
  /** rule thickness in px (default 8.5% of `size`) */
  thickness?: number;
  /** opacity of the marker box (default 0.35) */
  markerOpacity?: number;
  /** seconds the rule takes to glide to the next phrase (default 0.5) */
  glide?: number;
}
/** A line of text with an accent rule that lands under a phrase and glides to the next one at each mark. Widths are measured by flits. */
export function HighlightRule(props: HighlightRuleProps): GroupLayer;

export interface QuoteProps extends Timing, Placement, Omit<TypeProps, "uppercase" | "letterSpacing"> {
  /** the quote (wrapped and balanced) */
  text: string;
  /** who said it */
  author?: string;
  /** their role or source, as an eyebrow under the name */
  role?: string;
  /** wrap width in px (default 1300) */
  maxWidth?: number;
  /** seconds over which the words appear (default 0.07 s per word, at most 2.2) */
  revealDuration?: number;
  /** font of the big quotation mark (default "Georgia") */
  quoteFont?: string;
}
/** A pull quote: accent quotation mark, words masking in one by one, then an attribution (accent dash, name, role). Default size 68. */
export function Quote(props: QuoteProps): GroupLayer;

// ------------------------------------------------------------------ structure

export interface RuleProps extends Timing, Placement {
  /** length in px (default 400) */
  length?: number;
  /** alias of `length` for horizontal rules */
  width?: number;
  /** px (default 1.5: a hairline) */
  thickness?: number;
  /** colour (default theme line) */
  color?: Color;
  /** draw downward instead of across */
  vertical?: boolean;
  /** where it grows from: "left" (default) / "right" / "center", or "top" / "bottom" / "center" when vertical */
  from?: "left" | "right" | "center" | "top" | "bottom";
  /** seconds to draw (default 1.0) */
  duration?: number;
  /** seconds to retract on exit (default 0.5) */
  outDuration?: number;
  /** draw ease (default "expo-out") */
  ease?: Ease;
  /** retract ease (default "expo-in") */
  exitEase?: Ease;
  /** opacity (constant or animated) */
  opacity?: RectLayer["opacity"];
  /** `false`: never motion-blurred */
  motionBlur?: boolean;
}
/** A hairline or accent rule that draws from one end (`x`, `y`: its start, centred on its thickness) and retracts on `exit`. */
export function Rule(props: RuleProps): RectLayer;

export interface OutlineProps extends Timing, Placement {
  /** px (default 400) */
  width?: number;
  /** px (default 240) */
  height?: number;
  /** stroke px (default 1.5) */
  thickness?: number;
  /** stroke colour (default theme line) */
  color?: Color;
  /** seconds to draw (default 1.2) */
  duration?: number;
  /** draw ease (default "expo-out") */
  ease?: Ease;
  /** only corner brackets of this length in px (a viewfinder) */
  corners?: number;
  /** a panel colour that fades in behind the outline */
  fill?: Color;
  /** anchor of x/y in the box (default [0, 0]: top-left) */
  anchor?: [number, number];
  /** `false`: never motion-blurred */
  motionBlur?: boolean;
}
/** A box whose outline draws itself on (or corner brackets), optionally over a fill. */
export function Outline(props: OutlineProps): GroupLayer;

export interface TagProps extends Timing, Placement, Omit<TypeProps, "align"> {
  text: string;
  /** which side `x` refers to (default "left") */
  align?: HAlign;
  /** horizontal padding in px (default 0.8 x size) */
  padding?: number;
  /** frame in the accent colour */
  accent?: boolean;
  /** frame colour (default theme muted, or accent) */
  borderColor?: Color;
  /** frame px (default 1.5) */
  thickness?: number;
  /** a fill colour inside the frame */
  fill?: Color;
}
/** An outlined chip around a short mono label, sized to the text: the frame draws on, the text masks in. Default 28 px. */
export function Tag(props: TagProps): GroupLayer;

export interface GridProps extends Timing, Placement {
  /** px (default: canvas width minus margins) */
  width?: number;
  /** px (default: canvas height minus margins) */
  height?: number;
  /** column count: columns + 1 vertical lines (default 4) */
  columns?: number;
  /** row count: rows + 1 horizontal lines (default 0) */
  rows?: number;
  /** line colour (default theme line) */
  color?: Color;
  /** line px (default 1.5) */
  thickness?: number;
  /** seconds between lines (default 0.06) */
  stagger?: number;
  /** seconds per line (default 1.4) */
  duration?: number;
  /** overall opacity */
  opacity?: GroupLayer["opacity"];
}
/** Hairline layout guides that draw on one after another (never motion-blurred). */
export function Grid(props?: GridProps): GroupLayer;

export interface HudProps {
  /** the film's length in seconds (the playhead and timecode run over it) */
  duration: number;
  /** seconds the HUD fades out and ends (default `duration`) */
  end?: number;
  /** seconds the HUD starts (default 0) */
  at?: number;
  /** label top-left */
  title?: string;
  /** label top-right */
  meta?: string;
  /** `[time, label]`: chapter names bottom-right, each replacing the previous (absolute times) */
  chapters?: [time: number, label: string][];
  /** `false`: no running timecode */
  timecode?: boolean;
  /** text before the timecode (default "T+ ") */
  timecodePrefix?: string;
  /** px between ruler ticks (default 20) */
  tickSpacing?: number;
  /** y of the top hairline (default 0.93 x margin) */
  top?: number;
  /** canvas size (default theme width/height) and margin */
  width?: number;
  height?: number;
  margin?: number;
  theme?: ThemeOverrides;
}
/** The instrument frame of a film: hairlines, a tick ruler with a travelling playhead, timecode, labels and chapter names. */
export function Hud(props: HudProps): GroupLayer;

export interface LowerThirdProps extends Timing, Placement {
  /** the name (big) */
  name: string;
  /** alias of `name` */
  title?: string;
  /** a role or place under it (eyebrow) */
  role?: string;
  /** name size in px (default 46) */
  size?: number;
  /** role size in px (default 20) */
  roleSize?: number;
  /** name font / weight / colour */
  font?: string;
  weight?: number;
  color?: Color;
  /** bar colour (default theme accent) */
  accent?: Color;
  /** bar width in px (default 5) */
  barWidth?: number;
}
/** A name strap: an accent bar grows, the name masks in beside it, the role follows. Default: bottom-left in the margin. */
export function LowerThird(props: LowerThirdProps): GroupLayer;

export interface EndCardProps {
  /** the closing line ("\n" for more lines), centred */
  title: string;
  /** an eyebrow above it */
  eyebrow?: string;
  /** a line under it (muted) */
  subtitle?: string;
  /** a mono line (URL, handle) */
  url?: string;
  /** seconds the card starts */
  at?: number;
  /** seconds the card lasts (needed for `fadeOut`) */
  duration?: number;
  /** fade to the background over the last seconds: `true` (0.8 s) or seconds */
  fadeOut?: boolean | number;
  /** `false`: no accent rule along the foot of the frame */
  rule?: boolean;
  /** title size (default 112) */
  size?: number;
  /** top of the title (default: centred) */
  y?: number;
  /** canvas size (default theme width/height) */
  width?: number;
  height?: number;
  theme?: ThemeOverrides;
}
/** The closing card: eyebrow, centred title, subtitle, URL, an accent rule along the foot and an optional fade out. */
export function EndCard(props: EndCardProps): GroupLayer;

export interface BackdropProps {
  /** ground colour (default theme background) */
  color?: Color;
  /** colour of the glow in the centre (default: the ground mixed 5% toward the text colour) */
  glow?: Color;
  /** how much lighter the glow is (0..1, default 0.05) */
  strength?: number;
  /** seconds it starts */
  at?: number;
  theme?: ThemeOverrides;
}
/** A soft radial glow over the ground colour, so a flat background reads as a lit surface. */
export function Backdrop(props?: BackdropProps): RectLayer;

// ------------------------------------------------------------------ data

export interface StatCounterProps extends Timing, Placement {
  /** the number counted to */
  value: number;
  /** counted from (default 0) */
  from?: number;
  /** digits after the point */
  decimals?: number;
  prefix?: string;
  suffix?: string;
  /** thousands separator: "," (default), "." or "none" */
  separator?: string;
  /** seconds of counting (default 1.8) */
  duration?: number;
  /** counting ease (default "expo-out") */
  ease?: Ease;
  /** a label under the number (eyebrow) */
  label?: string;
  /** label size (default 22) */
  labelSize?: number;
  /** `false`: no rule between number and label */
  rule?: boolean;
  /** rule length (default 2.6 x size) */
  ruleWidth?: number;
  /** rule colour */
  ruleColor?: Color;
  /** a thick accent rule instead of a hairline */
  accent?: boolean;
  /** number size (default 140) */
  size?: number;
  font?: string;
  weight?: number;
  color?: Color;
  align?: HAlign;
}
/** A big number counting to `value`, masked in, over a rule and a label. */
export function StatCounter(props: StatCounterProps): GroupLayer;

/** A bar: a number, or the value with a label. */
export type BarDatum = number | { value: number; label?: string; highlight?: boolean };

export interface BarChartProps extends Timing, Placement {
  data: BarDatum[];
  /** plot width in px (default 800; labels hang below) */
  width?: number;
  /** plot height in px = the tallest bar (default 400) */
  height?: number;
  /** value of a full-height bar (default: the largest value) */
  max?: number;
  /** gap between bars as a fraction of a slot (default 0.38) */
  gap?: number;
  /** index of the accent bar (default: the last; -1 or a missing index: none) */
  highlight?: number;
  /** bar colour (default theme muted) */
  color?: Color;
  /** seconds between bars (default 0.08) */
  stagger?: number;
  /** seconds a bar grows (default 1.0) */
  duration?: number;
  /** growth ease (default "expo-out") */
  ease?: Ease;
  /** `false`: no counting values over the bars */
  values?: boolean;
  /** value formatting */
  decimals?: number;
  prefix?: string;
  suffix?: string;
  /** value size (default 26) */
  valueSize?: number;
  /** label size (default 18) */
  labelSize?: number;
}
/** Vertical bars growing from a baseline one after another, values counting above them, mono labels below, one accent bar. */
export function BarChart(props: BarChartProps): GroupLayer;

export interface LineChartProps extends Timing, Placement {
  /** the values, evenly spaced left to right */
  data: (number | { value: number })[];
  /** px (default 800) */
  width?: number;
  /** px (default 300) */
  height?: number;
  /** value at the bottom / top (default: the data's min / max) */
  min?: number;
  max?: number;
  /** line px (default 3) */
  thickness?: number;
  /** line colour (default theme text) */
  color?: Color;
  /** `false`: straight segments instead of a smooth curve */
  smooth?: boolean;
  /** seconds to draw (default 2.2) */
  duration?: number;
  /** draw ease (default "in-out") */
  ease?: "linear" | "in" | "out" | "in-out" | "smooth" | "expo-out" | "expo-in-out" | "quint-out" | "quint-in-out";
  /** horizontal grid lines (default 3) */
  grid?: number;
  /** `false`: no dot riding the tip and no accent marker on the last point */
  marker?: boolean;
  /** marker px (default 14) */
  markerSize?: number;
  /** an eyebrow above the chart */
  label?: string;
}
/** A line chart drawing itself left to right, a dot riding the tip and landing as an accent marker with a pulse. */
export function LineChart(props: LineChartProps): GroupLayer;

export interface ProgressRingProps extends Timing {
  /** fill, 0..1 */
  value: number;
  /** centre, px */
  x?: number;
  y?: number;
  /** diameter in px (default 220) */
  size?: number;
  /** stroke px (default 3.5% of size, at least 4) */
  thickness?: number;
  /** ring colour (default theme text) */
  color?: Color;
  /** track colour (default theme line) */
  trackColor?: Color;
  /** seconds to fill (default 1.4) */
  duration?: number;
  ease?: Ease;
  /** `false`: no percentage in the centre */
  showValue?: boolean;
  /** a label under the ring */
  label?: string;
  /** label size (default 20) */
  labelSize?: number;
}
/** A ring filling to `value` over a hairline track, the percentage counting in the centre, a label below. */
export function ProgressRing(props: ProgressRingProps): GroupLayer;

/** A checklist row: a string, or the label with a right-aligned value and a status. */
export type ChecklistItem = string | { label: string; value?: string | number; status?: "ok" | "fail" | "pending"; valueColor?: Color };

export interface ChecklistProps extends Timing, Placement {
  items: ChecklistItem[];
  /** row width in px (values align to its right end; default 800) */
  width?: number;
  /** text size (default 28) */
  size?: number;
  /** px per row (default 2.1 x size) */
  rowHeight?: number;
  /** seconds between rows (default 0.28) */
  stagger?: number;
  /** `false`: no hairlines between rows */
  rules?: boolean;
  /** label font / weight / colour (default theme mono) */
  font?: string;
  weight?: number;
  color?: Color;
}
/** Rows of checks: each row masks in, its box draws and a check (or cross) strokes in; values sit right-aligned. */
export function Checklist(props: ChecklistProps): GroupLayer;

export interface ProgressBarsProps extends Timing, Placement {
  /** one row per item; `value` 0..1, `highlight` draws the fill in the accent colour */
  items: { label: string; value: number; highlight?: boolean }[];
  /** px (default 800) */
  width?: number;
  /** text size (default 24) */
  size?: number;
  /** px per row (default 1.9 x size) */
  rowHeight?: number;
  /** px for the label column (default 32% of width) */
  labelWidth?: number;
  /** px for the value column (default 3.6 x size) */
  valueWidth?: number;
  /** bar px (default 4) */
  thickness?: number;
  /** seconds between rows (default 0.22) */
  stagger?: number;
  /** seconds a fill sweeps (default 1.0) */
  duration?: number;
  /** fill colour (default theme text) */
  fillColor?: Color;
  font?: string;
  weight?: number;
  color?: Color;
}
/** Horizontal meters: label, hairline track, a fill sweeping to the value and the percentage counting at the right. */
export function ProgressBars(props: ProgressBarsProps): GroupLayer;

// ------------------------------------------------------------------ motion and timing

export interface WhipProps extends Timing {
  children?: Children;
  /** the way it leaves (default "left"); `enter` comes in from the opposite side */
  direction?: "left" | "right" | "up" | "down";
  /** px travelled (default 1.35 x the canvas side) */
  distance?: number;
  /** seconds of the whip out (default 0.85, "expo-in") */
  duration?: number;
  /** also whip in at `at` */
  enter?: boolean;
  /** seconds of the whip in (default 0.8, "expo-out") */
  enterDuration?: number;
  /** canvas size (default theme width/height) */
  width?: number;
  height?: number;
}
/** A full-frame group that whips out sideways at `exit` (pair with scene `motionBlur` for the streak) and optionally whips in. */
export function Whip(props: WhipProps): GroupLayer;

export interface StaggerProps {
  children?: Children;
  /** seconds between children (default 0.1) */
  each?: number;
  /** seconds added to the first child (default 0) */
  from?: number;
  /** last child first */
  reverse?: boolean;
}
/** Shifts each child's `start` (and `end`) by `from + i * each`: a list that enters one after another. Returns the children. */
export function Stagger(props: StaggerProps): Layer[];

export interface ActProps {
  children?: Children;
  /** absolute start in seconds (relative to the parent) */
  at?: number;
  /** seconds it lasts (or give `end`) */
  duration?: number;
  /** when it ends */
  end?: number;
  /** how it leaves at its end: "rise" (default: drifts up while fading), "sink", "fade", "whip" or "none" */
  exit?: "rise" | "sink" | "fade" | "whip" | "none";
  /** seconds of the exit (default 0.6; whip 0.85) */
  exitDuration?: number;
  /** px of drift for rise/sink (default 90) */
  distance?: number;
  /** `false`: never motion-blurred */
  motionBlur?: boolean;
  /** canvas size (default theme width/height) */
  width?: number;
  height?: number;
  theme?: ThemeOverrides;
}
/** A chapter on the timeline: a full-frame group from `at` for `duration` seconds with an exit. Spread a `timeline()` entry into it. */
export function Act(props: ActProps): GroupLayer;
/** Alias of `Act`. */
export const Chapter: typeof Act;

/** One act on a timeline. */
export interface TimelineEntry {
  at: number;
  end: number;
  duration: number;
}
/**
 * Acts laid end to end: `timeline({ intro: 5, demo: 8 })` -> `{ intro: { at: 0, end: 5, duration: 5 }, demo: { at: 5, ... },
 * total: 13 }`. `overlap`: seconds each act starts before the previous one ends; `start`: time of the first act.
 */
export function timeline<K extends string>(acts: Record<K, number>, options?: { overlap?: number; start?: number }): Record<K, TimelineEntry> & { total: number };
