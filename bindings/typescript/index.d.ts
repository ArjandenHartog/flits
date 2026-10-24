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

/** A time in seconds or "mm:ss" / "h:mm:ss". */
export type ReelTime = number | string;

/** Options of `reel()`: every `flits reel` flag (camelCase), plus the run options. */
export interface ReelOptions extends RunOptions {
  /** start of the part to use */
  from?: ReelTime;
  /** end of the part to use */
  to?: ReelTime;
  /** several parts, played in this order: "1:12-1:40,2:05-2:31" or [["1:12", "1:40"], [125, 151]] */
  clips?: string | (string | [ReelTime, ReelTime])[];
  /** pick the best part from the transcript: `true` (about 45 s) or a target length in seconds (15..90) */
  auto?: boolean | number;
  /** with `auto`: render the N best parts, each another story (<out>-1.mp4, ...; 1..5) */
  candidates?: 1 | 2 | 3 | 4 | 5;
  /** only rank the candidates (implies `auto`); `reel()` then resolves with `ReelCandidate[]` */
  pick?: boolean;
  /** "9:16" (default) | "1:1" | "4:5" | "16:9" */
  format?: "9:16" | "1:1" | "4:5" | "16:9" | (string & {});
  /** "crop" (fill the frame, default) | "fit" (whole picture, captions underneath) */
  fit?: "crop" | "fit";
  /** "auto" (follow the people, shot by shot; default) or a fixed point 0 (left) .. 1 (right) */
  focus?: "auto" | number;
  /** hook text at the top for the first seconds */
  title?: string;
  /** caption look */
  preset?: "tiktok" | "karaoke-box" | "bounce" | "minimal" | "neon" | "highlighter" | "subtitle" | "word-pop-big";
  /** active word colour */
  highlight?: string;
  /** captions position as a fraction of the height */
  position?: number;
  /** also remove filler words (uh, ehm, ...) */
  fillers?: boolean;
  /** also remove repeated words and stutters */
  repeats?: boolean;
  /** keep silences as they are */
  noSilence?: boolean;
  /** pause left in place of a removed silence (seconds, default 0.2) */
  keep?: number;
  /** frame rate (default: the footage's, halved above 30 fps) */
  fps?: number;
  /** don't punch in on jump cuts */
  noPunch?: boolean;
  /** music under the voice: ducked while someone speaks, looped, faded out (over the end card when there is one) */
  music?: string;
  /** logo (png/svg) in the top right corner (and on the end card) */
  logo?: string;
  /** a thin progress bar along the bottom */
  progress?: boolean;
  /** brand colour: progress bar, active caption word, end card rule */
  accent?: string;
  /** end card after the last clip with this call to action ("Volg ons"), on a blurred still of the last frame */
  end?: string;
  /** end card with a logo: `true` = the `logo`, or a path */
  endLogo?: boolean | string;
  /** end card length in seconds (default 2) */
  endDuration?: number;
  /** existing transcript (.json/.srt/.vtt) instead of transcribing */
  transcript?: string;
  // transcription (when there is no transcript yet)
  model?: string;
  lang?: string;
  device?: "auto" | "cuda" | "cpu";
  python?: string;
  /** GPU server URL for transcription, or "off" */
  remote?: string;
  initialPrompt?: string;
  /** file with terms to favour */
  vocab?: string;
  diarize?: boolean;
  speakers?: number;
  // rendering
  workers?: number;
  encoder?: "auto" | "nvenc" | "qsv" | "vaapi" | "cpu" | "null";
  codec?: "h264" | "hevc" | "av1";
  speed?: "fast" | "balanced" | "quality";
  quality?: number;
  scale?: number;
  gpu?: "auto" | "gpu" | "cpu";
  keepTemp?: boolean;
  quiet?: boolean;
}

/** A part `reel({ pick: true })` proposes, best first. */
export interface ReelCandidate {
  rank: number;
  /** source seconds: pass them as `from`/`to` */
  from: number;
  to: number;
  /** estimated reel length in seconds (long pauses cut) */
  length: number;
  score: number;
  /** the opening sentence and the last one */
  first: string;
  last: string;
  /** why: points and explanation, largest effect first */
  reasons: { points: number; why: string }[];
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
/**
 * Raw footage to a vertical reel (`flits reel`): resolves with the written video paths (one per candidate). `out`
 * defaults to <video>-reel.mp4 (pass `null`).
 */
export function reel(video: string, out: string | null, options: ReelOptions & { pick: true }): Promise<ReelCandidate[]>;
export function reel(video: string, out?: string | null, options?: ReelOptions): Promise<string[]>;
/** The `flits reel` command line `reel()` runs (without the executable). */
export function reelArgs(video: string, out?: string | null, options?: ReelOptions): string[];
/** The JSON Schema of the scene format, from the installed flits. */
export function schema(options?: RunOptions): Promise<Record<string, unknown>>;
export function readTranscript(path: string): Promise<Transcript>;
/** Run any flits command; resolves with stdout. */
export function run(args: string[], options?: RunOptions): Promise<string>;
/** `{ keepTemp: true, workers: 4 }` -> `["--keep-temp", "--workers", "4"]` */
export function toArgs(options?: Record<string, unknown>): string[];
