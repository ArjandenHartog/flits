// Your own components: functions that take props and return layers.
import { Counter, Group, Shape, Text, type GroupLayer } from "#flits";

export interface Stat {
  label: string;
  value: number;
  decimals?: number;
  suffix?: string;
  /** how full the ring gets, 0..1 */
  ring: number;
}

/** A number counting up inside a ring that fills, with a label underneath. */
export function StatCard(p: Stat & { accent: string; duration: number; index: number }): GroupLayer {
  const ring = { shape: "circle", x: 960, y: 470, width: 380, height: 380 } as const;
  return Group({
    duration: p.duration,
    in: { type: p.index % 2 ? "push-left" : "zoom-blur", duration: 0.5 },
    children: [
      Shape({ ...ring, stroke: { width: 28, color: "#ffffff1a" } }),
      Shape({ ...ring, stroke: { width: 28, color: p.accent }, progress: { from: 0, to: p.ring, end: 1.4, ease: "out" } }),
      Counter({
        x: 960,
        y: 470,
        size: 92,
        value: { from: 0, to: p.value, end: 1.4, ease: "out" },
        decimals: p.decimals ?? 0,
        suffix: p.suffix ?? "",
      }),
      Text({ text: p.label, y: 780, size: 56, color: "#d6d3f0", in: "slide-fade 0.5" }),
    ],
  });
}
