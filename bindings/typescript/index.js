// The flits SDK: components and motion helpers for writing videos in TypeScript (components.js), and a
// thin wrapper around the flits CLI to render, check and transcribe from code.
// Dependency-free (Node >= 18, Bun, Deno). Types: index.d.ts, components.d.ts and scene.d.ts.

export * from "./components.js";

import { spawn } from "node:child_process";
import { compile } from "./compile.js";
import { randomBytes } from "node:crypto";
import { readFile, rm, writeFile } from "node:fs/promises";
import { join, resolve } from "node:path";

export class FlitsError extends Error {
  constructor(message, { code, stderr, stdout, args } = {}) {
    super(message);
    this.name = "FlitsError";
    this.code = code;
    this.stderr = stderr;
    this.stdout = stdout;
    this.args = args;
  }
}

/** The flits executable: `options.bin`, $FLITS_BIN or `flits` on PATH. */
function binary(options) {
  return options?.bin ?? process.env.FLITS_BIN ?? "flits";
}

/** `{ keepTemp: true, workers: 4 }` -> `["--keep-temp", "--workers", "4"]` */
export function toArgs(options = {}) {
  const args = [];
  for (const [key, value] of Object.entries(options)) {
    if (value === undefined || value === null || value === false) continue;
    const flag = "--" + key.replace(/[A-Z]/g, (c) => "-" + c.toLowerCase()).replace(/_/g, "-");
    if (value === true) args.push(flag);
    else if (Array.isArray(value)) for (const v of value) args.push(flag, String(v));
    else args.push(flag, String(value));
  }
  return args;
}

/** Run `flits <args>`; resolves with stdout, rejects with a FlitsError carrying stderr. */
export function run(args, options = {}) {
  const bin = binary(options);
  return new Promise((resolvePromise, reject) => {
    const child = spawn(bin, args, { stdio: ["ignore", "pipe", "pipe"], cwd: options.cwd, signal: options.signal });
    let stdout = "";
    let stderr = "";
    child.stdout.setEncoding("utf8").on("data", (d) => (stdout += d));
    child.stderr.setEncoding("utf8").on("data", (d) => {
      stderr += d;
      options.onStderr?.(d);
    });
    child.on("error", (e) =>
      reject(new FlitsError(e.code === "ENOENT" ? `flits not found ('${bin}'): install it or set FLITS_BIN` : e.message, { args })),
    );
    child.on("close", (code) => {
      if (code === 0) resolvePromise(stdout);
      else {
        const msg = stderr.trim().split(/\r?\n/).filter((l) => l.startsWith("error:")).pop() ?? `flits exited with code ${code}`;
        reject(new FlitsError(msg, { code, stderr, stdout, args }));
      }
    });
  });
}

/**
 * Call `fn(path)` with the scene as a file. A scene object is written to a temporary file in
 * `baseDir` (default: the current directory) so relative media paths resolve against it, and removed afterwards.
 */
async function withSceneFile(scene, baseDir, fn) {
  if (typeof scene === "string") return fn(scene);
  const path = join(resolve(baseDir ?? "."), `.flits-${process.pid}-${randomBytes(4).toString("hex")}.scene.json`);
  await writeFile(path, JSON.stringify(compile(scene), null, 2));
  try {
    return await fn(path);
  } finally {
    await rm(path, { force: true });
  }
}

const lines = (s) => s.split(/\r?\n/).map((l) => l.trim()).filter(Boolean);
const RUN_KEYS = ["bin", "cwd", "signal", "onStderr", "baseDir"];
function split(options = {}) {
  const run = {};
  const flags = {};
  for (const [k, v] of Object.entries(options)) (RUN_KEYS.includes(k) ? run : flags)[k] = v;
  return [run, flags];
}

/** Render a scene (object or path to a .json file) to a video. Resolves with the output path. */
export async function render(scene, out, options = {}) {
  const [ro, flags] = split(options);
  await withSceneFile(scene, ro.baseDir, (path) => run(["render", path, "-o", resolve(out), ...toArgs(flags)], ro));
  return resolve(out);
}

/** Render single frames to PNG. `at`: seconds (or "mm:ss"), one or several. Resolves with the written files. */
export async function still(scene, at, out, options = {}) {
  const [ro, flags] = split(options);
  const times = Array.isArray(at) ? at : [at];
  const stdout = await withSceneFile(scene, ro.baseDir, (path) => run(["still", path, ...toArgs({ at: times }), "-o", resolve(out), ...toArgs(flags)], ro));
  return lines(stdout);
}

/** Contact sheet: evenly spaced frames in one PNG. */
export async function sheet(scene, out, options = {}) {
  const [ro, flags] = split(options);
  await withSceneFile(scene, ro.baseDir, (path) => run(["sheet", path, "-o", resolve(out), ...toArgs(flags)], ro));
  return resolve(out);
}

/**
 * Validate a scene the way flits sees it (typos, unknown values, missing files). Never throws for an invalid scene:
 * resolves with `{ ok, warnings, error, output }`.
 */
export async function check(scene, options = {}) {
  const [ro] = split(options);
  const warnings = (output) => lines(output).filter((l) => l.startsWith("warning: ")).map((l) => l.slice(9));
  try {
    const output = await withSceneFile(scene, ro.baseDir, (path) => run(["check", path], ro));
    return { ok: true, warnings: warnings(output), error: null, output };
  } catch (e) {
    if (!(e instanceof FlitsError) || e.code === undefined) throw e;
    return { ok: false, warnings: warnings(e.stdout), error: e.message.replace(/^error: /, ""), output: e.stdout + e.stderr };
  }
}

/** Transcribe audio/video to word-timed JSON (+ .srt/.vtt/.txt next to it). Resolves with the JSON paths. */
export async function transcribe(inputs, options = {}) {
  const [ro, flags] = split(options);
  const files = (Array.isArray(inputs) ? inputs : [inputs]).map((p) => resolve(p));
  return lines(await run(["transcribe", ...files, ...toArgs(flags)], ro));
}

/** The JSON Schema of the scene format (from the installed flits). */
export async function schema(options = {}) {
  const [ro] = split(options);
  return JSON.parse(await run(["schema"], ro));
}

/** Read a transcript written by `transcribe`. */
export async function readTranscript(path) {
  return JSON.parse(await readFile(path, "utf8"));
}
