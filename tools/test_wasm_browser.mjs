import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { Nivren, NivrenError } from "../sdk/browser/nivren.mjs";

const path = process.argv[2] ?? "target/wasm32-unknown-unknown/release/nivren_wasm.wasm";
const nivren = await Nivren.instantiate(await readFile(path));

nivren.check("keep answer: Int = 42");
assert.equal(nivren.format("keep answer=42"), "keep answer=42\n");
assert.equal(new TextDecoder().decode(nivren.compile("40 + 2").slice(0, 4)), "NIVB");
assert.equal(nivren.run(`
choice Maybe<Value> { Some(Value), None }
define double(value: Int) gives Int { give value * 2 }
keep value: Maybe<Int> = Maybe.Some(double(21))
choose value { Some(answer) => answer, None => 0 }
`), "42");
assert.throws(
  () => nivren.check("keep answer: Int = yes"),
  error => error instanceof NivrenError && error.status === 1 && /expected Int, found Bool/.test(error.message),
);

console.log("Nivren browser host: check, diagnostics, format, compile, and Edition 3 execution passed");
