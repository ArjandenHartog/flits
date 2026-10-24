// node --test "bindings/typescript/test/*.test.js"
import assert from "node:assert/strict";
import { test } from "node:test";
import { resolve } from "node:path";
import { reelArgs, toArgs } from "../index.js";

test("toArgs turns camelCase options into flags", () => {
  assert.deepEqual(toArgs({ keepTemp: true, workers: 4, quiet: false, at: [1, 2] }), ["--keep-temp", "--workers", "4", "--at", "1", "--at", "2"]);
});

test("reelArgs mirrors the flits reel flags", () => {
  const args = reelArgs("in.mp4", "out/reel.mp4", {
    from: "1:12",
    to: 100,
    fillers: true,
    noPunch: true,
    logo: "logo.png",
    accent: "#f39200",
    end: "Volg ons",
    endDuration: 2.5,
    focus: 0.3,
    bin: "flits-test",
    quiet: true,
  });
  assert.deepEqual(args.slice(0, 4), ["reel", resolve("in.mp4"), "-o", resolve("out/reel.mp4")]);
  const flags = args.slice(4).join(" ");
  assert.match(flags, /--from 1:12 --to 100 --fillers --no-punch/);
  assert.match(flags, new RegExp(`--logo ${resolve("logo.png").replace(/\\/g, "\\\\")}`));
  assert.match(flags, /--end Volg ons --end-duration 2.5 --focus 0.3 --quiet/);
  assert.ok(!flags.includes("bin"), "run options are not flags");
});

test("reelArgs: clips, auto, endLogo and pick", () => {
  const clips = reelArgs("in.mp4", null, { clips: [["0:40", "1:05"], [190, 212], "5:00-5:20"] });
  assert.deepEqual(clips.slice(2), ["--clips", "0:40-1:05,190-212,5:00-5:20"]);
  assert.deepEqual(reelArgs("in.mp4", undefined, { auto: true, candidates: 3 }).slice(2), ["--auto", "--candidates", "3"]);
  assert.deepEqual(reelArgs("in.mp4", null, { auto: 30 }).slice(2), ["--auto", "30"]);
  assert.deepEqual(reelArgs("in.mp4", null, { endLogo: true }).slice(2), ["--end-logo"]);
  assert.deepEqual(reelArgs("in.mp4", null, { endLogo: "mark.svg" }).slice(2), ["--end-logo", resolve("mark.svg")]);
  // pick prints JSON for the wrapper to parse
  assert.deepEqual(reelArgs("in.mp4", null, { pick: true, auto: 60 }).slice(2), ["--pick", "--auto", "60", "--json"]);
});
