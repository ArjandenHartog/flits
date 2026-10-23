// <FlitsScene scene={...}/>: a small interpreter that draws a flits scene.json with plain
// React/CSS, the way a Remotion user would write it by hand. Every layer becomes an absolutely
// positioned <div> whose CSS transform mirrors flits' transform order:
//   translate(x, y) · rotate · scale · translate(-anchor · size)
// Supported: rect (colour, linear/radial gradient), image (fit, zoom/pan Ken Burns, blur),
// video (OffthreadVideo), text (font, size, colour, shadow, stroke), group (sequence/overlap/clip),
// keyframed x/y/scale/scaleX/scaleY/rotation/opacity/zoom/panX/panY, in/out transitions,
// radius, border, shadow, and scene audio tracks. Not supported (rendered as nothing):
// counter, captions, waveform, text reveal, colour filters.
import React, { useMemo } from "react";
import { AbsoluteFill, Audio, Img, OffthreadVideo, Sequence, staticFile, useCurrentFrame, useVideoConfig } from "remotion";
import { buildTimeline, evalProp, Layer, Scene, TimedLayer, transitionFx } from "./scene";

const url = (src: string) => (/^(https?:|data:|blob:)/.test(src) ? src : staticFile(src.replace(/\\/g, "/")));

/** CSS blur radius for a gaussian with standard deviation `sigma` (flits blur values are sigmas). */
const cssBlur = (sigma: number) => sigma * 2;

function boxShadow(l: Layer): string | undefined {
  const parts: string[] = [];
  const bw = l.border?.width ?? 0;
  if (l.border && bw > 0) parts.push(`0 0 0 ${bw}px ${l.border.color}`);
  if (l.shadow) {
    const s = l.shadow;
    parts.push(`${s.x ?? 0}px ${s.y ?? 12}px ${cssBlur(s.blur ?? 24)}px ${bw}px ${s.color ?? "rgba(0,0,0,0.5)"}`);
  }
  return parts.length ? parts.join(", ") : undefined;
}

function rectFill(l: Layer): string {
  const g = l.gradient;
  if (!g) return l.color ?? "#ffffff";
  let stops = g.stops ?? [];
  if (stops.length === 0) {
    const cs = g.colors ?? [];
    const n = Math.max(cs.length, 2) - 1;
    stops = cs.map((c, i) => [i / n, c]);
  }
  const list = stops.map(([p, c]) => `${c} ${(p * 100).toFixed(2)}%`).join(", ");
  return g.type === "radial"
    ? `radial-gradient(circle farthest-corner at 50% 50%, ${list})`
    : `linear-gradient(${g.angle ?? 180}deg, ${list})`;
}

/** Style for the <img> of an image/video layer: fit + Ken Burns zoom/pan (same maths as flits' media_op). */
function mediaStyle(l: Layer, lt: number, dur: number, boxKnown: { w: boolean; h: boolean }): React.CSSProperties {
  const zoom = evalProp(l.zoom, lt, dur, 1);
  const px = (1 + evalProp(l.panX, lt, dur, 0)) * 50;
  const py = (1 + evalProp(l.panY, lt, dur, 0)) * 50;
  const blur = l.blur ?? 0;
  // CSS blur fades to transparent at the edges, flits clamps: let the blurred image overhang the box
  const m = blur > 0 && boxKnown.w && boxKnown.h ? Math.ceil(blur * 3) : 0;
  return {
    position: m ? "absolute" : undefined,
    left: m ? -m : undefined,
    top: m ? -m : undefined,
    display: "block",
    width: boxKnown.w ? `calc(100% + ${2 * m}px)` : "auto",
    height: boxKnown.h ? `calc(100% + ${2 * m}px)` : "auto",
    objectFit: l.fit ?? "cover",
    objectPosition: `${px}% ${py}%`,
    transform: zoom !== 1 ? `scale(${zoom})` : undefined,
    transformOrigin: `${px}% ${py}%`,
    filter: blur > 0 ? `blur(${blur}px)` : undefined,
  };
}

