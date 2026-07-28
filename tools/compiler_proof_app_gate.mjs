import assert from "node:assert/strict";
import { mkdirSync, writeFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { performance } from "node:perf_hooks";

const executable = process.platform === "win32" ? "target/release/niv.exe" : "target/release/niv";
mkdirSync("target", { recursive: true });
const payload = "nivren-compiler-proof-".repeat(4096);
writeFileSync("target/compiler-proof-input.txt", payload);

function run(command, args) {
  const started = performance.now();
  const result = spawnSync(command, args, { encoding: "utf8" });
  const elapsed = performance.now() - started;
  assert.equal(result.status, 0, result.stderr);
  assert.equal(result.stdout, `${payload}\n`);
  return elapsed;
}

function median(command, args) {
  for (let index = 0; index < 3; index += 1) run(command, args);
  const samples = [];
  for (let index = 0; index < 11; index += 1) samples.push(run(command, args));
  samples.sort((left, right) => left - right);
  return samples[Math.floor(samples.length / 2)];
}

const nivren = median(executable, ["run", "--native", "benchmarks/compiler_proof_app.niv"]);
const node = median(process.execPath, ["benchmarks/compiler_proof_app.mjs"]);
const ratio = nivren / node;
console.log(`nivren_compiler_app_native_ms ${nivren.toFixed(3)}`);
console.log(`nivren_compiler_app_node_ms ${node.toFixed(3)}`);
console.log(`nivren_compiler_app_node_ratio ${ratio.toFixed(3)}`);
if (ratio > 1.5) {
  throw new Error(`compiler performance gate failed: native app ratio ${ratio.toFixed(3)} exceeds 1.5`);
}
