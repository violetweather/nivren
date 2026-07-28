import { readFileSync } from "node:fs";

const contents = readFileSync("target/compiler-proof-input.txt", "utf8");
process.stdout.write(`${contents}\n`);
