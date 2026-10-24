// Building blocks for writing flits videos in JavaScript/TypeScript, in the spirit of Remotion:
// components are plain functions that take props and return layers, so videos are composed with
// functions, loops, conditionals and data. They work called directly (`Text({ text: "Hi" })`) and as
// JSX (`<Text>Hi</Text>`, with jsxImportSource pointing at this package). No dependencies, no DOM.
//
// Animatable props (x, y, scale, rotation, opacity, zoom, value, progress, volume, ...) also accept a
// function `(t, { frame, fps, duration, progress }) => number`; flits samples it once per frame.

import { audioTrack, compile, flatten } from "./compile.js";

export { compile, flatten };

// motion design components (KineticTitle, MaskLine, Rule, LineChart, Whip, ...) and the theme
export * from "./motion.js";

// ------------------------------------------------------------------ formats

/** Common canvas sizes. Spread into a Composition: `<Composition {...formats.reel} fps={30}>`. */
export const formats = Object.freeze({
  youtube: Object.freeze({ width: 1920, height: 1080 }),
  youtube4k: Object.freeze({ width: 3840, height: 2160 }),
  reel: Object.freeze({ width: 1080, height: 1920 }),
  short: Object.freeze({ width: 1080, height: 1920 }),
  tiktok: Object.freeze({ width: 1080, height: 1920 }),
  story: Object.freeze({ width: 1080, height: 1920 }),
  square: Object.freeze({ width: 1080, height: 1080 }),
  portrait: Object.freeze({ width: 1080, height: 1350 }),
  hd: Object.freeze({ width: 1280, height: 720 }),
});

// ------------------------------------------------------------------ components

const omit = (o, ...keys) => {
  const r = { ...o };
  for (const k of keys) delete r[k];
  return r;
};

const textOf = (children) =>
  flatten(children)
    .map((c) => (typeof c === "string" || typeof c === "number" ? String(c) : ""))
    .join("");

/** The root: canvas, timing, soundtrack and the layers (as `children` or `layers`). Returns a scene. */
export function Composition(props = {}) {
  const { children, layers, ...rest } = props;
  return compile({ ...rest, layers: flatten([layers ?? [], children ?? []]) });
}

/** Text. The text comes from `text` or the children: `<Text size={80}>Hello</Text>`. */
export function Text(props = {}) {
  const text = props.text ?? textOf(props.children);
  return { type: "text", ...omit(props, "children", "text"), text };
}

/** Animated number: `<Counter value={{ from: 0, to: 100 }} suffix="%"/>`. */
export function Counter(props = {}) {
  return { type: "counter", ...omit(props, "children") };
}

/** Solid colour or gradient box (fills its parent unless sized). */
export function Rect(props = {}) {
  return { type: "rect", ...omit(props, "children") };
}

/** Photo or SVG (`src` relative to the scene file). */
export function Image(props = {}) {
  return { type: "image", ...omit(props, "children") };
}

/** Video clip, with its sound (`volume: 0` mutes it). */
export function Video(props = {}) {
  return { type: "video", ...omit(props, "children") };
}

/** Word-by-word captions from a transcript (`flits transcribe` output, .srt or .vtt). */
export function Captions(props = {}) {
  return { type: "captions", ...omit(props, "children") };
}

/** Live audio spectrum bars. */
export function Waveform(props = {}) {
  return { type: "waveform", ...omit(props, "children") };
}

/** Vector shape: rect, ellipse, circle, line, arrow, polygon, star or SVG path (`d`). */
export function Shape(props = {}) {
  return { type: "shape", ...omit(props, "children") };
}

/** Layers positioned and timed relative to this group. */
export function Group(props = {}) {
  return { type: "group", ...omit(props, "children"), children: flatten([props.children ?? []]) };
}

/** Children one after another (each needs a `duration`; clips use their length); `overlap` crossfades. */
export function Sequence(props = {}) {
  return { ...Group(props), sequence: true };
}

/** Alias of Sequence (Remotion's name for back-to-back scenes). */
export const Series = Sequence;

/** Place children on the timeline: `<At time={2.5} duration={3}>...</At>` (a full-size group starting then). */
export function At(props = {}) {
  const { time, ...rest } = props;
  return Group({ ...rest, start: time ?? rest.start ?? 0 });
}

/** A soundtrack entry (may appear among the layers; it is moved to the scene's `audio`). */
export function Audio(props = {}) {
  return audioTrack(omit(props, "children"));
}

/** A fragment: just its children. */
export function Fragment(props = {}) {
  return flatten([props.children ?? []]);
}

// ------------------------------------------------------------------ authoring helpers

/** Type helper for a whole scene object (returns it unchanged after compiling functions). */
export function defineScene(scene) {
  return compile(scene);
}

/** Type helper for a parametrized video: `export default defineVideo((props: { title: string }) => ...)`. */
export function defineVideo(fn) {
  return fn;
}

/** Animate from one value to another over the layer, or between `start` and `end`. */
export function fromTo(from, to, options = {}) {
  return { from, to, ...options };
}

/** Keyframes from [time, value, ease?] tuples (a typed spelling of the plain array). */
export function keyframes(...keys) {
  return keys;
}

/** Seconds -> frames at `fps` (rounded). */
export const frames = (seconds, fps = 30) => Math.round(seconds * fps);

/** Frames -> seconds at `fps`. */
export const seconds = (frameCount, fps = 30) => frameCount / fps;

// ------------------------------------------------------------------ motion maths (for function props)

const C1 = 1.70158;

