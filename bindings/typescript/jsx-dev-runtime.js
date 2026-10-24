// (development build of the JSX runtime: identical)
// JSX for flits components: with `"jsx": "react-jsx"` and `"jsxImportSource"` pointing at this package,
// <Text size={80}>Hello</Text> calls Text({ size: 80, children: "Hello" }). Works with Bun, Deno, tsx and
// any bundler; Node's built-in type stripping does not transform JSX (use .ts there).

import { Fragment } from "./components.js";

export { Fragment };

export function jsx(type, props) {
  if (typeof type !== "function") {
    throw new Error(`<${String(type)}>: not a flits component (use Text, Image, Video, Group, Sequence, ... from the flits SDK)`);
  }
  return type(props ?? {});
}

export const jsxs = jsx;
export const jsxDEV = jsx;
