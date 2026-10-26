// Motion design components for flits: the vocabulary of a polished brand film (masked line reveals,
// mono eyebrows, drawing hairlines, stroke-drawn boxes, counters, charts, whip-outs) as one-call
// components with good defaults. Everything here returns plain layers, so it mixes freely with the
// basic components (Text, Group, Shape, ...), JSX and hand-written JSON.
//
// Conventions shared by every component:
// * `at`   seconds (relative to the parent group) at which the element starts coming in (default 0);
// * `exit` seconds (relative to the parent) at which it starts leaving; omit it to stay until the parent ends;
// * `x`, `y` place the element's top-left corner (or top-centre / top-right with `align`), in parent pixels;
// * colours, fonts and timings come from the theme (`setTheme`, or a `theme` prop per component).
//
// Text widths (for rules under words, cursors, tags, lower thirds) are measured by the flits binary
// (`flits measure`; flits sets FLITS_BIN when it runs a script scene), with an estimate as fallback.

import { flatten } from "./compile.js";

// Node built-ins, only for measuring text; optional so the module also loads where they don't exist.
let childProcess = null;
let nodePath = null;
let nodeFs = null;
let nodeOs = null;
try {
  childProcess = await import("node:child_process");
  nodePath = await import("node:path");
  nodeFs = await import("node:fs");
  nodeOs = await import("node:os");
} catch {
  // no subprocesses here: widths are estimated
}

// ------------------------------------------------------------------ theme

/** The default look: ink ground, warm white text, one red accent, a clean sans and a mono. */
export const defaultTheme = Object.freeze({
  colors: Object.freeze({
    background: "#0B0B0C",
    surface: "#141415",
    text: "#F5F3EF",
    muted: "#8C8882",
    subtle: "#4A4743",
    line: "#2B2926",
    accent: "#FF4A2B",
    positive: "#3FB27F",
    negative: "#F0524D",
  }),
  fonts: Object.freeze({ display: "Segoe UI", body: "Segoe UI", mono: "Consolas" }),
  weights: Object.freeze({ display: 600, body: 400, mono: 400 }),
  fontFiles: Object.freeze({}),
  motion: Object.freeze({ in: 0.9, out: 0.55, ease: "expo-out", exitEase: "expo-in", move: "expo-in-out", stagger: 0.12 }),
  width: 1920,
  height: 1080,
  margin: 120,
  baseDir: undefined,
});

const isObj = (v) => v !== null && typeof v === "object" && !Array.isArray(v);

function merge(base, over) {
  if (!isObj(over)) return base;
  const out = { ...base };
  for (const [k, v] of Object.entries(over)) {
    if (v === undefined) continue;
    out[k] = isObj(v) && isObj(base[k]) ? merge(base[k], v) : v;
  }
  return out;
}

/** A theme: `defaultTheme` (or `base`) with `overrides` merged in, key by key. */
export function createTheme(overrides = {}, base = defaultTheme) {
  return merge(base, overrides);
}

let current = defaultTheme;

/** Set the theme every motion component uses from now on (merged onto the default). Returns it. */
export function setTheme(overrides = {}) {
  current = createTheme(overrides);
  return current;
}

/** The theme currently in use. */
export function getTheme() {
  return current;
}

/** Restore the default theme. */
export function resetTheme() {
  current = defaultTheme;
  return current;
}

/** Scene-level settings for a theme (`background` and `fonts`): spread them into `Composition`. */
export function themeScene(theme = current) {
  const t = theme === current ? current : createTheme(theme, current);
  const out = { background: t.colors.background };
  if (Object.keys(t.fontFiles ?? {}).length) out.fonts = { ...t.fontFiles };
  return out;
}

const themeOf = (p) => (p && p.theme ? createTheme(p.theme, current) : current);

// ------------------------------------------------------------------ small helpers

const ax = (align) => (align === "center" ? 0.5 : align === "right" ? 1 : 0);
const r2 = (v) => Math.round(v * 100) / 100;
const def = (o) => {
  const out = {};
  for (const [k, v] of Object.entries(o)) if (v !== undefined) out[k] = v;
  return out;
};

/** `"#rrggbb"` (or `#rgb`) + alpha -> `"rgba(r,g,b,a)"`. Other colour strings are returned unchanged. */
export function withAlpha(color, alpha) {
  const m = /^#([0-9a-f]{3}|[0-9a-f]{6})$/i.exec(String(color).trim());
  if (!m) return color;
  let h = m[1];
  if (h.length === 3) h = [...h].map((c) => c + c).join("");
  const [r, g, b] = [0, 2, 4].map((i) => parseInt(h.slice(i, i + 2), 16));
  return `rgba(${r},${g},${b},${alpha})`;
}

/** Mix two `#rrggbb` colours (`k` = 0 gives `a`, 1 gives `b`). */
export function mix(a, b, k) {
  const p = (c) => {
    let h = String(c).replace("#", "");
    if (h.length === 3) h = [...h].map((x) => x + x).join("");
    return [0, 2, 4].map((i) => parseInt(h.slice(i, i + 2), 16));
  };
  const [x, y] = [p(a), p(b)];
  return "#" + x.map((v, i) => Math.round(v + (y[i] - v) * k).toString(16).padStart(2, "0")).join("");
}

/** Keyframes for an enter (0 -> `inDur`) and optional exit (from `exitAt`, `outDur` long), local times. */
function inOut(from, to, inDur, ease, exitAt, outDur, exitTo, exitEase) {
  const k = [[0, from], [r2(inDur), to, ease]];
  if (exitAt !== undefined) {
    const s = Math.max(exitAt, inDur);
    k.push([r2(s), to], [r2(s + outDur), exitTo, exitEase]);
  }
  return k;
}

// ------------------------------------------------------------------ measuring text

const measureCache = new Map();
// measurements persist between runs (a preview reload re-runs the script): a small JSON file in the temp folder
let diskCache = null;
const diskPath = () => (nodeOs && nodePath ? nodePath.join(nodeOs.tmpdir(), "flits-measure-cache.json") : null);
function loadDisk() {
  if (diskCache) return diskCache;
  diskCache = {};
  try {
    const p = diskPath();
    if (p && nodeFs.existsSync(p)) diskCache = JSON.parse(nodeFs.readFileSync(p, "utf8")) ?? {};
  } catch {
    diskCache = {};
  }
  return diskCache;
}
function saveDisk() {
  try {
    const p = diskPath();
    const keys = Object.keys(diskCache);
    if (keys.length > 5000) for (const k of keys.slice(0, keys.length - 5000)) delete diskCache[k];
    if (p) nodeFs.writeFileSync(p, JSON.stringify(diskCache));
  } catch {
    // a cache only
  }
}

function sceneDir(t) {
  if (t.baseDir) return t.baseDir;
  try {
    const argv = globalThis.process?.argv ?? [];
    const loader = argv.findIndex((a) => /loader\.mjs$/.test(a));
    if (loader >= 0 && argv[loader + 1] && nodePath) return nodePath.dirname(argv[loader + 1]);
    if (globalThis.Deno?.args?.[0] && nodePath) return nodePath.dirname(globalThis.Deno.args[0]);
  } catch {
    // fall through
  }
  return globalThis.process?.cwd?.() ?? ".";
}

/** Rough width of `text` when flits can't be asked: per-character widths of a typical sans or mono. */
function estimate(req) {
  const mono = /mono|consol|courier|code/i.test(String(req.font ?? ""));
  const text = req.uppercase ? String(req.text).toUpperCase() : String(req.text);
  const line = (s) => {
    let w = 0;
    for (const ch of s) {
      let k;
      if (mono) k = 0.55;
      else if (ch === " ") k = 0.27;
      else if ("iljI.,:;'!|ft()[]".includes(ch)) k = 0.3;
      else if ("mwMW".includes(ch)) k = 0.86;
      else if (/[A-Z]/.test(ch)) k = 0.66;
      else if (/[0-9]/.test(ch)) k = 0.56;
      else k = 0.53;
      w += k * req.size + (req.letterSpacing ?? 0);
    }
    return w;
  };
  const lines = text.split("\n");
  const widths = lines.map(line);
  return { width: Math.max(0, ...widths), height: lines.length * req.size * (req.lineHeight ?? 1.15), lines, lineWidths: widths, estimated: true };
}

