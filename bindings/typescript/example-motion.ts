// A 30 s product film built only from the motion components (motion.js): no hand-written keyframes.
//
//   flits preview example-motion.ts
//   flits sheet example-motion.ts -o sheet.png
//   flits render example-motion.ts -o motion.mp4
import {
  Act,
  Backdrop,
  BarChart,
  Checklist,
  Composition,
  EndCard,
  Eyebrow,
  HighlightRule,
  Hud,
  KineticTitle,
  LineChart,
  LowerThird,
  Outline,
  ProgressBars,
  ProgressRing,
  Quote,
  StatCounter,
  Stagger,
  Tag,
  Typewriter,
  Whip,
  setTheme,
  themeScene,
  timeline,
} from "./index.js";

// one place restyles the whole film
const theme = setTheme({ colors: { accent: "#FF4A2B" } });
const { margin: M, width: W } = theme;

const acts = timeline({ claim: 5.4, method: 5, panel: 7.5, numbers: 5.5, voice: 5, end: 5 });

export default Composition({
  width: 1920,
  height: 1080,
  fps: 30,
  duration: acts.total,
  motionBlur: { samples: 16 },
  ...themeScene(),
  children: [
    Backdrop(),
    Hud({
      duration: acts.total,
      end: acts.end.at,
      title: "flits  ·  motion library",
      meta: "Render  ·  1920 x 1080",
      chapters: [
        [0.6, "01  Claim"],
        [acts.method.at, "02  Method"],
        [acts.panel.at, "03  Instruments"],
        [acts.numbers.at, "04  Numbers"],
        [acts.voice.at, "05  Voice"],
      ],
    }),

    // 01: the claim
    Act({
      ...acts.claim,
      exit: "none",
      children: KineticTitle({
        text: "Motion design,\nwritten as code.",
        eyebrow: "A component library for flits",
        marker: true,
        x: M + 40,
        y: 300,
        size: 132,
        rule: 620,
        at: 0.6,
        exit: 4.6,
      }),
    }),

    // 02: the method; the whole act whips out to the left
    Act({
      ...acts.method,
      exit: "none",
      children: Whip({
        exit: 4.1,
        children: [
          Eyebrow({ text: "How it works", x: M + 40, y: 290, at: 0.1 }),
          HighlightRule({
            text: "Write it. Render it. Ship it.",
            x: M + 40,
            y: 360,
            size: 104,
            at: 0.2,
            marks: [
              ["Write it.", 0.9],
              ["Render it.", 1.8],
              ["Ship it.", 2.7],
            ],
          }),
          Typewriter({ text: "// ten times faster than a browser.", x: M + 44, y: 600, at: 1.6, size: 36, color: theme.colors.muted }),
        ],
      }),
    }),

    // 03: the instrument panel: four modules
    Act({ ...acts.panel, children: Panel() }),

    // 04: numbers
    Act({
      ...acts.numbers,
      children: [
        Eyebrow({ text: "Benchmark  ·  same 60 s scene", x: M, y: 200, at: 0.1, marker: true }),
        ...Stagger({
          each: 0.15,
          from: 0.2,
          children: [
            StatCounter({ value: 8.2, decimals: 1, suffix: " s", label: "flits  ·  render time", x: M, y: 260, size: 150, accent: true }),
            StatCounter({ value: 84, suffix: " s", label: "browser renderer", x: M + 560, y: 260, size: 150 }),
          ],
        }),
        BarChart({
          x: 1160,
          y: 300,
          width: 640,
          height: 420,
          at: 0.4,
          suffix: "x",
          data: [
            { label: "Remotion", value: 1 },
            { label: "editly", value: 2.4 },
            { label: "ffmpeg", value: 4.1 },
            { label: "flits", value: 10.2 },
          ],
          decimals: 1,
        }),
        Tag({ text: "NVENC", x: M, y: 720, at: 1.4 }),
        Tag({ text: "Vulkan / DX12", x: M + 180, y: 720, at: 1.55 }),
        Tag({ text: "Motion blur", x: M + 470, y: 720, at: 1.7, accent: true }),
      ],
    }),

    // 05: a voice
    Act({
      ...acts.voice,
      children: [
        Quote({
          text: "We replaced a render farm with one laptop and a JSON file.",
          author: "Jamie Doe",
          role: "Head of motion  ·  Studio North",
          x: M + 120,
          y: 330,
          size: 76,
          maxWidth: 1350,
          at: 0.2,
        }),
        LowerThird({ name: "Studio North", role: "Amsterdam  ·  since 2019", x: W - M - 520, y: 820, at: 2.4, size: 40 }),
      ],
    }),

    // 06: the end card
    EndCard({
      at: acts.end.at,
      duration: acts.end.duration,
      eyebrow: "Open source  ·  MIT",
      title: "Make it move.",
      subtitle: "Components for kinetic type, data and transitions.",
      url: "github.com/ArjandenHartog/flits",
      fadeOut: 0.8,
    }),
  ],
});

