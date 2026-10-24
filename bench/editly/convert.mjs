#!/usr/bin/env node
// Converts a flits scene.json (as written by examples/benchmark.py) into an editly edit spec.
//
//   node convert.mjs <scene.json> <spec.json5> --out <video.mp4> [--font <file.ttf>]
//
// editly's model is "clips played one after another, each with full-frame layers, joined by
// transitions", so the converter maps the scene's top-level `sequence` group:
//   group child (one shot)            -> editly clip (duration; transition 'fade' = group overlap)
//   full-frame image (zoom from/to)   -> 'image' layer, resizeMode cover, Ken Burns zoomDirection/zoomAmount
//   sized image ("card")              -> 'image-overlay' at the same position/height, Ken Burns zoom
//   sized video ("card")              -> 'video' layer at the same position/size, cut to the clip length
//   text                              -> 'title' layer at the same anchor position
//   scene audio track                 -> audioFilePath
// What editly cannot express is dropped (and listed on stderr): image blur and pan, card
// border/radius/shadow/rotation/scale keyframes, slide-in transitions of cards and text,
// semi-transparent gradient rects, the progress bar, text size/shadow (editly titles size themselves).
// The last clip is shortened so the output has exactly the scene's duration.
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";

const argv = process.argv.slice(2);
const opt = (name, dflt) => {
  const i = argv.indexOf(name);
  if (i < 0) return dflt;
  const v = argv[i + 1];
  argv.splice(i, 2);
  return v;
};
const outVideo = opt("--out", "editly.mp4");
const defaultFont = process.platform === "win32" ? "C:/Windows/Fonts/segoeuib.ttf" : undefined;
const font = opt("--font", defaultFont && fs.existsSync(defaultFont) ? defaultFont : undefined);
const [scenePath, specPath] = argv;
if (!scenePath || !specPath) {
  console.error("usage: node convert.mjs <scene.json> <spec.json5> --out <video.mp4> [--font <file.ttf>]");
  process.exit(2);
}

const scene = JSON.parse(fs.readFileSync(scenePath, "utf8"));
const base = path.dirname(path.resolve(scenePath));
const W = scene.width, H = scene.height, fps = scene.fps ?? 30;
const resolve = (p) => (path.isAbsolute(p) ? p : path.join(base, p)).replace(/\\/g, "/");
const dropped = new Set();

const probeCache = new Map();
function videoSize(file) {
  if (!probeCache.has(file)) {
    const out = execFileSync("ffprobe", ["-v", "error", "-select_streams", "v:0", "-show_entries", "stream=width,height", "-of", "csv=p=0", file], { encoding: "utf8" });
    const [w, h] = out.trim().split(",").map(Number);
    probeCache.set(file, { w, h });
  }
  return probeCache.get(file);
}

/** Ken Burns from a flits `zoom` {from, to}: editly zooms from 1 to 1 + zoomAmount. */
function kenBurns(zoom) {
  if (zoom && typeof zoom === "object" && !Array.isArray(zoom) && zoom.to !== zoom.from) {
    return zoom.to > zoom.from
      ? { zoomDirection: "in", zoomAmount: +(zoom.to / zoom.from - 1).toFixed(4) }
      : { zoomDirection: "out", zoomAmount: +(zoom.from / zoom.to - 1).toFixed(4) };
  }
  return { zoomDirection: null };
}

const constant = (v, d) => (typeof v === "number" ? v : d);
const origin = (a, lo, hi) => (a <= 0.25 ? lo : a >= 0.75 ? hi : "center");