/**
 * Measure text the way flits lays it out: `{ width, height, lines, lineWidths }` per request (pixels).
 * `font` may be a font name, a file or a `theme.fontFiles` alias. Uses `flits measure` (cached); falls back to
 * an estimate (`estimated: true`) when flits is unavailable or `FLITS_MEASURE=estimate`.
 */
export function measureText(requests, options = {}) {
  const t = themeOf(options);
  const one = !Array.isArray(requests);
  const reqs = (one ? [requests] : requests).map((r) => ({
    text: String(r.text ?? ""),
    size: r.size ?? 64,
    font: r.font ?? t.fonts.body,
    weight: r.weight,
    variations: r.variations,
    letterSpacing: r.letterSpacing ?? 0,
    uppercase: !!r.uppercase,
    maxWidth: r.maxWidth,
    lineHeight: r.lineHeight ?? 1.15,
    balance: r.balance ?? false,
  }));
  const env = globalThis.process?.env ?? {};
  const estimateOnly = !childProcess || env.FLITS_MEASURE === "estimate";
  const key = (r) => JSON.stringify([r, t.fontFiles[r.font] ?? null, t.fontFiles[r.font] ? sceneDir(t) : null, estimateOnly]);
  if (!estimateOnly) {
    const disk = loadDisk();
    for (const r of reqs) if (!measureCache.has(key(r)) && disk[key(r)]) measureCache.set(key(r), disk[key(r)]);
  }
  const todo = reqs.filter((r) => !measureCache.has(key(r)));
  if (todo.length) {
    let results = null;
    if (!estimateOnly) {
      const dir = sceneDir(t);
      const wire = todo.map((r) => {
        const file = t.fontFiles?.[r.font];
        return def({ ...r, font: file ? (nodePath ? nodePath.resolve(dir, file) : file) : r.font });
      });
      try {
        const res = childProcess.spawnSync(env.FLITS_BIN || "flits", ["measure"], { input: JSON.stringify(wire), encoding: "utf8", windowsHide: true });
        if (res.status === 0) results = JSON.parse(res.stdout);
      } catch {
        results = null;
      }
    }
    todo.forEach((r, i) => measureCache.set(key(r), results?.[i] ?? estimate(r)));
    if (results) {
      todo.forEach((r, i) => (diskCache[key(r)] = results[i]));
      saveDisk();
    }
  }
  const out = reqs.map((r) => measureCache.get(key(r)));
  return one ? out[0] : out;
}

// ------------------------------------------------------------------ layer builders (plain objects)

const text = (o) => ({ type: "text", ...def(o) });
const rect = (o) => ({ type: "rect", ...def(o) });
const shape = (o) => ({ type: "shape", ...def(o) });
const group = (o) => ({ type: "group", ...def(o), children: flatten([o.children ?? []]) });
const counter = (o) => ({ type: "counter", ...def(o) });

/** Font + weight for a role ("display", "body", "mono"), unless given. */
const face = (t, role, p) => ({ font: p.font ?? t.fonts[role], weight: p.weight ?? (p.font ? undefined : t.weights[role]) });

/** start/end of an element that enters at `at` and leaves at `exit` over `outDur`. */
const life = (at, exit, outDur) => def({ start: at || undefined, end: exit !== undefined ? r2(exit + outDur) : undefined });

const WIDE = 8000; // width of a mask box: wide enough for any line, clipping only vertically

// ------------------------------------------------------------------ text

/**
 * One line of text that slides up out of its own mask (and leaves upward through it on `exit`): the classic
 * kinetic-type line. Returns a clipped group.
 */
export function MaskLine(p = {}) {
  const t = themeOf(p);
  const size = p.size ?? 64;
  const a = ax(p.align);
  const lh = p.lineHeight ?? 1.15;
  const box = size * (lh + 0.22);
  const inD = p.inDuration ?? t.motion.in;
  const outD = p.outDuration ?? t.motion.out;
  const dir = p.direction === "down" ? -1 : 1;
  const exitAt = p.exit !== undefined ? p.exit - (p.at ?? 0) : undefined;
  return group({
    ...life(p.at, p.exit, outD),
    clip: true,
    x: p.x ?? 0,
    y: p.y ?? 0,
    width: WIDE,
    height: r2(box),
    anchor: [a, 0],
    opacity: p.opacity,
    motionBlur: p.motionBlur,
    children: [
      text({
        text: String(p.text ?? ""),
        ...face(t, p.role ?? "display", p),
        size,
        color: p.color ?? t.colors.text,
        letterSpacing: p.letterSpacing,
        uppercase: p.uppercase,
        lineHeight: p.lineHeight,
        align: p.align ?? "left",
        anchor: [a, 0],
        x: a * WIDE,
        y: inOut(r2(box * dir), 0, inD, p.ease ?? t.motion.ease, exitAt, outD, r2(-box * dir), p.exitEase ?? t.motion.exitEase),
      }),
    ],
  });
}

/**
 * A multi-line headline: each line masks in after the previous (`stagger`), optionally under an eyebrow and over
 * an accent rule, and leaves line by line on `exit`. `text` is split on "\n" (or give `lines`); with `maxWidth` it
 * wraps (balanced) as flits would.
 */
export function KineticTitle(p = {}) {
  const t = themeOf(p);
  const size = p.size ?? 120;
  const leading = p.leading ?? 1.1;
  const stagger = p.stagger ?? t.motion.stagger;
  const exitStagger = p.exitStagger ?? 0.05;
  const outD = p.outDuration ?? t.motion.out;
  const f = face(t, "display", p);
  let lines = p.lines ?? String(p.text ?? "").split("\n");
  if (!p.lines && p.maxWidth) {
    lines = measureText({ text: String(p.text ?? ""), size, ...f, letterSpacing: p.letterSpacing, uppercase: p.uppercase, maxWidth: p.maxWidth, balance: true }, p).lines;
  }
  const x = p.x ?? t.margin;
  let y = p.y ?? t.margin;
  const kids = [];
  let delay = 0;
  if (p.eyebrow) {
    kids.push(Eyebrow({ text: p.eyebrow, x, y, align: p.align, at: 0, exit: p.exit !== undefined ? p.exit - (p.at ?? 0) : undefined, theme: p.theme, marker: p.marker }));
    y += 56;
    delay = 0.15;
  }
  lines.forEach((line, i) => {
    const accent = p.accentLine === i || p.accentLine === -1 && i === lines.length - 1;
    kids.push(
      MaskLine({
        text: line,
        x,
        y: r2(y + i * size * leading),
        size,
        align: p.align,
        font: f.font,
        weight: f.weight,
        color: accent ? t.colors.accent : p.color,
        letterSpacing: p.letterSpacing,
        uppercase: p.uppercase,
        at: r2(delay + i * stagger),
        exit: p.exit !== undefined ? r2(p.exit - (p.at ?? 0) + i * exitStagger) : undefined,
        inDuration: p.inDuration,
        outDuration: outD,
        theme: p.theme,
      }),
    );
  });
  if (p.rule) {
    const ruleW = typeof p.rule === "number" ? p.rule : Math.min(760, size * 5.5);
    const ry = y + lines.length * size * leading + size * 0.32;
    kids.push(
      Rule({
        x: r2(x - ax(p.align) * ruleW + (p.align === "left" || !p.align ? size * 0.03 : 0)),
        y: r2(ry),
        length: ruleW,
        thickness: Math.max(4, Math.round(size * 0.075)),
        color: t.colors.accent,
        at: r2(delay + lines.length * stagger + 0.35),
        duration: 1.1,
        exit: p.exit !== undefined ? r2(p.exit - (p.at ?? 0)) : undefined,
        theme: p.theme,
      }),
    );
  }
  const end = p.exit !== undefined ? r2(p.exit + (lines.length - 1) * exitStagger + outD) : undefined;
  return group({ start: p.at || undefined, end, children: kids });
}

/**
 * Small, tracked, monospaced capitals: an eyebrow over a title, a label, a readout. `marker: true` puts a small
 * accent square in front.
 */
