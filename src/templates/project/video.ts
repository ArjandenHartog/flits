// A flits video written in TypeScript.
//
//   flits preview src/video.ts                       live preview, reloads when you save
//   flits render src/video.ts -o out/video.mp4       render (add --props '{"title": "Hi"}')
//
// Components are plain functions returning layers, so use variables, loops, map() and your own
// components (see components.ts). Animatable props take numbers, keyframes, {from, to}, or a function
// of time `(t) => number` that flits samples every frame: springs, interpolate(), maths, data.
import { Composition, Group, Rect, Sequence, Shape, Text, defineVideo, formats, interpolate, spring } from "#flits";
import { StatCard, type Stat } from "./components.ts";
import stats from "./data.json" with { type: "json" };

export interface Props {
  title?: string;
  accent?: string;
}

export default defineVideo(({ title = "Made with flits", accent = "#f26328" }: Props) => {
  const intro = 2.4;
  const card = 2.2;
  const overlap = 0.4;
  const duration = intro + stats.length * (card - overlap);

  return Composition({
    ...formats.youtube,
    fps: 30,
    duration,
    background: "#0f1020",
    children: [
      Rect({ gradient: { type: "radial", colors: ["#2a1b4a", "#0f1020"] } }),

      Sequence({
        overlap,
        children: [
          Group({
            duration: intro,
            out: "fade 0.4",
            children: [
              Text({
                text: title,
                y: 480,
                size: 120,
                shadow: { blur: 24, y: 8 },
                // a spring and a fade, computed per frame
                scale: (t) => spring({ t, damping: 11 }),
                opacity: (t) => interpolate(t, [0, 0.25], [0, 1]),
              }),
              Shape({
                shape: "line",
                y: 600,
                width: 640,
                height: 12,
                stroke: { width: 10, color: accent },
                progress: { from: 0, to: 1, start: 0.4, end: 1.3, ease: "out" },
              }),
            ],
          }),
          ...(stats as Stat[]).map((s, i) => StatCard({ ...s, accent, duration: card, index: i })),
        ],
      }),

      // progress bar over the whole video
      Rect({ x: 0, y: 1074, anchor: [0, 0.5], width: 1920, height: 12, color: accent, scaleX: { from: 0, to: 1 } }),
    ],
  });
});