function Panel() {
  const top = 290;
  const gap = 24;
  const gw = (W - 2 * M - gap) / 2;
  const gh = 300;
  const cols = [M, M + gw + gap];
  const rows = [top, top + gh + gap];
  const pad = 36;
  return [
    Eyebrow({ text: "Live instruments", x: M, y: 200, at: 0.05, marker: true }),
    ...[0, 1, 2, 3].map((i) => Outline({ x: cols[i % 2], y: rows[Math.floor(i / 2)], width: gw, height: gh, at: 0.2 + i * 0.1, duration: 1.4 })),

    // response time, drawn as a line
    Eyebrow({ text: "Frames per second", x: cols[0] + pad, y: rows[0] + pad - 6, at: 0.6 }),
    LineChart({
      x: cols[0] + pad,
      y: rows[0] + 96,
      width: gw - 2 * pad - 260,
      height: 150,
      at: 0.8,
      data: [42, 48, 45, 60, 58, 72, 70, 88, 84, 96, 104, 118, 112, 131],
      grid: 2,
    }),
    StatCounter({ value: 131, label: "fps peak", x: cols[0] + gw - pad, y: rows[0] + 90, size: 64, align: "right", at: 1.0, rule: false }),

    // checks
    Eyebrow({ text: "Scene check", x: cols[1] + pad, y: rows[0] + pad - 6, at: 0.7 }),
    Checklist({
      x: cols[1] + pad,
      y: rows[0] + 84,
      width: gw - 2 * pad,
      size: 26,
      rowHeight: 50,
      at: 1.0,
      items: [
        { label: "Fonts resolved", value: "3 / 3" },
        { label: "Media found", value: "12 files" },
        { label: "Keyframes valid", value: "no typos" },
        { label: "Audio loudness", value: "-14 LUFS", status: "pending" },
      ],
    }),

    // render progress
    Eyebrow({ text: "Encoders", x: cols[0] + pad, y: rows[1] + pad - 6, at: 0.8 }),
    ProgressBars({
      x: cols[0] + pad,
      y: rows[1] + 90,
      width: gw - 2 * pad,
      size: 24,
      at: 1.2,
      items: [
        { label: "Chunk 1", value: 1 },
        { label: "Chunk 2", value: 0.86 },
        { label: "Chunk 3", value: 0.64, highlight: true },
        { label: "Chunk 4", value: 0.31 },
      ],
    }),

    // rings
    Eyebrow({ text: "GPU  ·  CPU  ·  Disk", x: cols[1] + pad, y: rows[1] + pad - 6, at: 0.9 }),
    ...[
      ["GPU", 0.92],
      ["CPU", 0.48],
      ["Disk", 0.17],
    ].map(([label, v], i) =>
      ProgressRing({ x: cols[1] + pad + 110 + i * 250, y: rows[1] + 150, size: 124, value: v as number, label: label as string, at: 1.3 + i * 0.2 }),
    ),
  ];
}