function convertLayer(l, clipDur) {
  const sized = l.width !== undefined || l.height !== undefined;
  if (l.blur) dropped.add("image blur");
  if (l.panX !== undefined || l.panY !== undefined) dropped.add("Ken Burns pan");
  if (l.border || l.shadow || l.radius) dropped.add("border/shadow/radius");
  if (l.rotation !== undefined || l.scale !== undefined) dropped.add("rotation/scale keyframes");
  if (l.in && l.type !== "group") dropped.add(`'${typeof l.in === "string" ? l.in : l.in.type}' layer transitions`);
  const x = constant(l.x, W / 2), y = constant(l.y, H / 2);
  switch (l.type) {
    case "image":
      if (!sized) return { type: "image", path: resolve(l.src), resizeMode: "cover", ...kenBurns(l.zoom) };
      return {
        type: "image-overlay",
        path: resolve(l.src),
        position: { x: x / W, y: y / H, originX: "center", originY: "center" },
        ...(l.height !== undefined ? { height: l.height / H } : { width: l.width / W }),
        ...kenBurns(l.zoom),
      };
    case "video": {
      const src = resolve(l.src);
      const { w: vw, h: vh } = videoSize(src);
      const h = l.height ?? (l.width !== undefined ? (l.width * vh) / vw : H);
      const w = l.width ?? (l.height !== undefined ? (l.height * vw) / vh : W);
      const [ax, ay] = l.anchor ?? [0.5, 0.5];
      if (l.zoom !== undefined) dropped.add("video zoom");
      const trim = l.trim ?? 0;
      return {
        type: "video",
        path: src,
        resizeMode: "cover",
        left: (x - ax * w) / W,
        top: (y - ay * h) / H,
        width: w / W,
        height: h / H,
        // cut exactly the clip length so editly plays the video at its real speed
        cutFrom: trim,
        cutTo: trim + clipDur,
        mixVolume: constant(l.volume, 1),
      };
    }
    case "text": {
      const [ax, ay] = l.anchor ?? [0.5, 0.5];
      dropped.add("text size/shadow (editly titles size themselves)");
      return {
        type: "title",
        text: l.text,
        textColor: l.color ?? "#ffffff",
        position: { x: x / W, y: y / H, originX: origin(ax, "left", "right"), originY: origin(ay, "top", "bottom") },
        zoomDirection: null,
      };
    }
    case "rect":
      dropped.add("rects/gradient overlays");
      return null;
    default:
      dropped.add(`${l.type} layers`);
      return null;
  }
}

const seq = scene.layers.find((l) => l.type === "group" && l.sequence);
if (!seq) throw new Error("expected a top-level group with \"sequence\": true (examples/benchmark.py writes one)");
for (const l of scene.layers) if (l !== seq) dropped.add(`top-level ${l.type} layers (e.g. the progress bar)`);
const overlap = seq.overlap ?? 0;
const total = scene.duration;

const clips = [];
let t = 0;
for (const shot of seq.children) {
  if (total !== undefined && t >= total - 1e-6) break;
  let dur = shot.duration;
  // the last clip ends exactly at the scene's end
  const last = total !== undefined && t + dur >= total - 1e-6;
  if (last) dur = total - t;
  const layers = (shot.type === "group" ? shot.children : [shot]).map((l) => convertLayer(l, dur)).filter(Boolean);
  const fades = shot.in !== undefined || overlap > 0;
  clips.push({ duration: +dur.toFixed(6), layers, transition: last || !fades ? null : { name: "fade", duration: overlap } });
  t += dur - overlap;
  if (last) break;
}
clips.at(-1).transition = null;

const spec = {
  outPath: path.resolve(outVideo).replace(/\\/g, "/"),
  width: W,
  height: H,
  fps,
  defaults: { transition: null, ...(font ? { layer: { fontPath: font } } : {}) },
  clips,
  keepSourceAudio: false,
  ...(scene.audio?.length ? { audioFilePath: resolve(scene.audio[0].src) } : {}),
};
fs.writeFileSync(specPath, JSON.stringify(spec, null, 1));
const len = clips.reduce((s, c, i) => s + c.duration - (i < clips.length - 1 && c.transition ? c.transition.duration : 0), 0);
console.error(`${specPath}: ${clips.length} clips, ${len.toFixed(3)} s (${Math.round(len * fps)} frames)`);
if (dropped.size) console.error(`not expressible in editly (dropped): ${[...dropped].join("; ")}`);
