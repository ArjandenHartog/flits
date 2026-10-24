import React from "react";
import { CalculateMetadataFunction, Composition } from "remotion";
import { FlitsScene } from "./FlitsScene";
import { buildTimeline, Scene } from "./scene";

type Props = { scene: Scene };

// Small built-in scene so `npm run studio` works without --props.
const demo: Scene = {
  width: 1920,
  height: 1080,
  fps: 30,
  duration: 4,
  background: "#1b1020",
  layers: [
    { type: "rect", gradient: { type: "radial", colors: ["#f26328", "#120a1c"] } },
    { type: "text", text: "flits scene in Remotion", size: 96, shadow: { blur: 18, y: 4 }, in: "pop" },
  ],
};

// Size, fps and length come from the scene passed with --props='{"scene": ...}'.
const calculateMetadata: CalculateMetadataFunction<Props> = ({ props }) => {
  const fps = props.scene.fps ?? 30;
  const { duration } = buildTimeline(props.scene);
  return {
    width: props.scene.width,
    height: props.scene.height,
    fps,
    durationInFrames: Math.max(1, Math.round(duration * fps)),
  };
};

export const RemotionRoot: React.FC = () => (
  <Composition
    id="FlitsScene"
    component={FlitsScene}
    defaultProps={{ scene: demo }}
    calculateMetadata={calculateMetadata}
    width={1920}
    height={1080}
    fps={30}
    durationInFrames={120}
  />
);