const LayerView: React.FC<{ node: TimedLayer; t: number; pw: number; ph: number; fps: number }> = ({ node, t, pw, ph, fps }) => {
  const { layer: l, start, end } = node;
  if (t < start || t >= end) return null;
  const lt = t - start;
  const dur = end - start;
  const fx = transitionFx(l, lt, dur);
  const opacity = Math.min(1, Math.max(0, evalProp(l.opacity, lt, dur, 1))) * fx.opacity;
  if (opacity <= 0.002) return null;
  const s = evalProp(l.scale, lt, dur, 1) * fx.scale;
  const sx = s * evalProp(l.scaleX, lt, dur, 1);
  const sy = s * evalProp(l.scaleY, lt, dur, 1);
  if (Math.abs(sx) < 1e-4 || Math.abs(sy) < 1e-4) return null;
  const x = evalProp(l.x, lt, dur, pw / 2) + fx.dx;
  const y = evalProp(l.y, lt, dur, ph / 2) + fx.dy;
  const rot = evalProp(l.rotation, lt, dur, 0);
  const [ax, ay] = l.anchor ?? [0.5, 0.5];

  // box size: explicit, else the parent's box (text and one-sided media size themselves)
  const isMedia = l.type === "image" || l.type === "video";
  const sizesItself = l.type === "text" || (isMedia && (l.width === undefined) !== (l.height === undefined));
  const w = l.width ?? (sizesItself ? undefined : pw);
  const h = l.height ?? (sizesItself ? undefined : ph);

  const style: React.CSSProperties = {
    position: "absolute",
    left: 0,
    top: 0,
    width: w,
    height: h,
    transformOrigin: "0 0",
    transform: `translate(${x}px, ${y}px) rotate(${rot}deg) scale(${sx}, ${sy}) translate(${-ax * 100}%, ${-ay * 100}%)`,
    opacity,
    borderRadius: l.radius ? l.radius : undefined,
    boxShadow: l.type === "text" ? undefined : boxShadow(l),
    clipPath: fx.clip,
  };

  let content: React.ReactNode = null;
  switch (l.type) {
    case "group":
      if (l.clip) style.overflow = "hidden";
      content = node.children.map((c, i) => <LayerView key={i} node={c} t={t} pw={w ?? pw} ph={h ?? ph} fps={fps} />);
      break;
    case "rect":
      style.background = rectFill(l);
      break;
    case "image":
      style.overflow = "hidden";
      content = <Img src={url(l.src ?? "")} style={mediaStyle(l, lt, dur, { w: w !== undefined, h: h !== undefined })} />;
      break;
    case "video": {
      style.overflow = "hidden";
      const vol = l.volume ?? 1;
      content = (
        <Sequence from={Math.round(start * fps)} durationInFrames={Math.max(1, Math.round(dur * fps))} layout="none">
          <OffthreadVideo
            src={url(l.src ?? "")}
            muted={!(vol > 0)}
            volume={vol}
            playbackRate={l.speed ?? 1}
            trimBefore={l.trim ? Math.round(l.trim * fps) : undefined}
            style={mediaStyle(l, lt, dur, { w: w !== undefined, h: h !== undefined })}
          />
        </Sequence>
      );
      break;
    }
    case "text": {
      const size = l.size ?? 64;
      const sh = l.shadow;
      Object.assign(style, {
        fontFamily: l.font && !/\.(ttf|otf)$/i.test(l.font) ? `"${l.font}"` : '"Segoe UI"',
        fontWeight: l.weight ?? 700,
        fontSize: size,
        lineHeight: l.lineHeight ?? 1.15,
        letterSpacing: l.letterSpacing ? `${l.letterSpacing}px` : undefined,
        color: l.color ?? "#ffffff",
        textAlign: l.align ?? "center",
        textTransform: l.uppercase ? "uppercase" : undefined,
        whiteSpace: l.maxWidth ? "pre-wrap" : "pre",
        maxWidth: l.maxWidth,
        textShadow: sh ? `${sh.x ?? 0}px ${sh.y ?? 12}px ${cssBlur(sh.blur ?? 24)}px ${sh.color ?? "rgba(0,0,0,0.5)"}` : undefined,
        WebkitTextStroke: l.stroke ? `${l.stroke.width}px ${l.stroke.color}` : undefined,
        paintOrder: l.stroke ? "stroke fill" : undefined,
      } as React.CSSProperties);
      content = l.text ?? "";
      break;
    }
    default:
      return null; // counter, captions, waveform: not part of this benchmark
  }
  return <div style={style}>{content}</div>;
};

export const FlitsScene: React.FC<{ scene: Scene }> = ({ scene }) => {
  const frame = useCurrentFrame();
  const { fps, durationInFrames } = useVideoConfig();
  const { nodes, duration } = useMemo(() => buildTimeline(scene), [scene]);
  const t = frame / fps;
  return (
    <AbsoluteFill style={{ background: scene.background ?? "#000000", overflow: "hidden" }}>
      {nodes.map((n, i) => (
        <LayerView key={i} node={n} t={t} pw={scene.width} ph={scene.height} fps={fps} />
      ))}
      {(scene.audio ?? []).map((a, i) => {
        const from = Math.round((a.start ?? 0) * fps);
        const len = Math.min(a.duration ?? Infinity, duration - (a.start ?? 0));
        const frames = Math.max(1, Math.min(durationInFrames - from, Math.round(len * fps)));
        const vol = a.volume ?? 1;
        return (
          <Sequence key={`a${i}`} from={from} durationInFrames={frames} layout="none">
            <Audio
              src={url(a.src)}
              loop={a.loop}
              trimBefore={a.trim ? Math.round(a.trim * fps) : undefined}
              volume={(f) => {
                const tt = f / fps;
                let g = vol;
                if (a.fadeIn) g *= Math.min(1, tt / a.fadeIn);
                if (a.fadeOut) g *= Math.min(1, Math.max(0, (len - tt) / a.fadeOut));
                return g;
              }}
            />
          </Sequence>
        );
      })}
    </AbsoluteFill>
  );
};