export function Eyebrow(p = {}) {
  const t = themeOf(p);
  const size = p.size ?? 20;
  const spacing = p.letterSpacing ?? Math.round(size * 0.16 * 10) / 10;
  const f = face(t, "mono", p);
  const line = MaskLine({
    text: String(p.text ?? ""),
    x: p.x,
    y: p.y,
    size,
    align: p.align,
    ...f,
    color: p.color ?? t.colors.muted,
    letterSpacing: spacing,
    uppercase: p.uppercase ?? true,
    at: p.at,
    exit: p.exit,
    inDuration: p.inDuration ?? 0.7,
    outDuration: p.outDuration,
    theme: p.theme,
  });
  if (!p.marker) return line;
  // the marker sits left of the text: shift the text right by marker + gap (left alignment)
  const m = Math.round(size * 0.5);
  const gap = Math.round(size * 0.7);
  let mx = p.x ?? 0;
  if (p.align === "center" || p.align === "right") {
    const w = measureText({ text: String(p.text ?? ""), size, ...f, letterSpacing: spacing, uppercase: p.uppercase ?? true }, p).width;
    mx -= ax(p.align) * w + m + gap;
  } else {
    line.x = (p.x ?? 0) + m + gap;
  }
  const exitAt = p.exit !== undefined ? p.exit - (p.at ?? 0) : undefined;
  const { start: _s, end: _e, ...inner } = line;
  if (exitAt !== undefined) inner.end = r2(exitAt + (p.outDuration ?? t.motion.out));
  return group({
    ...life(p.at, p.exit, p.outDuration ?? t.motion.out),
    children: [
      inner,
      rect({
        x: r2(mx),
        y: r2((p.y ?? 0) + size * 0.575),
        width: m,
        height: m,
        anchor: [0, 0.5],
        color: p.markerColor ?? t.colors.accent,
        scale: inOut(0, 1, 0.5, "back-out", exitAt, 0.3, 0, "in"),
      }),
    ],
  });
}

/** Alias of `Eyebrow`. */
export const Label = Eyebrow;

/**
 * Text typed out character by character, with a cursor that follows the last character and blinks afterwards.
 * Monospaced by default.
 */
export function Typewriter(p = {}) {
  const t = themeOf(p);
  const size = p.size ?? 34;
  const str = String(p.text ?? "");
  const f = face(t, "mono", p);
  const lh = p.lineHeight ?? 1.15;
  const chars = [...str].filter((c) => !/\s/.test(c)).length;
  const dur = p.duration ?? Math.max(0.3, chars * (p.speed ?? 0.045));
  const x = p.x ?? 0;
  const y = p.y ?? 0;
  const kids = [
    text({ text: str, ...f, size, color: p.color ?? t.colors.text, letterSpacing: p.letterSpacing, lineHeight: p.lineHeight, align: "left", anchor: [0, 0], x, y, reveal: "typewriter", revealDuration: r2(dur) }),
  ];
  if (p.cursor !== false && chars > 0) {
    // the cursor sits after each typed character: measure every prefix of each line once
    const lines = str.split("\n");
    const reqs = [];
    const where = [];
    lines.forEach((line, li) => {
      [...line].forEach((ch, ci) => {
        if (/\s/.test(ch)) return;
        reqs.push({ text: [...line].slice(0, ci + 1).join(""), size, ...f, letterSpacing: p.letterSpacing });
        where.push(li);
      });
    });
    const widths = measureText(reqs, p).map((m) => m.width);
    const step = dur / chars;
    const cw = Math.max(2, Math.round(size * (p.cursorWidth ?? 0.09)));
    const xs = [[0, r2(x)]];
    const ys = [[0, r2(y + size * lh * 0.5)]];
    widths.forEach((w, i) => {
      xs.push([r2(i * step), r2(x + w + size * 0.08), "hold"]);
      ys.push([r2(i * step), r2(y + size * lh * (where[i] + 0.5)), "hold"]);
    });
    const blink = p.blink ?? 4;
    const op = [[0, 1]];
    for (let k = 0; k < Math.floor(blink / 0.5); k++) op.push([r2(dur + 0.3 + k * 0.5), k % 2 === 0 ? 0 : 1, "hold"]);
    op.push([r2(dur + 0.3 + Math.floor(blink / 0.5) * 0.5), 1, "hold"]);
    kids.push(rect({ x: xs, y: ys, width: cw, height: r2(size * 1.0), anchor: [0, 0.5], color: p.cursorColor ?? t.colors.accent, opacity: op, motionBlur: false }));
  }
  return group({ ...life(p.at, p.exit, 0.3), opacity: p.exit !== undefined ? [[r2(p.exit - (p.at ?? 0)), 1], [r2(p.exit - (p.at ?? 0) + 0.3), 0, "linear"]] : undefined, children: kids });
}

/**
 * A line of text with an accent rule that lands under one word or phrase and glides to the next as the story
 * moves on. `marks`: `[phrase, time]` pairs (time relative to `at`). `variant: "marker"` draws a highlighter box
 * behind the words instead of a rule.
 */
export function HighlightRule(p = {}) {
  const t = themeOf(p);
  const size = p.size ?? 64;
  const str = String(p.text ?? "");
  const f = face(t, "display", p);
  const ls = p.letterSpacing;
  const marks = (p.marks ?? []).map((m) => (Array.isArray(m) ? m : [m.text, m.at]));
  // positions: x of a phrase = width of the line up to it minus nothing (layout sums word widths + spaces)
  const spans = marks.map(([phrase]) => {
    const i = typeof phrase === "number" ? null : str.indexOf(String(phrase));
    if (i === null || i < 0) throw new Error(`HighlightRule: "${phrase}" is not in "${str}"`);
    return [str.slice(0, i + String(phrase).length).trimEnd(), String(phrase).trim()];
  });
  const reqs = [{ text: str, size, ...f, letterSpacing: ls }];
  for (const [upto, phrase] of spans) reqs.push({ text: upto, size, ...f, letterSpacing: ls }, { text: phrase, size, ...f, letterSpacing: ls });
  const m = measureText(reqs, p);
  const total = m[0].width;
  const x0 = (p.x ?? 0) - ax(p.align) * total;
  const boxes = spans.map((_, i) => {
    const end = m[1 + i * 2].width;
    const w = m[2 + i * 2].width;
    return [x0 + end - w, w];
  });
  const y = p.y ?? 0;
  const marker = p.variant === "marker";
  const thick = p.thickness ?? (marker ? size * 0.95 : Math.max(4, Math.round(size * 0.085)));
  const ry = marker ? y + size * 0.6 : y + size * (1.15 + 0.12);
  const glide = p.glide ?? 0.5;
  const exitAt = p.exit !== undefined ? p.exit - (p.at ?? 0) : undefined;
  const kids = [];
  if (boxes.length) {
    const t0 = marks[0][1];
    const pad = marker ? size * 0.12 : 0;
    // the rect is as wide as the widest phrase and scaled down to each (a 1 px rect scaled up has soft ends)
    const base = Math.max(...boxes.map((b) => b[1])) + 2 * pad;
    const k = (i) => r2((boxes[i][1] + 2 * pad) / base);
    const xs = [[0, r2(boxes[0][0] - pad)]];
    const ws = [[0, 0], [0.7, k(0), "expo-out"]];
    for (let i = 1; i < boxes.length; i++) {
      const s = marks[i][1] - t0;
      xs.push([r2(s - 0.02), r2(boxes[i - 1][0] - pad)], [r2(s + glide), r2(boxes[i][0] - pad), t.motion.move]);
      ws.push([r2(s - 0.02), k(i - 1)], [r2(s + glide), k(i), t.motion.move]);
    }
    if (exitAt !== undefined) {
      const e = exitAt - t0;
      ws.push([r2(Math.max(e, ws[ws.length - 1][0])), ws[ws.length - 1][1]], [r2(Math.max(e, ws[ws.length - 1][0]) + 0.45), 0, t.motion.exitEase]);
    }
    const ruleLayer = rect({
      start: r2(t0),
      anchor: [0, 0.5],
      x: xs,
      y: r2(ry),
      width: r2(base),
      height: r2(thick),
      scaleX: ws,
      color: marker ? withAlpha(p.color ?? t.colors.accent, p.markerOpacity ?? 0.35) : p.color ?? t.colors.accent,
    });
    kids.push(ruleLayer);
  }
  const line = MaskLine({ text: str, x: p.x, y, size, align: p.align, ...f, color: p.textColor, letterSpacing: ls, exit: exitAt, theme: p.theme });
  if (marker) kids.push(line);
  else kids.unshift(line);
  return group({ ...life(p.at, p.exit, t.motion.out), children: kids });
}

/**
 * A pull quote: an accent quotation mark, the words masking in one by one (balanced lines), and an attribution
 * (accent dash, name, role) that follows.
 */
