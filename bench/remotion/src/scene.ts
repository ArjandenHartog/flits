// The parts of the flits scene format (SPEC.md) that the benchmark uses, plus the same
// animation maths as flits (src/scene/anim.rs) so both renderers produce the same motion.

export type KeyTime = number | string;
export type AnimValue = number | [KeyTime, number, string?][] | { from: number; to: number; ease?: string; start?: KeyTime; end?: KeyTime };
export type TransitionSpec = string | { type?: string; duration?: number; ease?: string; distance?: number };

export interface Shadow {
  color?: string;
  blur?: number;
  x?: number;
  y?: number;
}

export interface Gradient {
  type?: "linear" | "radial";
  angle?: number;
  stops?: [number, string][];
  colors?: string[];
}

export interface Layer {
  type: string;
  start?: number;
  end?: number;
  duration?: number;
  x?: AnimValue;
  y?: AnimValue;
  width?: number;
  height?: number;
  anchor?: [number, number];
  scale?: AnimValue;
  scaleX?: AnimValue;
  scaleY?: AnimValue;
  rotation?: AnimValue;
  opacity?: AnimValue;
  zoom?: AnimValue;
  panX?: AnimValue;
  panY?: AnimValue;
  in?: TransitionSpec;
  out?: TransitionSpec;
  radius?: number;
  border?: { width: number; color: string };
  shadow?: Shadow;
  clip?: boolean;
  // image / video
  src?: string;
  fit?: "cover" | "contain" | "fill";
  blur?: number;
  trim?: number;
  volume?: number;
  speed?: number;
  loop?: boolean;
  // rect
  color?: string;
  gradient?: Gradient;
  // text
  text?: string;
  font?: string;
  weight?: number;
  size?: number;
  align?: "left" | "center" | "right";
  maxWidth?: number;
  lineHeight?: number;
  letterSpacing?: number;
  uppercase?: boolean;
  stroke?: { width: number; color: string };
  // group
  children?: Layer[];
  sequence?: boolean;
  overlap?: number;
}

export interface AudioTrack {
  src: string;
  start?: number;
  trim?: number;
  duration?: number;
  volume?: number;
  fadeIn?: number;
  fadeOut?: number;
  loop?: boolean;
}

export interface Scene {
  width: number;
  height: number;
  fps?: number;
  duration?: number;
  background?: string;
  audio?: AudioTrack[];
  layers: Layer[];
}

// ------------------------------------------------------------------ easing

export function ease(name: string | undefined, x: number): number {
  x = Math.min(1, Math.max(0, x));
  switch ((name ?? "in-out").toLowerCase()) {
    case "linear":
      return x;
    case "in":
    case "ease-in":
      return x * x * x;
    case "out":
    case "ease-out":
    case "ease":
      return 1 - Math.pow(1 - x, 3);
    case "smooth":
    case "sine":
    case "sine-in-out":
      return -(Math.cos(Math.PI * x) - 1) / 2;
    case "back-in": {
      const c1 = 1.70158;
      return (c1 + 1) * x * x * x - c1 * x * x;
    }
    case "back-out":
    case "back":
    case "overshoot": {
      const c1 = 1.70158;
      return 1 + (c1 + 1) * Math.pow(x - 1, 3) + c1 * Math.pow(x - 1, 2);
    }
    case "elastic":
      if (x === 0 || x === 1) return x;
      return Math.pow(2, -10 * x) * Math.sin((x * 10 - 0.75) * ((2 * Math.PI) / 3)) + 1;
    case "bounce": {
      const n1 = 7.5625, d1 = 2.75;
      if (x < 1 / d1) return n1 * x * x;
      if (x < 2 / d1) return n1 * (x -= 1.5 / d1) * x + 0.75;
      if (x < 2.5 / d1) return n1 * (x -= 2.25 / d1) * x + 0.9375;
      return n1 * (x -= 2.625 / d1) * x + 0.984375;
    }
    case "spring":
      return x >= 1 ? 1 : 1 - Math.exp(-6.5 * x) * Math.cos(11 * x);
    case "hold":
    case "step":
      return x >= 1 ? 1 : 0;
    default: // "in-out"
      return x < 0.5 ? 4 * x * x * x : 1 - Math.pow(-2 * x + 2, 3) / 2;
  }
}

