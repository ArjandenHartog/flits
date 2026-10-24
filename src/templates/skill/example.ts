// Example for the flits-video skill: a reviewed launch video for flits 0.2 - 15 s, 1920x1080, 60 fps.
//
// Storyboard (one idea per beat):
//   0.0 - 3.6   hook      INTRODUCING FLITS 0.2 / "Video at the speed of code."
//   3.6 - 7.8   proof     the benchmark: two bars, Remotion 199.6 s vs flits 21.0 s -> "9.5x faster"
//   7.8 - 11.8  features  three numbered lines, staggered
//  11.8 - 15.0  CTA       wordmark + install command, held to the end
import { At, Composition, Counter, Group, Rect, Text, defineVideo, formats } from "#flits";

// ---------------------------------------------------------------- design tokens
const W = 1920;
const M = 160; // margin
const BG = "#0b0b10";
const INK = "#f4f2ee";
const MUTED = "#8b8a99";
const ACCENT = "#ff5a1f";
const DISPLAY = "Segoe UI Black";
const MONO = "Cascadia Mono";
const ARRIVE = "expo-out" as const;
const LEAVE = { type: "slide-up", duration: 0.4, ease: "expo-in", distance: 60 } as const;

/** Small uppercase mono label. */
const Label = (text: string, y: number, start = 0) =>
  Text({
    text, y, x: M, anchor: [0, 0.5], font: MONO, size: 26, letterSpacing: 4, color: MUTED, uppercase: true,
    start, in: { type: "slide-fade", duration: 0.6, ease: ARRIVE },
  });

/** Big tight headline, revealed line by line out of a mask. */
const Headline = (text: string, y: number, start = 0, size = 150) =>
  Text({
    text, y, x: M, anchor: [0, 0], align: "left", font: DISPLAY, size, lineHeight: 1.02, letterSpacing: -4,
    color: INK, start, reveal: "lines", revealStyle: "mask", revealDuration: 0.25,
  });

// ---------------------------------------------------------------- beats
function Hook() {
  return Group({
    out: LEAVE,
    children: [
      Label("Introducing flits 0.2", 360, 0.15),
      Headline("Video at the\nspeed of code.", 410, 0.3),
      Rect({ x: M, y: 770, anchor: [0, 0.5], width: 240, height: 10, color: ACCENT,
             scaleX: { from: 0, to: 1, start: 0.9, end: 1.7, ease: ARRIVE } }),
    ],
  });
}

function Bar(name: string, seconds: number, y: number, start: number, color: string) {
  const full = 1180; // px for the slowest
  const width = Math.max(14, (seconds / 199.6) * full);
  const grow = { start, end: start + 1.1, ease: ARRIVE } as const; // in group time (the rect starts with the group)
  const growOwn = { start: 0, end: 1.1, ease: ARRIVE } as const; // the same, for a layer that itself starts at `start`
  return [
    Text({ text: name, x: M, y, anchor: [0, 0.5], font: MONO, size: 30, color: INK, start: start - 0.1,
           in: { type: "slide-fade", duration: 0.5, ease: ARRIVE } }),
    Rect({ x: M + 220, y, anchor: [0, 0.5], width, height: 56, radius: 6, color, scaleX: { from: 0, to: 1, ...grow } }),
    // the number rides on the tip of the growing bar (same timing and ease as its scaleX)
    Counter({ x: { from: M + 220 + 28, to: M + 220 + width + 28, ...growOwn }, y, anchor: [0, 0.5], align: "left", font: MONO, size: 30, color: INK,
              value: { from: 0, to: seconds, ...growOwn }, decimals: 1, suffix: " s", start: start,
              motionBlur: false }), // blur would average the changing digits into a smear
  ];
}

function Proof() {
  return Group({
    out: LEAVE,
    children: [
      Label("1080p · 1,800 frames · the same scene", 300, 0),
      Headline("9.5× faster.", 340, 0.1),
      ...Bar("Remotion", 199.6, 640, 0.5, "#3a3946"),
      ...Bar("flits", 21.0, 730, 0.62, ACCENT),
    ],
  });
}

function Features() {
  const items = ["GPU compositing, hardware encoding", "Motion blur and masked type", "TypeScript in, MP4 out. No browser."];
  return Group({
    out: LEAVE,
    children: [
      Label("What's inside", 330, 0),
      ...items.flatMap((text, i) => {
        const y = 450 + i * 130;
        const start = 0.15 + i * 0.1;
        return [
          Text({ text: `0${i + 1}`, x: M, y: y + 6, anchor: [0, 0.5], font: MONO, size: 30, color: ACCENT, start,
                 in: { type: "slide-fade", duration: 0.6, ease: ARRIVE } }),
          Text({ text, x: M + 110, y, anchor: [0, 0.5], font: DISPLAY, size: 72, letterSpacing: -2, color: INK,
                 start, reveal: "lines", revealStyle: "mask" }),
        ];
      }),
    ],
  });
}

function Cta() {
  return Group({
    children: [
      Text({ text: "flits", y: 470, font: DISPLAY, size: 260, letterSpacing: -10, color: INK,
             reveal: "chars", revealStyle: "mask", revealDuration: 0.3 }),
      Text({ text: "cargo install flits", y: 690, font: MONO, size: 34, color: INK, start: 0.5,
             background: { color: "#1c1b24", padding: [28, 16], radius: 12 },
             in: { type: "slide-fade", duration: 0.6, ease: ARRIVE } }),
      Text({ text: "github.com/ArjandenHartog/flits", y: 790, font: MONO, size: 26, color: MUTED, start: 0.7,
             in: { type: "fade", duration: 0.6, ease: ARRIVE } }),
    ],
  });
}

// ---------------------------------------------------------------- composition
export default defineVideo(() =>
  Composition({
    ...formats.youtube,
    fps: 60,
    duration: 15,
    background: BG,
    motionBlur: true,
    children: [
      Rect({ gradient: { type: "radial", colors: ["#17161f", BG] } }),
      At({ time: 0, duration: 3.6, children: [Hook()] }),
      At({ time: 3.8, duration: 4.0, children: [Proof()] }),
      At({ time: 8.0, duration: 3.8, children: [Features()] }),
      At({ time: 12.0, duration: 3.0, children: [Cta()] }),
      // a thin progress rule along the bottom edge
      Rect({ x: 0, y: 1080 - 4, anchor: [0, 0.5], width: W, height: 4, color: ACCENT, opacity: 0.6,
             scaleX: { from: 0, to: 1 } }),
    ],
  }),
);