export function Quote(p = {}) {
  const t = themeOf(p);
  const size = p.size ?? 68;
  const str = String(p.text ?? "");
  const maxWidth = p.maxWidth ?? 1300;
  const f = face(t, "display", p);
  const align = p.align ?? "left";
  const a = ax(align);
  const x = p.x ?? t.margin + 80;
  const y = p.y ?? 300;
  const lh = 1.18;
  const m = measureText({ text: str, size, ...f, maxWidth, balance: true, lineHeight: lh }, p);
  const words = str.split(/\s+/).filter(Boolean).length;
  const revealDur = p.revealDuration ?? Math.min(2.2, words * 0.07);
  const attrY = y + m.height + size * 0.7;
  const exitAt = p.exit !== undefined ? p.exit - (p.at ?? 0) : undefined;
  const fadeOut = exitAt !== undefined ? [[0, 1], [r2(exitAt), 1], [r2(exitAt + 0.5), 0, "in"]] : undefined;
  const kids = [
    text({
      text: "“",
      font: p.quoteFont ?? "Georgia",
      weight: 700,
      size: size * 2.6,
      color: t.colors.accent,
      anchor: [a, 0],
      x: r2(align === "left" ? x - size * 0.12 : x),
      y: r2(y - size * 1.95),
      in: { type: "slide-fade", duration: 0.8, ease: "expo-out", distance: 30 },
    }),
    text({
      text: str,
      ...f,
      size,
      color: p.color ?? t.colors.text,
      maxWidth,
      wrap: "balance",
      align,
      lineHeight: lh,
      anchor: [a, 0],
      x,
      y,
      start: 0.25,
      reveal: "words",
      revealStyle: "mask",
      revealDuration: r2(revealDur),
    }),
  ];
  const t1 = 0.25 + revealDur + 0.1;
  if (p.author) {
    const dash = 48;
    kids.push(
      Rule({ x: r2(align === "left" ? x : x - a * dash), y: r2(attrY + 14), length: dash, thickness: 3, color: t.colors.accent, at: r2(t1), duration: 0.7, theme: p.theme }),
      MaskLine({ text: p.author, x: r2(align === "left" ? x + dash + 22 : x), y: r2(align === "left" ? attrY : attrY + 34), size: 30, align, ...face(t, "body", {}), weight: 600, at: r2(t1 + 0.1), theme: p.theme }),
    );
    if (p.role) kids.push(Eyebrow({ text: p.role, x: r2(align === "left" ? x + dash + 22 : x), y: r2(align === "left" ? attrY + 44 : attrY + 80), align, at: r2(t1 + 0.22), theme: p.theme }));
  }
  return group({ ...life(p.at, p.exit, 0.5), opacity: fadeOut, children: kids });
}

// ------------------------------------------------------------------ structure

/**
 * A hairline (or a thick accent rule) that draws from one end: `from` "left" (default), "right" or "center";
 * `vertical: true` draws downward (`from` "top"/"bottom"/"center"). On `exit` it retracts toward its far end.
 */
export function Rule(p = {}) {
  const t = themeOf(p);
  const len = p.length ?? p.width ?? 400;
  const th = p.thickness ?? 1.5;
  const v = !!p.vertical;
  const from = p.from ?? (v ? "top" : "left");
  const k = from === "center" ? 0.5 : from === "right" || from === "bottom" ? 1 : 0;
  const dur = p.duration ?? 1.0;
  const outD = p.outDuration ?? 0.5;
  const exitAt = p.exit !== undefined ? p.exit - (p.at ?? 0) : undefined;
  const s = inOut(0, 1, dur, p.ease ?? t.motion.ease, exitAt, outD, 0, p.exitEase ?? t.motion.exitEase);
  const x = p.x ?? 0;
  const y = p.y ?? 0;
  return rect({
    ...life(p.at, p.exit, outD),
    x: v ? x : r2(x + k * len),
    y: v ? r2(y + k * len) : y,
    width: v ? th : len,
    height: v ? len : th,
    anchor: v ? [0.5, k] : [k, 0.5],
    color: p.color ?? t.colors.line,
    [v ? "scaleY" : "scaleX"]: s,
    opacity: p.opacity,
    motionBlur: p.motionBlur,
  });
}

/**
 * A box whose outline draws itself on (clockwise from the top-left). `corners: 24` draws only corner brackets of
 * that length (a viewfinder); `fill` adds a panel colour that fades in behind it.
 */
export function Outline(p = {}) {
  const t = themeOf(p);
  const w = p.width ?? 400;
  const h = p.height ?? 240;
  const th = p.thickness ?? 1.5;
  const dur = p.duration ?? 1.2;
  const exitAt = p.exit !== undefined ? p.exit - (p.at ?? 0) : undefined;
  const opacity = exitAt !== undefined ? [[0, 1], [r2(exitAt), 1], [r2(exitAt + 0.4), 0, "in"]] : undefined;
  const stroke = { width: th, color: p.color ?? t.colors.line };
  const progress = { from: 0, to: 1, end: dur, ease: p.ease ?? t.motion.ease };
  const kids = [];
  if (p.fill) kids.push(rect({ color: p.fill, opacity: [[0, 0], [r2(dur * 0.6), 1, "out"]] }));
  if (p.corners) {
    const c = Math.min(p.corners, w / 2, h / 2);
    const i = th / 2;
    const d = [
      `M ${i} ${c} L ${i} ${i} L ${c} ${i}`,
      `M ${w - c} ${i} L ${w - i} ${i} L ${w - i} ${c}`,
      `M ${w - i} ${h - c} L ${w - i} ${h - i} L ${w - c} ${h - i}`,
      `M ${c} ${h - i} L ${i} ${h - i} L ${i} ${h - c}`,
    ].join(" ");
    kids.push(shape({ shape: "path", d, stroke, lineCap: "butt", progress }));
  } else {
    kids.push(shape({ shape: "rect", stroke, lineCap: "butt", progress }));
  }
  return group({
    ...life(p.at, p.exit, 0.4),
    x: p.x ?? 0,
    y: p.y ?? 0,
    width: w,
    height: h,
    anchor: p.anchor ?? [0, 0],
    opacity,
    motionBlur: p.motionBlur,
    children: kids,
  });
}

/**
 * An outlined chip around a short mono label ("ISO 27001", "NEW", "v2.0"): the frame draws on, the text masks in.
 * Sized to the text.
 */
export function Tag(p = {}) {
  const t = themeOf(p);
  const size = p.size ?? 28;
  const f = face(t, "mono", p);
  const str = String(p.text ?? "");
  const padX = p.padding ?? Math.round(size * 0.8);
  const w = measureText({ text: str, size, ...f, letterSpacing: p.letterSpacing, uppercase: p.uppercase }, p).width;
  const bw = Math.round(w + padX * 2);
  const bh = Math.round(size * 2);
  const x = (p.x ?? 0) - ax(p.align) * bw;
  const y = p.y ?? 0;
  const exitAt = p.exit !== undefined ? p.exit - (p.at ?? 0) : undefined;
  return group({
    ...life(p.at, p.exit, 0.5),
    children: [
      Outline({ x: r2(x), y, width: bw, height: bh, color: p.borderColor ?? (p.accent ? t.colors.accent : t.colors.muted), thickness: p.thickness ?? 1.5, duration: 0.9, fill: p.fill, exit: exitAt, theme: p.theme }),
      MaskLine({ text: str, x: r2(x + padX), y: r2(y + (bh - size * 1.15) / 2 - size * 0.02), size, ...f, color: p.color ?? t.colors.text, letterSpacing: p.letterSpacing, uppercase: p.uppercase, at: 0.15, exit: exitAt, inDuration: 0.7, theme: p.theme }),
    ],
  });
}

/**
 * Hairline layout guides: `columns` vertical and `rows` horizontal lines over a box (default: the canvas inside the
 * theme margin), drawing on one after another.
 */
