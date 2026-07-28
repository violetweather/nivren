# nivren_gpu

Checked portable GPU-compute plans targeting WebGPU/WGSL-class hosts. `compile_add` validates item/workgroup limits, emits deterministic WGSL, and always includes a four-lane checked CPU fallback with a scalar tail. `execute_gpu` crosses one opaque `Native within "gpu"` boundary, sends the inspected artifact, and rejects result lengths that disagree with the fallback. Unsupported, failed, or slower GPU execution must use `cpu_fallback` rather than changing results.

This beta package covers portable compute, not rendering, arbitrary-memory shaders, game engines, or a promise that a GPU is present. Integer addition retains Nivren overflow behavior. VM/native-control Product Proof verifies identical host operations, exactly-once cleanup, and GPU/CPU result parity; real WebGPU platform evidence remains gated.