function keyTime(k: KeyTime | undefined, dur: number, dflt: number): number {
  if (k === undefined) return dflt;
  if (typeof k === "number") return k;
  const s = k.trim().replace(/\s/g, "");
  if (s.startsWith("end")) return dur + (s.length > 3 ? parseFloat(s.slice(3)) : 0);
  return parseFloat(s);
}

/** Value of an animated property at `t` seconds into a layer that lasts `dur` seconds. */
export function evalProp(v: AnimValue | undefined, t: number, dur: number, dflt: number): number {
  if (v === undefined) return dflt;
  if (typeof v === "number") return v;
  let keys: [number, number, string | undefined][];
  if (Array.isArray(v)) {
    keys = v.map((k) => [keyTime(k[0], dur, 0), k[1], k[2]]);
  } else {
    keys = [
      [keyTime(v.start, dur, 0), v.from, "linear"],
      [keyTime(v.end, dur, dur), v.to, v.ease ?? "linear"],
    ];
  }
  if (t <= keys[0][0]) return keys[0][1];
  for (let i = 1; i < keys.length; i++) {
    const [t0, v0] = keys[i - 1];
    const [t1, v1, e] = keys[i];
    if (t < t1) {
      const p = ease(e, (t - t0) / Math.max(t1 - t0, 1e-9));
      return v0 + (v1 - v0) * p;
    }
  }
  return keys[keys.length - 1][1];
}

// ------------------------------------------------------------- transitions

export interface Fx {
  opacity: number;
  dx: number;
  dy: number;
  scale: number;
  /** CSS clip-path for wipes */
  clip?: string;
}

interface Transition {
  kind: string;
  dur: number;
  ease: string;
  dist: number;
}

function parseTransition(spec: TransitionSpec): Transition {
  let kind = "fade", dur: number | undefined, e: string | undefined, dist: number | undefined;
  if (typeof spec === "string") {
    const parts = spec.trim().split(/\s+/);
    kind = parts[0] || "fade";
    if (parts[1]) dur = parseFloat(parts[1]);
  } else {
    kind = spec.type ?? "fade";
    dur = spec.duration;
    e = spec.ease;
    dist = spec.distance;
  }
  const aliases: Record<string, string> = { up: "slide-up", down: "slide-down", left: "slide-left", right: "slide-right", zoom: "zoom-in", wipe: "wipe-right" };
  kind = aliases[kind] ?? kind;
  const defaultEase = kind === "pop" ? "back-out" : kind.startsWith("wipe") ? "in-out" : "out";
  return { kind, dur: Math.max(dur ?? (kind === "pop" ? 0.45 : 0.5), 0.001), ease: e ?? defaultEase, dist: dist ?? 120 };
}

function applyTransition(tr: Transition, p: number, raw: number, exit: boolean, fx: Fx) {
  const q = 1 - p;
  const dir = exit ? -1 : 1;
  const fade = Math.min(raw * 1.6, 1);
  const pct = (v: number) => `${(Math.min(1, Math.max(0, v)) * 100).toFixed(3)}%`;
  switch (tr.kind) {
    case "fade":
      fx.opacity *= Math.min(1, Math.max(0, p));
      break;
    case "slide-up":
      fx.dy += dir * q * tr.dist;
      fx.opacity *= fade;
      break;
    case "slide-down":
      fx.dy -= dir * q * tr.dist;
      fx.opacity *= fade;
      break;
    case "slide-left":
      fx.dx += dir * q * tr.dist;
      fx.opacity *= fade;
      break;
    case "slide-right":
      fx.dx -= dir * q * tr.dist;
      fx.opacity *= fade;
      break;
    case "zoom-in":
      fx.scale *= exit ? 1 + 0.15 * q : 0.85 + 0.15 * p;
      fx.opacity *= fade;
      break;
    case "zoom-out":
      fx.scale *= exit ? 1 - 0.15 * q : 1.15 - 0.15 * p;
      fx.opacity *= fade;
      break;
    case "pop":
      fx.scale *= Math.max(p, 0);
      fx.opacity *= Math.min(raw * 4, 1);
      break;
    // wipes: visible part of the box as an inset() clip
    case "wipe-left":
      fx.clip = exit ? `inset(0 ${pct(1 - p)} 0 0)` : `inset(0 0 0 ${pct(1 - p)})`;
      break;
    case "wipe-right":
      fx.clip = exit ? `inset(0 0 0 ${pct(1 - p)})` : `inset(0 ${pct(1 - p)} 0 0)`;
      break;
    case "wipe-up":
      fx.clip = exit ? `inset(0 0 ${pct(1 - p)} 0)` : `inset(${pct(1 - p)} 0 0 0)`;
      break;
    case "wipe-down":
      fx.clip = exit ? `inset(${pct(1 - p)} 0 0 0)` : `inset(0 0 ${pct(1 - p)} 0)`;
      break;
  }
}