export function Grid(p = {}) {
  const t = themeOf(p);
  const x = p.x ?? t.margin;
  const y = p.y ?? t.margin;
  const w = p.width ?? t.width - 2 * x;
  const h = p.height ?? t.height - 2 * y;
  const cols = p.columns ?? 4;
  const rows = p.rows ?? 0;
  const stagger = p.stagger ?? 0.06;
  const color = p.color ?? t.colors.line;
  const exitAt = p.exit !== undefined ? p.exit - (p.at ?? 0) : undefined;
  const kids = [];
  let i = 0;
  for (let c = 0; c <= cols; c++) {
    kids.push(Rule({ vertical: true, x: r2(x + (w * c) / cols), y, length: h, color, thickness: p.thickness, at: r2(i++ * stagger), duration: p.duration ?? 1.4, from: c % 2 ? "bottom" : "top", exit: exitAt, theme: p.theme }));
  }
  for (let r = 0; r <= rows && rows > 0; r++) {
    kids.push(Rule({ x, y: r2(y + (h * r) / rows), length: w, color, thickness: p.thickness, at: r2(i++ * stagger), duration: p.duration ?? 1.4, from: r % 2 ? "right" : "left", exit: exitAt, theme: p.theme }));
  }
  return group({ ...life(p.at, p.exit, 0.5), motionBlur: false, opacity: p.opacity, children: kids });
}

/**
 * The instrument frame of a film: hairlines top and bottom, a tick ruler with a playhead travelling with the video,
 * a running timecode, a title and meta label, and chapter names that change at given times. Needs `duration`.
 * Never motion-blurred.
 */
export function Hud(p = {}) {
  const t = themeOf(p);
  const W = p.width ?? t.width;
  const H = p.height ?? t.height;
  const m = p.margin ?? t.margin;
  const dur = p.duration ?? 10;
  const end = p.end ?? dur;
  const top = p.top ?? Math.round(m * 0.93);
  const bottom = H - top;
  const span = W - 2 * m;
  const ticks = [];
  const tick = p.tickSpacing ?? 20;
  for (let x = 0; x <= span + 0.01; x += tick) {
    const n = Math.round(x / tick);
    const hgt = n % 10 === 0 ? 18 : n % 5 === 0 ? 12 : 6;
    ticks.push(`M ${r2(x)} 0 L ${r2(x)} ${hgt}`);
  }
  const chapters = p.chapters ?? [];
  const at = p.at ?? 0;
  const kids = [
    Rule({ x: m, y: top, length: span, at: 0.2, duration: 1.4, theme: p.theme }),
    Rule({ x: m, y: bottom, length: span, at: 0.3, duration: 1.4, from: "right", theme: p.theme }),
    shape({
      shape: "path",
      d: ticks.join(" "),
      start: 0.5,
      x: m,
      y: bottom + 2,
      width: span,
      height: 20,
      anchor: [0, 0],
      stroke: { width: 1.5, color: t.colors.subtle },
      lineCap: "butt",
      progress: { from: 0, to: 1, end: 1.6, ease: t.motion.ease },
    }),
    rect({
      start: 0.9,
      x: [[0, m], [r2(end - at - 0.9), r2(m + span * ((end - 0.9) / dur)), "linear"]],
      y: bottom + 10,
      width: 3,
      height: 26,
      color: t.colors.accent,
      opacity: [[0, 0], [0.4, 1, "linear"]],
    }),
  ];
  if (p.title) kids.push(Eyebrow({ text: p.title, x: m, y: top - 48, at: 0.5, theme: p.theme }));
  if (p.meta) kids.push(Eyebrow({ text: p.meta, x: W - m, y: top - 48, at: 0.65, align: "right", theme: p.theme }));
  if (p.timecode !== false) {
    kids.push(
      counter({
        start: 0.8,
        value: { from: r2(at + 0.8), to: r2(end), ease: "linear" },
        decimals: 2,
        prefix: p.timecodePrefix ?? "T+ ",
        suffix: " s",
        separator: ",",
        ...face(t, "mono", {}),
        size: 20,
        color: t.colors.muted,
        align: "left",
        anchor: [0, 0],
        x: m,
        y: bottom + 40,
        opacity: [[0, 0], [0.5, 1, "linear"]],
      }),
    );
  }
  chapters.forEach(([ct, label], i) => {
    const next = chapters[i + 1];
    kids.push(Eyebrow({ text: label, x: W - m, y: bottom + 38, at: r2(ct - at), exit: next ? r2(next[0] - at - 0.3) : undefined, align: "right", theme: p.theme }));
  });
  return group({
    start: at || undefined,
    end: r2(end - at),
    motionBlur: false,
    opacity: [[r2(Math.max(0, end - at - 0.6)), 1], [r2(end - at), 0, "out"]],
    children: kids,
  });
}

/**
 * A name strap: an accent bar grows, the name masks in beside it and the role follows as an eyebrow.
 * Placed bottom-left inside the theme margin by default.
 */
export function LowerThird(p = {}) {
  const t = themeOf(p);
  const size = p.size ?? 46;
  const x = p.x ?? t.margin;
  const y = p.y ?? t.height - t.margin - size * 2.6;
  const barH = Math.round(size * 2.3);
  const gap = Math.round(size * 0.55);
  const exitAt = p.exit !== undefined ? p.exit - (p.at ?? 0) : undefined;
  const kids = [
    rect({
      x,
      y,
      width: p.barWidth ?? 5,
      height: barH,
      anchor: [0, 0],
      color: p.accent ?? t.colors.accent,
      scaleY: inOut(0, 1, 0.6, t.motion.ease, exitAt !== undefined ? exitAt + 0.15 : undefined, 0.4, 0, t.motion.exitEase),
    }),
    MaskLine({ text: p.name ?? p.title ?? "", x: x + gap, y: r2(y - size * 0.06), size, ...face(t, "display", p), color: p.color, at: 0.15, exit: exitAt, theme: p.theme }),
  ];
  if (p.role) kids.push(Eyebrow({ text: p.role, x: x + gap, y: r2(y + size * 1.42), size: p.roleSize ?? 20, at: 0.3, exit: exitAt, theme: p.theme }));
  return group({ ...life(p.at, p.exit, 0.6), children: kids });
}

/**
 * The closing card: eyebrow, a centred title, a mono line (URL, handle), an accent rule along the foot of the frame
 * and, with `fadeOut`, a fade to the background over the last `fadeOut` seconds (give `duration`).
 */
export function EndCard(p = {}) {
  const t = themeOf(p);
  const W = p.width ?? t.width;
  const H = p.height ?? t.height;
  const size = p.size ?? 112;
  const cx = W / 2;
  const lines = String(p.title ?? "").split("\n");
  const titleH = lines.length * size * 1.1;
  const top = p.y ?? Math.round(H / 2 - titleH / 2 - 60);
  const kids = [];
  if (p.eyebrow) kids.push(Eyebrow({ text: p.eyebrow, x: cx, y: top - 70, align: "center", size: 22, letterSpacing: 6, at: 0.1, theme: p.theme }));
  kids.push(KineticTitle({ lines, x: cx, y: top, size, align: "center", at: 0.2, theme: p.theme }));
  let y = top + titleH + 50;
  if (p.subtitle) {
    kids.push(MaskLine({ text: p.subtitle, x: cx, y, size: 40, align: "center", ...face(t, "body", {}), color: t.colors.muted, at: 0.55, theme: p.theme }));
    y += 76;
  }
  if (p.url) kids.push(MaskLine({ text: p.url, x: cx, y: y + 10, size: 36, align: "center", ...face(t, "mono", {}), color: t.colors.text, at: 0.75, theme: p.theme }));
  if (p.rule !== false) {
    kids.push(rect({ start: 0.6, x: 0, y: H - 4.5, width: W, height: 9, anchor: [0, 0.5], color: t.colors.accent, scaleX: { from: 0, to: 1, end: 1.4, ease: t.motion.move } }));
  }
  if (p.fadeOut && p.duration) {
    const f = typeof p.fadeOut === "number" ? p.fadeOut : 0.8;
    kids.push(rect({ start: r2(p.duration - f), color: t.colors.background, opacity: { from: 0, to: 1, ease: "in" } }));
  }
  return group({ start: p.at || undefined, end: p.duration ? r2((p.at ?? 0) + p.duration) : undefined, children: kids });
}

/** A soft radial glow over the background colour, so a flat ground reads as a lit surface. */
export function Backdrop(p = {}) {
  const t = themeOf(p);
  const bg = p.color ?? t.colors.background;
  const glow = p.glow ?? mix(bg, t.colors.text, p.strength ?? 0.05);
  return rect({ start: p.at || undefined, gradient: { type: "radial", colors: [glow, bg] }, motionBlur: false });
}

// ------------------------------------------------------------------ data

/**
 * A big number that counts up (or down) to `value`, masked in, with an eyebrow label under a hairline.
 */
