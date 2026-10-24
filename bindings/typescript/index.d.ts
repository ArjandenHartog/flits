import type { Scene } from "./scene.js";

export * from "./scene.js";
export * from "./components.js";

/** A scene object (functions allowed, see compile()), or the path of a scene file (.json, .ts, .js). */
export type SceneInput = Scene | string;

/** How the CLI is run (not passed to flits as flags). */
export interface RunOptions {
  /** flits executable (default: $FLITS_BIN or `flits` on PATH) */
  bin?: string;
  /** working directory of the flits process */
  cwd?: string;
  /** abort the process */
  signal?: AbortSignal;
  /** receives progress/log output (stderr) as it arrives */
  onStderr?: (chunk: string) => void;
  /**
   * Folder a scene object is written to before running (default: the current directory). Relative media paths in
   * the scene resolve against it, exactly as for a scene file in that folder.
   */
  baseDir?: string;
}

export interface RenderOptions extends RunOptions {
  /** parallel render workers (default: automatic) */
  workers?: number;
  /** "auto" | "nvenc" | "qsv" | "vaapi" | "cpu" | "null" */
  encoder?: "auto" | "nvenc" | "qsv" | "vaapi" | "cpu" | "null";
  codec?: "h264" | "hevc" | "av1";
  /** encoder preset */
  speed?: "fast" | "balanced" | "quality";
  /** CRF/CQ, lower = better (18..28, default 21) */
  quality?: number;
  /** render scale, e.g. 0.5 for a fast half-resolution preview */
  scale?: number;
  /** compositor: "auto" | "gpu" | "cpu" */
  gpu?: "auto" | "gpu" | "cpu";
  keepTemp?: boolean;
  /** no progress output */
  quiet?: boolean;
  /** render on the GPU server configured with `flits remote` */
  remote?: boolean;
}

export interface StillOptions extends RunOptions {
  scale?: number;
}

export interface SheetOptions extends RunOptions {
  /** number of frames (default 12) */
  count?: number;
  /** columns (default 4) */
  cols?: number;
  /** thumbnail width in px (default 480) */
  thumb?: number;
}

export interface TranscribeOptions extends RunOptions {
  /** output JSON (one input only; default: <input>.words.json) */
  out?: string;
  /** folder for the outputs (default: next to each input) */
  outdir?: string;
  /** "default" (large-v3-turbo on GPU, small on CPU), "tiny", "base", "small", "medium", "large-v3", "large-v3-turbo" */
  model?: string;
  /** language code ("en", "nl", ...) or "auto" */
  lang?: string;
  device?: "auto" | "cuda" | "cpu";
  /** python with faster-whisper (default: run through uv) */
  python?: string;
  /** GPU server URL, or "off" to force local */
  remote?: string;
}

export interface CheckResult {
  /** the scene loads and prepares (warnings don't make it fail) */
  ok: boolean;
  /** unknown properties and other non-fatal problems */
  warnings: string[];
  /** why the scene can't be rendered (when !ok) */
  error: string | null;
  /** full CLI output (timeline when ok, stderr otherwise) */
  output: string;
}

/** Word-timed transcript as written by `transcribe` (faster-whisper JSON). */
export interface Transcript {
  language: string;
  duration: number;
  model: string;
  device: string;
  seconds: number;
  segments: { start: number; end: number; text: string; words: { word: string; start: number; end: number; prob: number }[] }[];
}

export class FlitsError extends Error {
  /** exit code (undefined when the process could not start) */
  code?: number;
  stderr?: string;
  stdout?: string;
  args?: string[];
}

/** Render a scene to a video. Resolves with the absolute output path. */
export function render(scene: SceneInput, out: string, options?: RenderOptions): Promise<string>;
/** Render single frames to PNG; `at` in seconds or "mm:ss". Resolves with the written files. */
export function still(scene: SceneInput, at: number | string | (number | string)[], out: string, options?: StillOptions): Promise<string[]>;
/** Contact sheet: evenly spaced frames in one PNG. */
export function sheet(scene: SceneInput, out: string, options?: SheetOptions): Promise<string>;
/** Validate a scene the way flits sees it; never rejects for an invalid scene. */
export function check(scene: SceneInput, options?: RunOptions): Promise<CheckResult>;
/** Transcribe audio/video files; resolves with the .words.json paths (.srt/.vtt/.txt are written next to them). */
export function transcribe(inputs: string | string[], options?: TranscribeOptions): Promise<string[]>;
/** The JSON Schema of the scene format, from the installed flits. */
export function schema(options?: RunOptions): Promise<Record<string, unknown>>;
export function readTranscript(path: string): Promise<Transcript>;
/** Run any flits command; resolves with stdout. */
export function run(args: string[], options?: RunOptions): Promise<string>;
/** `{ keepTemp: true, workers: 4 }` -> `["--keep-temp", "--workers", "4"]` */
export function toArgs(options?: Record<string, unknown>): string[];
