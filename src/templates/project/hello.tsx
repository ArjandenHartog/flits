/** @jsxImportSource #flits */
// The same components as JSX. Runs with Bun or Deno (Node's type stripping does not do JSX; use .ts there):
//   flits preview src/hello.tsx
import { Composition, Sequence, Shape, Text, formats } from "#flits";

export default (
  <Composition {...formats.square} fps={30} duration={4} background="#111318">
    <Shape shape="circle" width={700} height={700} stroke={{ width: 24, color: "#ffd60a" }} progress={{ from: 0, to: 1 }} />
    <Sequence>
      <Text size={150} duration={2} in="pop">
        Hello
      </Text>
      <Text size={150} duration={2} in="zoom-blur" color="#ffd60a">
        JSX ✨
      </Text>
    </Sequence>
  </Composition>
);
