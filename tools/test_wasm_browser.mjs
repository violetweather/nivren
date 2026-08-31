import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { Nivren, NivrenError } from "../sdk/browser/nivren.mjs";

const path = process.argv[2] ?? "target/wasm32-unknown-unknown/release/nivren_wasm.wasm";
const nivren = await Nivren.instantiate(await readFile(path));

nivren.check("keep answer is Int set 42");
assert.equal(nivren.format("keep  answer   set 42"), "keep answer set 42\n");
assert.equal(new TextDecoder().decode(nivren.compile("40 + 2").slice(0, 4)), "NIVB");
assert.equal(nivren.run(`
shape NumberPlan holds { value is Int }
choice Calculation holds {
  case Ready carries NumberPlan
  case Missing
}
define double takes { value is Int } gives Int { give value * 2 }
prepare plan as NumberPlan with { value set 21 }
choose Calculation.Ready(perform plan) {
  case Ready carries ready => double with { value set ready.value }
  case Missing => 0
}
`), "42");
assert.throws(
  () => nivren.check("keep answer is Int set yes"),
  error => error instanceof NivrenError && error.status === 1 && /expected Int, found Bool/.test(error.message),
);

console.log("Nivren browser host: check, diagnostics, format, compile, and Edition 5 execution passed");
