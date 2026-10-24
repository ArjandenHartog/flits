import type {
  AudioTrack,
  CaptionsLayer,
  CounterLayer,
  Ease,
  FromTo,
  GroupLayer,
  ImageLayer,
  KeyTime,
  Keyframe,
  Layer,
  RectLayer,
  Scene,
  ShapeLayer,
  TextLayer,
  VideoLayer,
  WaveformLayer,
} from "./scene.js";
import type { Children } from "./compile.js";

export { compile, flatten } from "./compile.js";
export type { Children } from "./compile.js";

/** Props of a component for layer type L: everything but `type`, plus `children` where it makes sense. */
export type LayerOptions<L extends Layer> = Omit<L, "type" | "children">;

// ------------------------------------------------------------------ formats

export interface Format {
  readonly width: number;
  readonly height: number;
}
/** Common canvas sizes: `<Composition {...formats.reel} fps={30}>`. */
export const formats: {
  readonly youtube: Format;
  readonly youtube4k: Format;
  readonly reel: Format;
  readonly short: Format;
  readonly tiktok: Format;
  readonly story: Format;
  readonly square: Format;
  readonly portrait: Format;
  readonly hd: Format;
};

// ------------------------------------------------------------------ components

export type CompositionProps = Omit<Scene, "layers"> & { layers?: Children; children?: Children };
/** The root: canvas, timing, soundtrack and the layers. Returns the scene (export it as default). */
export function Composition(props: CompositionProps): Scene;

export type TextProps = Omit<LayerOptions<TextLayer>, "text"> & { text?: string; children?: string | number | readonly (string | number)[] };
/** Text from `text` or the children: `<Text size={80}>Hello</Text>`. */
export function Text(props: TextProps): TextLayer;
/** Animated number. */
export function Counter(props: LayerOptions<CounterLayer>): CounterLayer;
/** Solid colour or gradient box. */
export function Rect(props?: LayerOptions<RectLayer>): RectLayer;
/** Photo or SVG. */
export function Image(props: LayerOptions<ImageLayer>): ImageLayer;
/** Video clip with sound. */
export function Video(props: LayerOptions<VideoLayer>): VideoLayer;
/** Word-by-word captions from a transcript. */
export function Captions(props: LayerOptions<CaptionsLayer>): CaptionsLayer;
/** Live audio spectrum bars. */
export function Waveform(props: LayerOptions<WaveformLayer>): WaveformLayer;
/** Vector shape: rect, ellipse, circle, line, arrow, polygon, star or SVG path. */
export function Shape(props?: LayerOptions<ShapeLayer>): ShapeLayer;

export type GroupProps = LayerOptions<GroupLayer> & { children?: Children };
/** Layers positioned and timed relative to this group. */
export function Group(props?: GroupProps): GroupLayer;
/** Children one after another (each needs a duration); `overlap` crossfades. */
export function Sequence(props?: Omit<GroupProps, "sequence">): GroupLayer;
/** Alias of Sequence. */
export const Series: typeof Sequence;
/** A full-size group starting at `time`: `<At time={2.5} duration={3}>...</At>`. */
export function At(props: GroupProps & { time?: number }): GroupLayer;
/** A soundtrack entry; may appear among the layers. */
export function Audio(props: AudioTrack): AudioTrack;
/** Just its children. */
export function Fragment(props: { children?: Children }): Layer[];

// ------------------------------------------------------------------ authoring helpers

/** Compile a scene object (functions sampled, children flattened). */
export function defineScene(scene: CompositionProps): Scene;
/** Type helper for a parametrized video; flits calls it with `--props`. */
export function defineVideo<P = Record<string, unknown>>(fn: (props: P) => Scene | Promise<Scene>): (props: P) => Scene | Promise<Scene>;
/** `{ from, to, ...options }` */
export function fromTo(from: number, to: number, options?: Omit<FromTo, "from" | "to">): FromTo;
/** Typed keyframes. */
export function keyframes(...keys: Keyframe[]): Keyframe[];
/** Seconds -> frames (rounded). */
export function frames(seconds: number, fps?: number): number;
/** Frames -> seconds. */
export function seconds(frames: number, fps?: number): number;

// ------------------------------------------------------------------ motion maths

export type EaseName =
  | "linear" | "in" | "out" | "in-out" | "smooth" | "back-in" | "back-out" | "elastic" | "bounce" | "spring" | "hold"
  | "expo-in" | "expo-out" | "expo-in-out" | "quint-out" | "quint-in-out" | "circ-out";
/** The scene format's easing curves as functions of 0..1. */
export const easing: Readonly<Record<EaseName, (x: number) => number>>;
/** A CSS-style cubic-bezier curve as a function of 0..1; in scene props use the string `"cubic-bezier(x1, y1, x2, y2)"`. */
export function cubicBezier(x1: number, y1: number, x2: number, y2: number): (x: number) => number;

export interface InterpolateOptions {
  /** per-segment easing (default linear) */
  ease?: EaseName | ((x: number) => number);
  /** "clamp" (default) or "extend" beyond the ranges */
  extrapolate?: "clamp" | "extend";
}
/** Map `input` from `inputRange` to `outputRange`, like Remotion's interpolate(). */
export function interpolate(input: number, inputRange: readonly number[], outputRange: readonly number[], options?: InterpolateOptions): number;

export interface SpringOptions {
  /** seconds (or give frame + fps) */
  t?: number;
  frame?: number;
  fps?: number;
  from?: number;
  to?: number;
  mass?: number;
  stiffness?: number;
  damping?: number;
  /** seconds before the spring starts */
  delay?: number;
}
/** Damped spring from `from` to `to`, like Remotion's spring(). */
export function spring(options: SpringOptions): number;
/** Deterministic pseudo-random number in 0..1 for a seed. */
export function random(seed: string | number): number;

export type { Ease, KeyTime };