export function StatCounter(p = {}) {
  const t = themeOf(p);
  const size = p.size ?? 140;
  const a = ax(p.align);
  const x = p.x ?? 0;
  const y = p.y ?? 0;
  const dur = p.duration ?? 1.8;
  const exitAt = p.exit !== undefined ? p.exit - (p.at ?? 0) : undefined;
  const box = size * 1.37;
  const outD = t.motion.out;
  const kids = [
    group({
      clip: true,
      x,
      y,
      width: WIDE,
      height: r2(box),
      anchor: [a, 0],
      children: [
        counter({
          value: { from: p.from ?? 0, to: p.value ?? 0, end: dur, ease: p.ease ?? t.motion.ease },
          decimals: p.decimals,
          prefix: p.prefix,
          suffix: p.suffix,
          separator: p.separator,
          ...face(t, "display", p),
          size,
          color: p.color ?? t.colors.text,
          align: p.align ?? "left",
          anchor: [a, 0],
          x: a * WIDE,
          y: inOut(r2(box), 0, t.motion.in, t.motion.ease, exitAt, outD, r2(-box), t.motion.exitEase),
        }),
      ],
    }),
  ];
  const ruleW = p.ruleWidth ?? Math.round(size * 2.6);
  if (p.rule !== false) {
    kids.push(Rule({ x: r2(x - a * ruleW), y: r2(y + size * 1.22), length: ruleW, from: p.align === "right" ? "right" : p.align === "center" ? "center" : "left", color: p.ruleColor ?? (p.accent ? t.colors.accent : t.colors.subtle), thickness: p.accent ? 4 : 1.5, at: 0.3, exit: exitAt, theme: p.theme }));
  }
  if (p.label) kids.push(Eyebrow({ text: p.label, x, y: r2(y + size * (p.rule === false ? 1.2 : 1.32)), align: p.align, size: p.labelSize ?? 22, at: 0.45, exit: exitAt, theme: p.theme }));
  return group({ ...life(p.at, p.exit, outD + 0.1), children: kids });
}

const values = (data) => data.map((d) => (typeof d === "number" ? { value: d } : d));

/**
 * Vertical bars that grow from a baseline one after another, each with its value counting above it and a mono label
 * beneath. One bar (`highlight` index, default the last) is drawn in the accent colour.
 */
export function BarChart(p = {}) {
  const t = themeOf(p);
  const data = values(p.data ?? []);
  const x = p.x ?? 0;
  const y = p.y ?? 0;
  const w = p.width ?? 800;
  const h = p.height ?? 400;
  const n = Math.max(1, data.length);
  const gap = p.gap ?? 0.38;
  const slot = w / n;
  const bw = slot * (1 - gap);
  const max = p.max ?? Math.max(...data.map((d) => d.value), 1e-9);
  const stagger = p.stagger ?? 0.08;
  const dur = p.duration ?? 1.0;
  const hi = p.highlight === undefined ? n - 1 : p.highlight;
  const ease = p.ease ?? t.motion.ease;
  const exitAt = p.exit !== undefined ? p.exit - (p.at ?? 0) : undefined;
  const base = y + h;
  const kids = [Rule({ x: r2(x - 12), y: base, length: w + 24, color: t.colors.subtle, at: 0, duration: 1.0, exit: exitAt, theme: p.theme })];
  data.forEach((d, i) => {
    const cx = x + slot * (i + 0.5);
    const bh = (Math.max(0, d.value) / max) * h;
    const s = r2(0.2 + i * stagger);
    const accent = d.highlight ?? i === hi;
    const color = accent ? t.colors.accent : p.color ?? t.colors.muted;
    const sY = inOut(0, 1, dur, ease, exitAt !== undefined ? exitAt - s : undefined, 0.45, 0, t.motion.exitEase);
    kids.push(rect({ start: s, x: r2(cx), y: r2(base), width: r2(bw), height: r2(Math.max(bh, 1)), anchor: [0.5, 1], color, scaleY: sY }));
    if (p.values !== false) {
      const vy = (k) => r2(base - bh * k - 14);
      kids.push(
        counter({
          start: s,
          value: { from: 0, to: d.value, end: dur, ease },
          decimals: p.decimals,
          prefix: p.prefix,
          suffix: p.suffix,
          ...face(t, "mono", {}),
          size: p.valueSize ?? 26,
          color: accent ? t.colors.text : t.colors.muted,
          align: "center",
          anchor: [0.5, 1],
          x: r2(cx),
          y: inOut(vy(0), vy(1), dur, ease, exitAt !== undefined ? exitAt - s : undefined, 0.45, vy(0), t.motion.exitEase),
          opacity: exitAt !== undefined ? [[0, 0], [0.2, 1, "linear"], [r2(exitAt - s), 1], [r2(exitAt - s + 0.3), 0, "linear"]] : [[0, 0], [0.2, 1, "linear"]],
        }),
      );
    }
    if (d.label !== undefined) {
      kids.push(Eyebrow({ text: String(d.label), x: r2(cx), y: r2(base + 18), align: "center", size: p.labelSize ?? 18, color: accent ? t.colors.text : undefined, at: s, exit: exitAt, theme: p.theme }));
    }
  });
  return group({ ...life(p.at, p.exit, 0.6), children: kids });
}

/** Catmull-Rom through points -> cubic bezier segments `[p0, c1, c2, p1]`. */
function smoothSegments(pts) {
  const segs = [];
  for (let i = 0; i < pts.length - 1; i++) {
    const p0 = pts[Math.max(0, i - 1)];
    const p1 = pts[i];
    const p2 = pts[i + 1];
    const p3 = pts[Math.min(pts.length - 1, i + 2)];
    const c1 = [p1[0] + (p2[0] - p0[0]) / 6, p1[1] + (p2[1] - p0[1]) / 6];
    const c2 = [p2[0] - (p3[0] - p1[0]) / 6, p2[1] - (p3[1] - p1[1]) / 6];
    segs.push([p1, c1, c2, p2]);
  }
  return segs;
}

const bez = ([a, b, c, d], u) => {
  const v = 1 - u;
  return [0, 1].map((k) => v * v * v * a[k] + 3 * v * v * u * b[k] + 3 * v * u * u * c[k] + u * u * u * d[k]);
};

/**
 * A line chart that draws itself from left to right, a dot riding the tip of the line and landing as an accent
 * marker on the last point, over a baseline and optional grid lines.
 */
