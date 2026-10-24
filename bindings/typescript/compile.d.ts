import type { AnimFn, AudioTrack, Keyframe, Layer, Scene } from "./scene.js";

/** Properties that can be animated (and therefore be functions). */
export const ANIMATABLE: readonly string[];

/** Anything that can appear where layers go: layers, nested arrays, Audio elements, and null/false/undefined. */
export type Children = Layer | AudioTrack | null | undefined | false | true | readonly Children[];

/** Plain scene JSON: function props sampled, children flattened, Audio elements moved to `audio`. */
export function compile(scene: Omit<Scene, "layers"> & { layers?: Children }): Scene;

/** Flatten nested arrays and drop null/undefined/booleans. */
export function flatten<T>(children: unknown): T[];

/** Sample `fn` over `duration` seconds at `fps` into linear keyframes. */
export function sample(fn: AnimFn, duration: number, fps: number, what?: string): Keyframe[];

/** Mark an object as a soundtrack entry. */
export function audioTrack(track: AudioTrack): AudioTrack;
