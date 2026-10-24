// Turns a scene built in JavaScript/TypeScript into plain scene JSON:
// * animatable properties given as functions `(t, ctx) => number` are sampled once per frame into
//   linear keyframes (t = seconds since the layer starts), so any maths (springs, noise, physics,
//   data) animates exactly as computed;
// * `children`/`layers` may be nested arrays and contain null/false/undefined (conditional layers);
// * <Audio> elements among the layers move to the scene's `audio` list.
//
// Dependency-free and runtime-agnostic: flits embeds this file in the loader that runs .ts/.js scenes,
// and the SDK (index.js) uses it for scene objects passed to render()/still()/check().

/** Properties that can be animated (and therefore be functions). */
export const ANIMATABLE = ["x", "y", "scale", "scaleX", "scaleY", "rotation", "opacity", "zoom", "panX", "panY", "value", "progress", "volume"];

const AUDIO = Symbol.for("flits.audio");

/** Mark an object as a soundtrack entry (what <Audio> / Audio() return). */
export function audioTrack(track) {
  return Object.defineProperty({ ...track }, AUDIO, { value: true, enumerable: false });
}

const isAudio = (v) => v !== null && typeof v === "object" && v[AUDIO] === true;

/** Flatten nested arrays and drop null/undefined/false/true (so `cond && <Text/>` works). */
export function flatten(children) {
  const out = [];
  const walk = (c) => {
    if (Array.isArray(c)) c.forEach(walk);
    else if (c !== null && c !== undefined && c !== false && c !== true) out.push(c);
  };
  walk(children);
  return out;
}

/**
 * Sample `fn` over `duration` seconds at `fps`: linear keyframes, with points on straight stretches dropped.
 * `fn(t, { frame, fps, duration, progress })`.
 */
export function sample(fn, duration, fps, what = "property") {
  if (!(duration > 0) || !Number.isFinite(duration)) {
    throw new Error(`${what}: a function needs to know how long the layer lasts: give it (or its group, or the scene) a duration`);
  }
  const n = Math.max(1, Math.ceil(duration * fps - 1e-9));
  const pts = [];
  for (let frame = 0; frame <= n; frame++) {
    const t = Math.min(frame / fps, duration);
    const v = fn(t, { frame, fps, duration, progress: t / duration });
    if (typeof v !== "number" || !Number.isFinite(v)) {
      throw new Error(`${what}: the function returned ${String(v)} at t=${t.toFixed(3)}s (expected a finite number)`);
    }
    pts.push([round(t), round(v)]);
  }
  // keep a point only where the curve bends (or at the ends)
  const keep = [pts[0]];
  for (let i = 1; i < pts.length - 1; i++) {
    const [a, b, c] = [keep[keep.length - 1], pts[i], pts[i + 1]];
    const lin = a[1] + ((c[1] - a[1]) * (b[0] - a[0])) / (c[0] - a[0]);
    if (Math.abs(lin - b[1]) > 1e-4) keep.push(b);
  }
  if (pts.length > 1) keep.push(pts[pts.length - 1]);
  return keep.map(([t, v], i) => (i === 0 ? [t, v] : [t, v, "linear"]));
}

const round = (v) => Math.round(v * 1e6) / 1e6;

/** Seconds a layer lasts, if it can be known here (else undefined). */
function lifetime(layer, parentDuration) {
  if (typeof layer.duration === "number") return layer.duration;
  if (typeof layer.end === "number") return layer.end - (layer.start ?? 0);
  if (parentDuration !== undefined) return parentDuration - (layer.start ?? 0);
  return undefined;
}

function compileLayer(layer, parentDuration, fps, path) {
  if (layer === null || typeof layer !== "object" || Array.isArray(layer)) {
    throw new Error(`${path}: expected a layer object, got ${Array.isArray(layer) ? "an array" : typeof layer}`);
  }
  const out = {};
  const dur = lifetime(layer, parentDuration);
  for (const [k, v] of Object.entries(layer)) {
    if (v === undefined) continue;
    if (typeof v === "function") {
      if (!ANIMATABLE.includes(k)) throw new Error(`${path}.${k}: only animatable properties (${ANIMATABLE.join(", ")}) can be functions`);
      out[k] = sample(v, dur, fps, `${path}.${k}`);
    } else if (k === "children") {
      // children of a sequence each start when the previous one ends: their own duration is what counts
      const kids = flatten(v);
      out.children = kids.map((c, i) => compileLayer(c, layer.sequence ? undefined : dur, fps, `${path}.children[${i}]`));
    } else {
      out[k] = v;
    }
  }
  return out;
}

/** Plain scene JSON from a scene object that may use functions, nested arrays and Audio elements. */
export function compile(scene) {
  if (scene === null || typeof scene !== "object" || Array.isArray(scene)) {
    throw new Error("the scene must be an object like { width, height, layers } (or a function returning one)");
  }
  const fps = scene.fps ?? 30;
  const out = {};
  for (const [k, v] of Object.entries(scene)) if (v !== undefined && k !== "layers" && k !== "audio") out[k] = v;
  const items = flatten(scene.layers ?? []);
  const audio = [...flatten(scene.audio ?? []), ...items.filter(isAudio)];
  out.layers = items.filter((l) => !isAudio(l)).map((l, i) => compileLayer(l, scene.duration, fps, `layers[${i}]`));
  if (audio.length) {
    out.audio = audio.map((a, i) => {
      const t = { ...a };
      if (typeof t.volume === "function") {
        const d = t.duration ?? (scene.duration !== undefined ? scene.duration - (t.start ?? 0) : undefined);
        t.volume = sample(t.volume, d, fps, `audio[${i}].volume`);
      }
      return t;
    });
  }
  return out;
}
