const MAXIMUM_BYTES = 16 * 1024 * 1024;

export class NivrenError extends Error {
  constructor(status, message) {
    super(message || `Nivren operation failed with status ${status}`);
    this.name = "NivrenError";
    this.status = status;
  }
}

export class Nivren {
  static async instantiate(source) {
    let bytes;
    if (typeof source === "string" || source instanceof URL) {
      const response = await fetch(source);
      if (!response.ok) throw new Error(`Could not load Nivren WebAssembly: HTTP ${response.status}`);
      bytes = await response.arrayBuffer();
    } else if (source instanceof Response) {
      if (!source.ok) throw new Error(`Could not load Nivren WebAssembly: HTTP ${source.status}`);
      bytes = await source.arrayBuffer();
    } else {
      bytes = source;
    }
    const result = await WebAssembly.instantiate(bytes, {});
    const instance = result instanceof WebAssembly.Instance ? result : result.instance;
    return new Nivren(instance);
  }

  constructor(instance) {
    this.instance = instance;
    this.exports = instance.exports;
    if (this.exports.nivren_wasm_abi_version() !== 1) {
      throw new Error("Unsupported Nivren WebAssembly ABI");
    }
  }

  check(source) {
    this.#invoke("nivren_wasm_check", source, false);
  }

  format(source) {
    return new TextDecoder().decode(this.#invoke("nivren_wasm_format", source));
  }

  compile(source) {
    return this.#invoke("nivren_wasm_compile", source);
  }

  run(source) {
    return new TextDecoder().decode(this.#invoke("nivren_wasm_run", source));
  }

  #invoke(name, source, returnBytes = true) {
    if (typeof source !== "string") throw new TypeError("Nivren source must be a string");
    const input = new TextEncoder().encode(source);
    if (input.length > MAXIMUM_BYTES) throw new RangeError("Nivren source exceeds 16 MiB");
    const pointer = input.length === 0 ? 0 : this.exports.nivren_wasm_alloc(input.length);
    if (input.length > 0 && pointer === 0) throw new Error("Nivren WebAssembly allocation failed");
    if (input.length > 0) new Uint8Array(this.exports.memory.buffer, pointer, input.length).set(input);
    let packed;
    try {
      packed = this.exports[name](pointer, input.length);
    } finally {
      if (input.length > 0) this.exports.nivren_wasm_free(pointer, input.length);
    }
    const outputPointer = Number(packed & 0xffff_ffffn);
    const outputLength = Number((packed >> 32n) & 0x0fff_ffffn);
    const status = Number(packed >> 60n);
    if (outputLength > MAXIMUM_BYTES) throw new Error("Nivren returned an invalid result length");
    let output;
    try {
      output = new Uint8Array(this.exports.memory.buffer, outputPointer, outputLength).slice();
    } finally {
      if (outputLength > 0) this.exports.nivren_wasm_free(outputPointer, outputLength);
    }
    if (status !== 0) throw new NivrenError(status, new TextDecoder().decode(output));
    return returnBytes ? output : undefined;
  }
}
