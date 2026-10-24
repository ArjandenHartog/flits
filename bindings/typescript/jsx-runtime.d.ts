export { Fragment } from "./components.js";

export function jsx(type: (props: any) => unknown, props: unknown): any;
export const jsxs: typeof jsx;
export const jsxDEV: typeof jsx;

export namespace JSX {
  // components return layers, scenes or arrays of them
  type Element = any;
  interface ElementChildrenAttribute {
    children: {};
  }
  interface IntrinsicElements {}
}
