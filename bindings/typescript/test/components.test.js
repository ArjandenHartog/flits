// node --test "bindings/typescript/test/*.test.js"
import assert from "node:assert/strict";
import { test } from "node:test";
import { At, Audio, Composition, Group, Sequence, Shape, Text, compile, easing, formats, interpolate, random, spring } from "../index.js";
import { jsx } from "../jsx-runtime.js";

test("components build layers; children flatten and skip falsy", () => {
  const scene = Composition({
    ...formats.square,
    duration: 2,
    children: [Text({ children: ["Hi ", 2] }), false && Text({ text: "no" }), [[Shape({ shape: "star" })]], null],
  });
  assert.deepEqual(scene.layers.map((l) => l.type), ["text", "shape"]);
  assert.equal(scene.layers[0].text, "Hi 2");
  assert.equal(scene.width, 1080);
});

test("function props are sampled per frame into linear keyframes", () => {
  const scene = Composition({ width: 10, height: 10, fps: 10, duration: 1, layers: [Text({ text: "x", x: (t) => t * 100, opacity: (t) => (t < 0.5 ? 0 : 1) })] });
  const x = scene.layers[0].x;
  assert.deepEqual(x[0], [0, 0]);
  assert.deepEqual(x[x.length - 1], [1, 100, "linear"]);
  assert.equal(x.length, 2, "a straight line needs only its ends");
  assert.ok(scene.layers[0].opacity.length >= 3, "a step keeps its corner points");
});

test("sequence children use their own duration; missing durations are reported", () => {
  const ok = Composition({ width: 10, height: 10, fps: 10, layers: [Sequence({ children: [Text({ text: "a", duration: 0.5, x: (t) => t })] })] });
  assert.equal(ok.layers[0].children[0].x.at(-1)[0], 0.5);
  assert.throws(() => Composition({ width: 10, height: 10, layers: [Text({ text: "a", x: (t) => t })] }), /duration/);
  assert.throws(() => Composition({ width: 10, height: 10, duration: 1, layers: [Text({ text: "a", size: () => 3 })] }), /animatable/);
});

test("Audio among the layers moves to the soundtrack; At starts a group", () => {
  const s = Composition({ width: 10, height: 10, duration: 2, children: [Audio({ src: "m.mp3", volume: (t) => 1 - t / 2 }), At({ time: 1, children: Text({ text: "x" }) })] });
  assert.equal(s.layers.length, 1);
  assert.equal(s.layers[0].start, 1);
  assert.equal(s.audio[0].src, "m.mp3");
  assert.ok(Array.isArray(s.audio[0].volume));
});

test("JSX runtime calls components", () => {
  const g = jsx(Group, { x: 5, children: [jsx(Text, { children: "a" }), jsx(Text, { children: "b" })] });
  assert.equal(g.type, "group");
  assert.deepEqual(g.children.map((c) => c.text), ["a", "b"]);
  assert.throws(() => jsx("div", {}), /not a flits component/);
});

test("motion helpers", () => {
  assert.equal(interpolate(5, [0, 10], [0, 100]), 50);
  assert.equal(interpolate(20, [0, 10], [0, 100]), 100);
  assert.equal(interpolate(20, [0, 10], [0, 100], { extrapolate: "extend" }), 200);
  assert.equal(interpolate(15, [0, 10, 20], [0, 100, 0]), 50);
  assert.equal(spring({ t: 0 }), 0);
  assert.ok(Math.abs(spring({ t: 5 }) - 1) < 1e-3);
  assert.ok(Math.max(...Array.from({ length: 60 }, (_, f) => spring({ frame: f }))) > 1, "default spring overshoots");
  for (const [name, f] of Object.entries(easing)) {
    assert.ok(Math.abs(f(0)) < 1e-9 && Math.abs(f(1) - 1) < 1e-9, name);
  }
  assert.equal(random("a"), random("a"));
  assert.notEqual(random(1), random(2));
});

test("compile is idempotent", () => {
  const s = Composition({ width: 10, height: 10, duration: 1, layers: [Text({ text: "x", x: (t) => t })] });
  assert.deepEqual(compile(s), s);
});
