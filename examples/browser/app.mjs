import { Nivren } from "../../sdk/browser/nivren.mjs";

const runtime = await Nivren.instantiate("./nivren_wasm.wasm");
const source = document.querySelector("#source");
const result = document.querySelector("#result");

document.querySelector("#run").addEventListener("click", () => {
  try {
    result.textContent = runtime.run(source.value);
  } catch (error) {
    result.textContent = error.message;
  }
});
