import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { WASI } from "node:wasi";

const path = process.argv[2] ?? "target/wasm32-wasip1/release/nivren_wasm.wasm";
const bytes = await readFile(path);
const wasi = new WASI({ version: "preview1" });
const module = await WebAssembly.compile(bytes);
const instance = await WebAssembly.instantiate(module, { wasi_snapshot_preview1: wasi.wasiImport });
wasi.initialize(instance);

const api = instance.exports;
assert.equal(api.nivren_wasm_abi_version(), 1);

function invoke(name, source) {
  const input = new TextEncoder().encode(source);
  const pointer = api.nivren_wasm_alloc(input.length);
  assert.notEqual(pointer, 0);
  new Uint8Array(api.memory.buffer, pointer, input.length).set(input);
  const packed = api[name](pointer, input.length);
  api.nivren_wasm_free(pointer, input.length);
  const outputPointer = Number(packed & 0xffff_ffffn);
  const outputLength = Number((packed >> 32n) & 0x0fff_ffffn);
  const status = Number(packed >> 60n);
  const output = new Uint8Array(api.memory.buffer, outputPointer, outputLength).slice();
  if (outputLength > 0) api.nivren_wasm_free(outputPointer, outputLength);
  return { status, output };
}

assert.equal(invoke("nivren_wasm_check", "keep answer is Int set 42").status, 0);
const invalid = invoke("nivren_wasm_check", "keep answer is Int set yes");
assert.equal(invalid.status, 1);
assert.match(new TextDecoder().decode(invalid.output), /expected Int, found Bool/);
const formatted = invoke("nivren_wasm_format", "keep  answer   set 42");
assert.equal(new TextDecoder().decode(formatted.output), "keep answer set 42\n");
const compiled = invoke("nivren_wasm_compile", "40 + 2");
assert.equal(compiled.status, 0);
assert.equal(new TextDecoder().decode(compiled.output.slice(0, 4)), "NIVB");
const executed = invoke("nivren_wasm_run", `
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
`);
assert.equal(executed.status, 0, new TextDecoder().decode(executed.output));
assert.equal(new TextDecoder().decode(executed.output), "42");

assert.equal(api.nivren_wasm_alloc(16 * 1024 * 1024 + 1), 0);

console.log("Nivren WASI host/guest: check, diagnostics, format, compile, and Edition 5 execution passed");
