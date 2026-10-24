// flits scene loader: runs a JavaScript/TypeScript module and writes the scene it exports as JSON.
//   <runtime> loader.mjs <scene module> <props JSON> <output file>
// The module's default export (or `scene` export) is a scene object, or a function (props) => scene
// (may be async). Function-valued animatable properties are sampled per frame by compile.js.
import { writeFile } from "node:fs/promises";
import process from "node:process";
import { pathToFileURL } from "node:url";
import { compile } from "./compile.js";

const [file, propsJson, out] = globalThis.Deno ? globalThis.Deno.args : process.argv.slice(2);
const mod = await import(pathToFileURL(file).href);
let scene = mod.default ?? mod.scene;
if (scene === undefined) {
  throw new Error(`${file}: no scene exported (write \`export default <scene>\` or \`export default (props) => <scene>\`)`);
}
if (typeof scene === "function") scene = await scene(JSON.parse(propsJson || "{}"));
await writeFile(out, JSON.stringify(compile(await scene)));