export function LineChart(p = {}) {
  const t = themeOf(p);
  const data = (p.data ?? []).map((d) => (typeof d === "number" ? d : d.value));
  const x = p.x ?? 0;
  const y = p.y ?? 0;
  const w = p.width ?? 800;
  const h = p.height ?? 300;
  const th = p.thickness ?? 3;
  const dur = p.duration ?? 2.2;
  const ease = p.ease ?? "in-out";
  const lo = p.min ?? Math.min(...data);
  const hi = p.max ?? Math.max(...data);
  const range = hi - lo || 1;
  const pad = th + 2;
  const pts = data.map((v, i) => [r2((i / Math.max(1, data.length - 1)) * w), r2(pad + (1 - (v - lo) / range) * (h - 2 * pad))]);
  const smooth = p.smooth !== false && pts.length > 2;
  // dense polyline of the drawn curve (for the riding dot: arc-length parametrised like the stroke)
  let poly = pts;
  let d;
  if (smooth) {
    const segs = smoothSegments(pts);
    d = `M ${pts[0][0]} ${pts[0][1]} ` + segs.map(([, c1, c2, e]) => `C ${r2(c1[0])} ${r2(c1[1])} ${r2(c2[0])} ${r2(c2[1])} ${e[0]} ${e[1]}`).join(" ");
    poly = [pts[0]];
    for (const s of segs) for (let k = 1; k <= 16; k++) poly.push(bez(s, k / 16));
  } else {
    d = pts.map(([px, py], i) => `${i ? "L" : "M"} ${px} ${py}`).join(" ");
  }
  const lens = [0];
  for (let i = 1; i < poly.length; i++) lens.push(lens[i - 1] + Math.hypot(poly[i][0] - poly[i - 1][0], poly[i][1] - poly[i - 1][1]));
  const total = lens[lens.length - 1] || 1;
  const at = (s) => {
    let i = 1;
    while (i < lens.length - 1 && lens[i] < s) i++;
    const k = (s - lens[i - 1]) / (lens[i] - lens[i - 1] || 1);
    return [poly[i - 1][0] + (poly[i][0] - poly[i - 1][0]) * k, poly[i - 1][1] + (poly[i][1] - poly[i - 1][1]) * k];
  };
  const easeFn = EASES[ease] ?? EASES["in-out"];
  const N = 48;
  const dx = [];
  const dy = [];
  for (let k = 0; k <= N; k++) {
    const [px, py] = at(easeFn(k / N) * total);
    const tt = r2((k / N) * dur);
    dx.push(k ? [tt, r2(x + px), "linear"] : [0, r2(x + px)]);
    dy.push(k ? [tt, r2(y + py), "linear"] : [0, r2(y + py)]);
  }
  const exitAt = p.exit !== undefined ? p.exit - (p.at ?? 0) : undefined;
  const kids = [];
  const grid = p.grid ?? 3;
  for (let g = 1; g <= grid; g++) {
    kids.push(Rule({ x, y: r2(y + (h * (g - 1)) / grid), length: w, color: t.colors.line, thickness: 1, at: r2(0.05 * g), duration: 1.2, theme: p.theme }));
  }
  kids.push(Rule({ x, y: y + h, length: w, color: t.colors.subtle, at: 0, duration: 1.2, theme: p.theme }));
  const draw0 = 0.3;
  kids.push(
    shape({
      shape: "path",
      d,
      start: draw0,
      x,
      y,
      width: w,
      height: h,
      anchor: [0, 0],
      stroke: { width: th, color: p.color ?? t.colors.text },
      lineCap: "round",
      progress: { from: 0, to: 1, end: dur, ease },
    }),
  );
  if (p.marker !== false && pts.length) {
    const ms = p.markerSize ?? 14;
    kids.push(
      rect({ start: draw0, x: dx, y: dy, width: ms, height: ms, radius: ms / 2, color: p.color ?? t.colors.text, opacity: [[0, 0], [0.2, 1, "linear"], [r2(dur), 1], [r2(dur + 0.15), 0, "linear"]] }),
      rect({ start: r2(draw0 + dur), x: r2(x + pts[pts.length - 1][0]), y: r2(y + pts[pts.length - 1][1]), width: ms, height: ms, color: t.colors.accent, in: { type: "pop", duration: 0.5 } }),
      rect({
        start: r2(draw0 + dur),
        end: r2(draw0 + dur + 0.9),
        x: r2(x + pts[pts.length - 1][0]),
        y: r2(y + pts[pts.length - 1][1]),
        width: ms,
        height: ms,
        border: { width: 2, color: t.colors.accent },
        color: "rgba(0,0,0,0)",
        scale: { from: 1, to: 4, ease: "expo-out" },
        opacity: { from: 0.9, to: 0, ease: "out" },
      }),
    );
  }
  if (p.label) kids.push(Eyebrow({ text: p.label, x, y: r2(y - 48), at: 0.1, theme: p.theme }));
  const opacity = exitAt !== undefined ? [[0, 1], [r2(exitAt), 1], [r2(exitAt + 0.45), 0, "in"]] : undefined;
  return group({ ...life(p.at, p.exit, 0.45), opacity, children: kids });
}

/**
 * A ring that fills to `value` (0..1) over a hairline track, with the percentage counting in its centre and a label
 * underneath. `x`, `y` are the ring's centre.
 */
export function ProgressRing(p = {}) {
  const t = themeOf(p);
  const size = p.size ?? 220;
  const th = p.thickness ?? Math.max(4, Math.round(size * 0.035));
  const v = Math.max(0, Math.min(1, p.value ?? 1));
  const dur = p.duration ?? 1.4;
  const x = p.x ?? 0;
  const y = p.y ?? 0;
  const exitAt = p.exit !== undefined ? p.exit - (p.at ?? 0) : undefined;
  const kids = [
    shape({ shape: "circle", x, y, width: size, height: size, stroke: { width: th, color: p.trackColor ?? t.colors.line }, scale: [[0, 0.85], [0.6, 1, t.motion.ease]], opacity: [[0, 0], [0.3, 1, "linear"]] }),
    shape({ shape: "circle", start: 0.2, x, y, width: size, height: size, stroke: { width: th, color: p.color ?? t.colors.text }, lineCap: "butt", progress: { from: 0, to: v, end: dur, ease: p.ease ?? t.motion.ease } }),
  ];
  if (p.showValue !== false) {
    kids.push(
      counter({
        start: 0.2,
        value: { from: 0, to: v * 100, end: dur, ease: p.ease ?? t.motion.ease },
        suffix: "%",
        separator: "none",
        ...face(t, "mono", {}),
        size: Math.round(size * 0.2),
        color: t.colors.text,
        align: "center",
        x,
        y,
        opacity: [[0, 0], [0.3, 1, "linear"]],
      }),
    );
  }
  if (p.label) kids.push(Eyebrow({ text: p.label, x, y: r2(y + size / 2 + 26), align: "center", size: p.labelSize ?? 20, at: 0.5, theme: p.theme }));
  const opacity = exitAt !== undefined ? [[0, 1], [r2(exitAt), 1], [r2(exitAt + 0.4), 0, "in"]] : undefined;
  return group({ ...life(p.at, p.exit, 0.4), opacity, children: kids });
}

/**
 * Rows of checks: each row masks in, a box draws and a check (or cross) strokes into it, an optional value sits
 * right-aligned, and a hairline separates the rows. Items: strings or `{ label, value, status: "ok" | "fail" | "pending" }`.
 */
export function Checklist(p = {}) {
  const t = themeOf(p);
  const items = (p.items ?? []).map((it) => (typeof it === "string" ? { label: it } : it));
  const x = p.x ?? 0;
  const y = p.y ?? 0;
  const w = p.width ?? 800;
  const size = p.size ?? 28;
  const rowH = p.rowHeight ?? Math.round(size * 2.1);
  const stagger = p.stagger ?? 0.28;
  const f = face(t, "mono", p);
  const box = Math.round(size * 0.85);
  const exitAt = p.exit !== undefined ? p.exit - (p.at ?? 0) : undefined;
  const kids = [];
  items.forEach((it, i) => {
    const ry = y + i * rowH;
    const s = r2(0.1 + i * stagger);
    const status = it.status ?? "ok";
    const col = status === "fail" ? t.colors.negative : status === "pending" ? t.colors.muted : t.colors.positive;
    const by = ry + (size * 1.37 - box) / 2;
    kids.push(
      Outline({ x, y: r2(by), width: box, height: box, color: status === "pending" ? t.colors.subtle : t.colors.muted, thickness: 1.5, at: s, duration: 0.5, exit: exitAt, theme: p.theme }),
      MaskLine({ text: it.label, x: x + box + Math.round(size * 0.8), y: ry, size, ...f, color: status === "pending" ? t.colors.muted : p.color, at: r2(s + 0.05), exit: exitAt, inDuration: 0.7, theme: p.theme }),
    );
    if (status !== "pending") {
      const pts = status === "fail" ? null : [[box * 0.2, box * 0.52], [box * 0.42, box * 0.74], [box * 0.82, box * 0.28]];
      kids.push(
        shape({
          start: r2(s + 0.35),
          ...(pts ? { shape: "line", points: pts.map(([a, b]) => [r2(a), r2(b)]) } : { shape: "path", d: `M ${r2(box * 0.25)} ${r2(box * 0.25)} L ${r2(box * 0.75)} ${r2(box * 0.75)} M ${r2(box * 0.75)} ${r2(box * 0.25)} L ${r2(box * 0.25)} ${r2(box * 0.75)}` }),
          x,
          y: r2(by),
          width: box,
          height: box,
          anchor: [0, 0],
          stroke: { width: Math.max(2.5, size * 0.12), color: col },
          lineCap: "square",
          progress: { from: 0, to: 1, end: 0.4, ease: "out" },
          opacity: exitAt !== undefined ? [[0, 1], [r2(exitAt - s - 0.35), 1], [r2(exitAt - s - 0.05), 0, "linear"]] : undefined,
        }),
      );
    }
    if (it.value !== undefined) {
      kids.push(MaskLine({ text: String(it.value), x: x + w, y: ry, size, ...f, align: "right", color: it.valueColor ?? t.colors.muted, at: r2(s + 0.15), exit: exitAt, inDuration: 0.7, theme: p.theme }));
    }
    if (p.rules !== false && i < items.length - 1) {
      kids.push(Rule({ x, y: r2(ry + rowH - (rowH - size * 1.37) / 2), length: w, color: t.colors.line, thickness: 1, at: r2(s + 0.1), duration: 0.9, exit: exitAt, theme: p.theme }));
    }
  });
  return group({ ...life(p.at, p.exit, 0.6), children: kids });
}

