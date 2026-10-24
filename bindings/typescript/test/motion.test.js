// node --test "bindings/typescript/test/*.test.js"
import assert from "node:assert/strict";
import { test } from "node:test";

// deterministic widths: never run flits from the tests
process.env.FLITS_MEASURE = "estimate";

const m = await import("../index.js");
const {
  Act,
  Backdrop,
  BarChart,
  Checklist,
  Composition,
  EndCard,
  Eyebrow,
  Grid,
  HighlightRule,
  Hud,
  KineticTitle,
  Label,
  LineChart,
  LowerThird,
  MaskLine,
  Outline,
  ProgressBars,
  ProgressRing,
  Quote,
  Rule,
  StatCounter,
  Stagger,
  Tag,
  Text,
  Typewriter,
  Whip,
  createTheme,
  defaultTheme,
  getTheme,
  measureText,
  resetTheme,
  setTheme,
  themeScene,
  timeline,
  withAlpha,
} = m;

const last = (k) => k[k.length - 1];

test("MaskLine: a clipped group whose text rises in and leaves upward", () => {
  const g = MaskLine({ text: "Hello", x: 100, y: 200, size: 100, at: 1, exit: 3 });
  assert.equal(g.type, "group");
  assert.equal(g.clip, true);
  assert.equal(g.start, 1);
  assert.equal(g.end, 3 + defaultTheme.motion.out, "gone once the exit finishes");
  assert.deepEqual(g.anchor, [0, 0]);
  const t = g.children[0];
  assert.equal(t.type, "text");
  assert.equal(t.font, defaultTheme.fonts.display);
  assert.equal(t.weight, defaultTheme.weights.display);
  assert.equal(t.color, defaultTheme.colors.text);
  const box = g.height;
  assert.ok(box > 100 && box < 150, "mask is a line box with room for descenders");
  assert.deepEqual(t.y[0], [0, box]);
  assert.deepEqual(t.y[1], [defaultTheme.motion.in, 0, "expo-out"]);
  assert.deepEqual(t.y[2], [2, 0], "holds until the exit (relative to its start)");
  assert.deepEqual(last(t.y), [2 + defaultTheme.motion.out, -box, "expo-in"]);
  // no exit: stays, no end
  const s = MaskLine({ text: "x" });
  assert.equal(s.end, undefined);
  assert.equal(s.start, undefined);
  assert.equal(s.children[0].y.length, 2);
  // right alignment anchors the right edge
  assert.deepEqual(MaskLine({ text: "x", align: "right" }).anchor, [1, 0]);
});

test("KineticTitle: one masked line per line, staggered in and out", () => {
  const g = KineticTitle({ text: "One\nTwo\nThree", x: 100, y: 100, size: 100, at: 2, exit: 6, stagger: 0.2 });
  const lines = g.children;
  assert.equal(g.start, 2);
  assert.equal(lines.length, 3);
  assert.deepEqual(lines.map((l) => l.start ?? 0), [0, 0.2, 0.4]);
  assert.deepEqual(lines.map((l) => l.y), [100, 210, 320]);
  assert.deepEqual(lines.map((l) => l.children[0].text), ["One", "Two", "Three"]);
  // exits relative to the title: 6 - 2 = 4, plus 0.05 per line, plus the exit itself
  assert.deepEqual(lines.map((l) => l.end), [4.55, 4.6, 4.65]);
  assert.equal(g.end, 6.65);
  const withExtras = KineticTitle({ lines: ["a", "b"], eyebrow: "Intro", rule: 300, accentLine: -1 });
  assert.equal(withExtras.children.length, 4, "eyebrow + 2 lines + rule");
  assert.equal(withExtras.children[2].children[0].color, defaultTheme.colors.accent);
  assert.equal(last(withExtras.children).width, 300);
});

test("Eyebrow: small tracked mono capitals; marker adds an accent square", () => {
  const e = Eyebrow({ text: "Chapter one", x: 50, y: 60 });
  const t = e.children[0];
  assert.equal(t.uppercase, true);
  assert.equal(t.font, defaultTheme.fonts.mono);
  assert.equal(t.color, defaultTheme.colors.muted);
  assert.equal(t.size, 20);
  assert.ok(t.letterSpacing > 0);
  assert.equal(Label, Eyebrow);
  const withMarker = Eyebrow({ text: "x", x: 50, y: 60, marker: true, at: 1, exit: 2 });
  assert.equal(withMarker.start, 1);
  const [line, square] = withMarker.children;
  assert.ok(line.x > 50, "text moves right of the marker");
  assert.equal(square.x, 50);
  assert.equal(square.color, defaultTheme.colors.accent);
});

