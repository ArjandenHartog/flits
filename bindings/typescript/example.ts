// Minimal example: a typed scene rendered through the flits CLI.
// Run with Node >= 22.6 (`node --experimental-strip-types example.ts`), tsx or Deno.
import { check, render, type Scene } from "./index.js";

const scene: Scene = {
  width: 1080,
  height: 1920,
  fps: 30,
  duration: 4,
  background: "#101014",
  layers: [
    { type: "rect", gradient: { type: "radial", colors: ["#2b1d3b", "#0d0b10"] } },
    { type: "text", text: "Hello from TypeScript", size: 96, maxWidth: 900, wrap: "balance", in: "pop", out: "fade 0.4" },
    { type: "counter", y: 1300, value: { from: 0, to: 2026, ease: "out", end: 2 }, separator: "none", size: 120, color: "#f26328" },
  ],
};

const result = await check(scene);
if (!result.ok) throw new Error(result.error ?? "invalid scene");
for (const w of result.warnings) console.warn("warning:", w);

const out = await render(scene, "hello-ts.mp4", { scale: 0.5, onStderr: (s) => process.stderr.write(s) });
console.log("rendered", out);