/**
 * Horizontal meters, one per row: a label, a hairline track, a fill that sweeps to the value (0..1) and the value
 * counting at the right. Items: `{ label, value }`.
 */
export function ProgressBars(p = {}) {
  const t = themeOf(p);
  const items = p.items ?? [];
  const x = p.x ?? 0;
  const y = p.y ?? 0;
  const w = p.width ?? 800;
  const size = p.size ?? 24;
  const rowH = p.rowHeight ?? Math.round(size * 1.9);
  const labelW = p.labelWidth ?? Math.round(w * 0.32);
  const valueW = p.valueWidth ?? Math.round(size * 3.6);
  const stagger = p.stagger ?? 0.22;
  const dur = p.duration ?? 1.0;
  const th = p.thickness ?? 4;
  const f = face(t, "mono", p);
  const exitAt = p.exit !== undefined ? p.exit - (p.at ?? 0) : undefined;
  const kids = [];
  items.forEach((it, i) => {
    const ry = y + i * rowH;
    const s = r2(0.1 + i * stagger);
    const bx = x + labelW;
    const bw = w - labelW - valueW;
    const cy = r2(ry + size * 0.685);
    const v = Math.max(0, Math.min(1, it.value ?? 0));
    const accent = it.highlight ?? false;
    kids.push(
      MaskLine({ text: it.label, x, y: ry, size, ...f, color: p.color, at: s, exit: exitAt, inDuration: 0.7, theme: p.theme }),
      Rule({ x: bx, y: cy, length: bw, color: t.colors.line, thickness: th, at: s, duration: 0.6, exit: exitAt, theme: p.theme }),
      rect({
        start: r2(s + 0.2),
        x: bx,
        y: cy,
        width: r2(Math.max(1, bw * v)),
        height: th,
        anchor: [0, 0.5],
        color: accent ? t.colors.accent : p.fillColor ?? t.colors.text,
        scaleX: inOut(0, 1, dur, t.motion.move, exitAt !== undefined ? exitAt - s - 0.2 : undefined, 0.45, 0, t.motion.exitEase),
      }),
      counter({
        start: r2(s + 0.2),
        value: { from: 0, to: Math.round(v * 100), end: dur, ease: t.motion.move },
        suffix: "%",
        separator: "none",
        ...f,
        size,
        color: t.colors.muted,
        align: "right",
        anchor: [1, 0],
        x: x + w,
        y: r2(ry + size * 0.11),
        opacity: exitAt !== undefined ? [[0, 0], [0.2, 1, "linear"], [r2(exitAt - s - 0.2), 1], [r2(exitAt - s + 0.1), 0, "linear"]] : [[0, 0], [0.2, 1, "linear"]],
      }),
    );
  });
  return group({ ...life(p.at, p.exit, 0.6), children: kids });
}

// ------------------------------------------------------------------ motion and timing

/**
 * A group that whips out sideways at `exit` (fast expo-in travel: motion blur turns it into a streak) and, with
 * `enter`, whips in from the opposite side. Full-size (the canvas, or `width` x `height`).
 */
export function Whip(p = {}) {
  const t = themeOf(p);
  const W = p.width ?? t.width;
  const H = p.height ?? t.height;
  const dir = p.direction ?? "left";
  const horizontal = dir === "left" || dir === "right";
  const sign = dir === "left" || dir === "up" ? -1 : 1;
  const dist = p.distance ?? (horizontal ? W : H) * 1.35;
  const c = horizontal ? W / 2 : H / 2;
  const outD = p.duration ?? 0.85;
  const inD = p.enterDuration ?? 0.8;
  const k = [];
  if (p.enter) k.push([0, r2(c - sign * dist)], [inD, c, t.motion.ease]);
  else k.push([0, c]);
  if (p.exit !== undefined) {
    const e = p.exit - (p.at ?? 0);
    k.push([r2(e), c], [r2(e + outD), r2(c + sign * dist), "expo-in"]);
  }
  return group({
    ...life(p.at, p.exit, outD),
    width: W,
    height: H,
    x: horizontal ? (k.length > 1 ? k : c) : W / 2,
    y: horizontal ? H / 2 : k.length > 1 ? k : c,
    children: p.children,
  });
}

/**
 * Offsets the start of each child by `each` seconds (after `from`), so a list of elements enters one after another.
 * Children keep their own `start` (it is shifted). Returns the children (a fragment).
 */
export function Stagger(p = {}) {
  const kids = flatten([p.children ?? []]);
  const each = p.each ?? 0.1;
  const from = p.from ?? 0;
  const n = kids.length;
  return kids.map((c, i) => {
    const off = r2(from + (p.reverse ? n - 1 - i : i) * each);
    if (!off) return c;
    const out = { ...c, start: r2((typeof c.start === "number" ? c.start : 0) + off) };
    if (typeof c.end === "number") out.end = r2(c.end + off);
    return out;
  });
}

/**
 * A chapter of the film on the absolute timeline: a full-size group from `at` for `duration` seconds (or until `end`)
 * that leaves with `exit`: "rise" (drifts up while fading, default), "sink", "fade", "whip" or "none".
 */
export function Act(p = {}) {
  const t = themeOf(p);
  const at = p.at ?? 0;
  const len = p.duration ?? (p.end !== undefined ? p.end - at : undefined);
  const style = p.exit ?? "rise";
  const ed = p.exitDuration ?? (style === "whip" ? 0.85 : 0.6);
  const H = p.height ?? t.height;
  const W = p.width ?? t.width;
  const g = { start: at || undefined, end: len !== undefined ? r2(at + len) : undefined, children: p.children, motionBlur: p.motionBlur };
  if (len === undefined || style === "none") return group(g);
  const e = r2(len - ed);
  if (style === "whip") return group({ ...g, width: W, height: H, x: [[e, W / 2], [r2(len), r2(W / 2 - W * 1.35), "expo-in"]], y: H / 2 });
  const op = [[e, 1], [r2(len), 0, "in"]];
  if (style === "fade") return group({ ...g, opacity: op });
  const d = (p.distance ?? 90) * (style === "sink" ? 1 : -1);
  return group({ ...g, width: W, height: H, x: W / 2, y: [[e, H / 2], [r2(len), r2(H / 2 + d), t.motion.exitEase]], opacity: op });
}

/** Alias of `Act`. */
export const Chapter = Act;

/**
 * Lay acts end to end: `timeline({ intro: 5, problem: 4.5, outro: 6 })` ->
 * `{ intro: { at: 0, end: 5, duration: 5 }, problem: { at: 5, ... }, ..., total: 15.5 }`.
 * A negative `overlap` (seconds) makes each act start that much before the previous one ends.
 */
export function timeline(acts, options = {}) {
  const out = {};
  let at = options.start ?? 0;
  const ov = options.overlap ?? 0;
  for (const [name, duration] of Object.entries(acts)) {
    out[name] = { at: r2(at), end: r2(at + duration), duration };
    at += duration - ov;
  }
  const ends = Object.values(out).map((a) => a.end);
  out.total = ends.length ? Math.max(...ends) : 0;
  return out;
}

// eases used to place things along a drawn path (mirrors the scene format's)
const EASES = {
  linear: (x) => x,
  in: (x) => x * x * x,
  out: (x) => 1 - (1 - x) ** 3,
  "in-out": (x) => (x < 0.5 ? 4 * x * x * x : 1 - (-2 * x + 2) ** 3 / 2),
  smooth: (x) => -(Math.cos(Math.PI * x) - 1) / 2,
  "expo-out": (x) => (x >= 1 ? 1 : 1 - 2 ** (-10 * x)),
  "expo-in-out": (x) => (x <= 0 || x >= 1 ? x : x < 0.5 ? 2 ** (20 * x - 10) / 2 : (2 - 2 ** (-20 * x + 10)) / 2),
  "quint-out": (x) => 1 - (1 - x) ** 5,
  "quint-in-out": (x) => (x < 0.5 ? 16 * x ** 5 : 1 - (-2 * x + 2) ** 5 / 2),
};