/** The easing curves of the scene format, as functions of x in 0..1. */
export const easing = Object.freeze({
  linear: (x) => x,
  in: (x) => x * x * x,
  out: (x) => 1 - (1 - x) ** 3,
  "in-out": (x) => (x < 0.5 ? 4 * x * x * x : 1 - (-2 * x + 2) ** 3 / 2),
  smooth: (x) => -(Math.cos(Math.PI * x) - 1) / 2,
  "back-in": (x) => (C1 + 1) * x * x * x - C1 * x * x,
  "back-out": (x) => 1 + (C1 + 1) * (x - 1) ** 3 + C1 * (x - 1) ** 2,
  elastic: (x) => (x === 0 || x === 1 ? x : 2 ** (-10 * x) * Math.sin((x * 10 - 0.75) * ((2 * Math.PI) / 3)) + 1),
  bounce: (x) => {
    const [n, d] = [7.5625, 2.75];
    if (x < 1 / d) return n * x * x;
    if (x < 2 / d) return n * (x -= 1.5 / d) * x + 0.75;
    if (x < 2.5 / d) return n * (x -= 2.25 / d) * x + 0.9375;
    return n * (x -= 2.625 / d) * x + 0.984375;
  },
  spring: (x) => (x >= 1 ? 1 : 1 - Math.exp(-6.5 * x) * Math.cos(11 * x)),
  hold: (x) => (x >= 1 ? 1 : 0),
  "expo-in": (x) => (x <= 0 ? 0 : 2 ** (10 * x - 10)),
  "expo-out": (x) => (x >= 1 ? 1 : 1 - 2 ** (-10 * x)),
  "expo-in-out": (x) => (x <= 0 || x >= 1 ? x : x < 0.5 ? 2 ** (20 * x - 10) / 2 : (2 - 2 ** (-20 * x + 10)) / 2),
  "quint-out": (x) => 1 - (1 - x) ** 5,
  "quint-in-out": (x) => (x < 0.5 ? 16 * x ** 5 : 1 - (-2 * x + 2) ** 5 / 2),
  "circ-out": (x) => Math.sqrt(1 - (x - 1) ** 2),
});

/** A CSS-style cubic-bezier curve as a function of 0..1 (for interpolate); in scene props use the string "cubic-bezier(x1, y1, x2, y2)". */
export function cubicBezier(x1, y1, x2, y2) {
  const at = (u, a, b) => 3 * (1 - u) ** 2 * u * a + 3 * (1 - u) * u * u * b + u ** 3;
  return (x) => {
    let [lo, hi, u] = [0, 1, x];
    for (let i = 0; i < 40; i++) {
      u = (lo + hi) / 2;
      if (at(u, x1, x2) < x) lo = u;
      else hi = u;
    }
    return at(u, y1, y2);
  };
}

/**
 * Map `input` from `inputRange` to `outputRange` (both ascending lists of equal length), like Remotion's
 * interpolate(). `ease`: a name from `easing` or a function, applied per segment. Clamped at both ends
 * unless `extrapolate: "extend"`.
 */
export function interpolate(input, inputRange, outputRange, options = {}) {
  if (inputRange.length !== outputRange.length || inputRange.length < 2) {
    throw new Error("interpolate: inputRange and outputRange need the same length (at least 2)");
  }
  const ease = typeof options.ease === "function" ? options.ease : easing[options.ease ?? "linear"];
  if (!ease) throw new Error(`interpolate: unknown ease '${options.ease}'`);
  const extend = options.extrapolate === "extend";
  const last = inputRange.length - 1;
  let i = 0;
  while (i < last - 1 && input > inputRange[i + 1]) i++;
  const [a, b] = [inputRange[i], inputRange[i + 1]];
  let x = b === a ? 1 : (input - a) / (b - a);
  if (!extend) x = Math.min(1, Math.max(0, x));
  const e = x >= 0 && x <= 1 ? ease(x) : x;
  return outputRange[i] + (outputRange[i + 1] - outputRange[i]) * e;
}

/**
 * A damped spring going from `from` (0) to `to` (1), like Remotion's spring(): `t` in seconds (or pass
 * `frame` and `fps`). Defaults: mass 1, stiffness 100, damping 10 (a little overshoot).
 */
export function spring({ t, frame, fps = 30, from = 0, to = 1, mass = 1, stiffness = 100, damping = 10, delay = 0 } = {}) {
  const time = (t ?? (frame ?? 0) / fps) - delay;
  if (time <= 0) return from;
  const w0 = Math.sqrt(stiffness / mass);
  const zeta = damping / (2 * Math.sqrt(stiffness * mass));
  let x; // remaining distance, 1 -> 0
  if (zeta < 1) {
    const wd = w0 * Math.sqrt(1 - zeta * zeta);
    x = Math.exp(-zeta * w0 * time) * (Math.cos(wd * time) + ((zeta * w0) / wd) * Math.sin(wd * time));
  } else {
    x = Math.exp(-w0 * time) * (1 + w0 * time);
  }
  return from + (to - from) * (1 - x);
}

/** Deterministic pseudo-random number in 0..1 for a seed (same value on every render). */
export function random(seed) {
  let h = 2166136261;
  for (const ch of String(seed)) h = Math.imul(h ^ ch.charCodeAt(0), 16777619);
  h = Math.imul(h ^ (h >>> 15), 2246822507);
  h = Math.imul(h ^ (h >>> 13), 3266489909);
  return ((h ^ (h >>> 16)) >>> 0) / 4294967296;
}