test("Rule: draws from its start, retracts on exit; vertical and from-right variants", () => {
  const r = Rule({ x: 10, y: 20, length: 300, at: 1, duration: 0.8, exit: 3 });
  assert.equal(r.type, "rect");
  assert.deepEqual(r.anchor, [0, 0.5]);
  assert.equal(r.width, 300);
  assert.equal(r.height, 1.5);
  assert.deepEqual(r.scaleX[0], [0, 0]);
  assert.deepEqual(r.scaleX[1], [0.8, 1, "expo-out"]);
  assert.deepEqual(last(r.scaleX), [2.5, 0, "expo-in"]);
  assert.equal(r.end, 3.5);
  const right = Rule({ x: 10, length: 300, from: "right" });
  assert.equal(right.x, 310);
  assert.deepEqual(right.anchor, [1, 0.5]);
  const v = Rule({ x: 10, y: 20, length: 300, vertical: true, from: "bottom" });
  assert.equal(v.height, 300);
  assert.equal(v.y, 320);
  assert.ok(Array.isArray(v.scaleY) && v.scaleX === undefined);
});

test("HighlightRule: the rule glides between measured phrases", () => {
  const g = HighlightRule({ text: "Measure. Fix. Measure again.", x: 100, y: 500, size: 64, marks: [["Fix.", 1], ["Measure again.", 2]] });
  const rule = g.children.find((c) => c.type === "rect");
  const [line] = g.children;
  assert.equal(line.type, "group", "text first, rule over it");
  const w = (s) => measureText({ text: s, size: 64, font: defaultTheme.fonts.display, weight: defaultTheme.weights.display }).width;
  assert.equal(rule.start, 1);
  // first phrase: x of "Fix." = width("Measure. Fix.") - width("Fix.")
  assert.ok(Math.abs(rule.x[0][1] - (100 + w("Measure. Fix.") - w("Fix."))) < 0.02);
  assert.ok(Math.abs(last(rule.x)[1] - (100 + w("Measure. Fix. Measure again.") - w("Measure again."))) < 0.02);
  assert.equal(last(rule.x)[0], 1 + 0.5, "arrives `glide` seconds after the mark");
  assert.equal(last(rule.scaleX)[1], 1, "the widest phrase is the full rule");
  assert.throws(() => HighlightRule({ text: "abc", marks: [["zzz", 0]] }), /not in/);
  const marker = HighlightRule({ text: "a b", marks: [["b", 0]], variant: "marker" });
  assert.equal(marker.children[0].type, "rect", "a marker sits behind the text");
  assert.match(marker.children[0].color, /^rgba\(/);
});

test("Typewriter: the cursor steps once per character", () => {
  const g = Typewriter({ text: "ab cd", x: 0, y: 0, duration: 2 });
  const [txt, cursor] = g.children;
  assert.equal(txt.reveal, "typewriter");
  assert.equal(txt.revealDuration, 2);
  assert.equal(cursor.x.length, 1 + 4, "start + one step per non-space character");
  assert.deepEqual(cursor.x.map((k) => k[0]), [0, 0, 0.5, 1, 1.5]);
  assert.ok(cursor.x.slice(1).every((k, i, a) => i === 0 || k[1] > a[i - 1][1]), "moves right");
  assert.equal(Typewriter({ text: "x", cursor: false }).children.length, 1);
  const two = Typewriter({ text: "a\nb", size: 40 });
  assert.ok(last(two.children[1].y)[1] > two.children[1].y[1][1], "second line lower");
});

test("theme: set once, override per component, scene settings", () => {
  try {
    setTheme({ colors: { accent: "#00ff00" }, fonts: { display: "Inter" }, fontFiles: { brand: "fonts/b.ttf" } });
    assert.equal(getTheme().colors.accent, "#00ff00");
    assert.equal(getTheme().colors.text, defaultTheme.colors.text, "merged key by key");
    assert.equal(Rule({ color: undefined, thickness: 6 }).color, defaultTheme.colors.line);
    assert.equal(KineticTitle({ text: "x", rule: true }).children[1].color, "#00ff00");
    assert.equal(MaskLine({ text: "x" }).children[0].font, "Inter");
    assert.equal(MaskLine({ text: "x", theme: { colors: { text: "#123456" } } }).children[0].color, "#123456");
    assert.deepEqual(themeScene(), { background: defaultTheme.colors.background, fonts: { brand: "fonts/b.ttf" } });
  } finally {
    resetTheme();
  }
  assert.equal(getTheme(), defaultTheme);
  assert.equal(createTheme({ margin: 64 }).margin, 64);
  assert.equal(defaultTheme.margin, 120, "the default is never mutated");
  assert.equal(withAlpha("#ff0000", 0.5), "rgba(255,0,0,0.5)");
  assert.equal(withAlpha("red", 0.5), "red");
});

test("timing helpers: timeline, Stagger, Act, Whip", () => {
  const tl = timeline({ a: 2, b: 3, c: 1.5 });
  assert.deepEqual(tl.a, { at: 0, end: 2, duration: 2 });
  assert.deepEqual(tl.b, { at: 2, end: 5, duration: 3 });
  assert.equal(tl.c.at, 5);
  assert.equal(tl.total, 6.5);
  const ov = timeline({ a: 2, b: 2 }, { overlap: 0.5 });
  assert.equal(ov.b.at, 1.5);
  assert.equal(ov.total, 3.5);

  const kids = Stagger({ each: 0.25, from: 1, children: [Text({ text: "a" }), Text({ text: "b", start: 0.5, end: 2 }), null, [Text({ text: "c" })]] });
  assert.deepEqual(kids.map((k) => k.start), [1, 1.75, 1.5]);
  assert.equal(kids[1].end, 3.25);
  assert.deepEqual(Stagger({ each: 1, reverse: true, children: [Text({ text: "a" }), Text({ text: "b" })] }).map((k) => k.start), [1, undefined]);

  const act = Act({ ...tl.b, children: Text({ text: "x" }) });
  assert.equal(act.start, 2);
  assert.equal(act.end, 5);
  assert.deepEqual(act.opacity, [[2.4, 1], [3, 0, "in"]]);
  assert.deepEqual(act.y, [[2.4, 540], [3, 450, "expo-in"]], "rises 90 px");
  assert.equal(Act({ at: 1, children: [] }).opacity, undefined, "no duration: no exit");
  assert.equal(Act({ at: 0, duration: 2, exit: "fade" }).y, undefined);

  const w = Whip({ at: 1, exit: 4, children: Text({ text: "x" }) });
  assert.equal(w.end, 4 + 0.85);
  assert.deepEqual(w.x, [[0, 960], [3, 960], [3.85, 960 - 1920 * 1.35, "expo-in"]]);
  assert.equal(w.y, 540);
  const wi = Whip({ enter: true, direction: "up", children: [] });
  assert.equal(wi.x, 960);
  assert.deepEqual(wi.y[0], [0, 540 + 1080 * 1.35]);
});

test("data components: bars, lines, rings, rows", () => {
  const bars = BarChart({ data: [{ label: "a", value: 1 }, { label: "b", value: 4 }], x: 0, y: 0, width: 400, height: 200 });
  const rects = bars.children.filter((c) => c.type === "rect" && c.anchor?.[1] === 1);
  assert.equal(rects.length, 2);
  assert.deepEqual(rects.map((r) => r.height), [50, 200], "heights proportional to the largest value");
  assert.deepEqual(rects.map((r) => r.x), [100, 300]);
  assert.equal(rects[1].color, defaultTheme.colors.accent, "the last bar is highlighted by default");
  assert.equal(rects[0].color, defaultTheme.colors.muted);
  assert.deepEqual(rects.map((r) => r.start), [0.2, 0.28]);
  assert.equal(bars.children.filter((c) => c.type === "counter").length, 2);

  const lc = LineChart({ data: [0, 10, 5, 20], x: 50, y: 60, width: 300, height: 100, grid: 0 });
  const path = lc.children.find((c) => c.type === "shape");
  assert.match(path.d, /^M 0 /);
  assert.ok(path.d.includes("C "), "smooth by default");
  assert.equal(LineChart({ data: [0, 10, 5], smooth: false }).children.find((c) => c.type === "shape").d.includes("C"), false);
  const dot = lc.children.find((c) => Array.isArray(c.x));
  assert.ok(Math.abs(last(dot.x)[1] - 350) < 0.05 && Math.abs(last(dot.y)[1] - (60 + 5)) < 0.05, "the riding dot ends on the last (highest) point");

  const ring = ProgressRing({ value: 0.75, x: 100, y: 100 });
  assert.equal(ring.children[1].progress.to, 0.75);
  assert.equal(ring.children[2].value.to, 75);

  const cl = Checklist({ items: ["ok", { label: "bad", status: "fail", value: 3 }, { label: "later", status: "pending" }] });
  const marks = cl.children.filter((c) => c.type === "shape");
  assert.equal(marks.length, 2, "pending rows get no mark");
  assert.equal(marks[0].stroke.color, defaultTheme.colors.positive);
  assert.equal(marks[1].stroke.color, defaultTheme.colors.negative);

  const pb = ProgressBars({ items: [{ label: "a", value: 0.5 }], width: 1000 });
  const fill = pb.children.find((c) => c.type === "rect" && c.color === defaultTheme.colors.text);
  assert.equal(fill.width, (1000 - 320 - Math.round(24 * 3.6)) * 0.5);

  const sc = StatCounter({ value: 42, suffix: "%", label: "share" });
  assert.equal(sc.children[0].clip, true);
  assert.equal(sc.children[0].children[0].value.to, 42);
});

test("every component compiles inside a Composition and survives JSON", () => {
  const scene = Composition({
    width: 1920,
    height: 1080,
    duration: 12,
    ...themeScene(),
    children: [
      Backdrop(),
      Hud({ duration: 12, title: "t", meta: "m", chapters: [[0, "a"], [5, "b"]] }),
      Grid({ columns: 6, rows: 3 }),
      Act({ at: 0, duration: 6, children: [KineticTitle({ text: "A\nB", eyebrow: "e", rule: true, exit: 5 }), Typewriter({ text: "hello" })] }),
      Whip({ at: 6, exit: 9, enter: true, children: [Quote({ text: "Words, words.", author: "A", role: "B" }), LowerThird({ name: "N", role: "R", exit: 2 })] }),
      Tag({ text: "NEW", accent: true, align: "right", x: 500 }),
      Outline({ width: 300, height: 200, corners: 24, fill: "#111111", exit: 3 }),
      EndCard({ at: 9, duration: 3, title: "Bye", subtitle: "s", url: "u", eyebrow: "e", fadeOut: true }),
      StatCounter({ value: 9, align: "right", x: 900, exit: 4 }),
      BarChart({ data: [1, 2, 3], highlight: -1, exit: 4 }),
      LineChart({ data: [1, 3, 2], label: "l", exit: 4 }),
      ProgressRing({ value: 0.5, label: "x", exit: 4 }),
      Checklist({ items: ["a", "b"], exit: 4 }),
      ProgressBars({ items: [{ label: "a", value: 1, highlight: true }], exit: 4 }),
    ],
  });
  const json = JSON.parse(JSON.stringify(scene));
  assert.deepEqual(json, scene, "plain data only (no undefined, no functions)");
  // every keyframe list is in time order
  const walk = (l) => {
    for (const [k, v] of Object.entries(l)) {
      if (Array.isArray(v) && Array.isArray(v[0]) && typeof v[0][0] === "number" && k !== "points" && k !== "chapters") {
        for (let i = 1; i < v.length; i++) assert.ok(v[i][0] >= v[i - 1][0], `${l.type}.${k} keyframes out of order: ${JSON.stringify(v)}`);
      }
    }
    (l.children ?? []).forEach(walk);
  };
  scene.layers.forEach(walk);
});

test("measureText: estimates without flits, one or many", () => {
  const one = measureText({ text: "Hello", size: 100 });
  assert.equal(one.estimated, true);
  assert.ok(one.width > 200 && one.width < 400);
  const [a, b] = measureText([{ text: "ab", size: 50, font: "Consolas" }, { text: "abcd", size: 50, font: "Consolas" }]);
  assert.equal(b.width, a.width * 2, "mono: equal advances");
  assert.equal(measureText({ text: "a\nb" }).lines.length, 2);
});