/** Combined effect of a layer's `in` and `out` transitions at `t` seconds into the layer. */
export function transitionFx(l: Layer, t: number, dur: number): Fx {
  const fx: Fx = { opacity: 1, dx: 0, dy: 0, scale: 1 };
  if (l.in !== undefined) {
    const tr = parseTransition(l.in);
    if (t < tr.dur) {
      const raw = Math.min(1, Math.max(0, t / tr.dur));
      applyTransition(tr, ease(tr.ease, raw), raw, false, fx);
    }
  }
  if (l.out !== undefined) {
    const tr = parseTransition(l.out);
    const remaining = dur - t;
    if (remaining < tr.dur) {
      const raw = Math.min(1, Math.max(0, remaining / tr.dur));
      applyTransition(tr, ease(tr.ease, raw), raw, true, fx);
    }
  }
  return fx;
}

// ---------------------------------------------------------------- timeline

/** A layer with its absolute lifetime (seconds), resolved like flits' compose::build_list. */
export interface TimedLayer {
  layer: Layer;
  start: number;
  end: number;
  children: TimedLayer[];
}

function buildLayer(l: Layer, base: number, defaultEnd: number): TimedLayer {
  const start = base + (l.start ?? 0);
  let end = l.end !== undefined ? base + l.end : l.duration !== undefined ? start + l.duration : defaultEnd;
  end = Math.min(end, defaultEnd);
  let children: TimedLayer[] = [];
  if (l.type === "group") {
    children = buildList(l.children ?? [], !!l.sequence, l.overlap ?? 0, start, end);
    if (!Number.isFinite(end)) {
      end = children.map((c) => c.end).filter(Number.isFinite).reduce((a, b) => Math.max(a, b), start);
    }
  }
  return { layer: l, start, end, children };
}

function buildList(layers: Layer[], sequence: boolean, overlap: number, base: number, defaultEnd: number): TimedLayer[] {
  let cursor = 0;
  return layers.map((l) => {
    if (!sequence) return buildLayer(l, base, defaultEnd);
    const node = buildLayer({ ...l, start: cursor + (l.start ?? 0) }, base, defaultEnd);
    if (!Number.isFinite(node.end)) throw new Error(`layers inside a sequence need a "duration" (${l.type})`);
    cursor = node.end - base - overlap;
    return node;
  });
}

export function buildTimeline(scene: Scene): { nodes: TimedLayer[]; duration: number } {
  const nodes = buildList(scene.layers, false, 0, 0, scene.duration ?? Infinity);
  const duration =
    scene.duration ?? nodes.map((n) => n.end).filter(Number.isFinite).reduce((a, b) => Math.max(a, b), 0);
  if (!(duration > 0)) throw new Error('cannot determine the video length: set "duration" on the scene');
  const clamp = (n: TimedLayer) => {
    if (!Number.isFinite(n.end) || n.end > duration) n.end = duration;
    n.children.forEach(clamp);
  };
  nodes.forEach(clamp);
  return { nodes, duration };
}
