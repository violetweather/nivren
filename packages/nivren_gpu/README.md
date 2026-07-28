# nivren_gpu

Checked portable GPU-compute plans targeting WebGPU/WGSL-class hosts. `compile_add` validates item and workgroup limits, emits deterministic WGSL, and always includes a four-lane checked CPU fallback with a scalar tail. Hosts may select the GPU target only after validating the generated artifact against their adapter limits; unsupported or failed GPU execution must use `cpu_fallback` rather than changing results.

This beta package covers portable compute, not rendering, shaders with arbitrary memory access, game engines, or a promise that a GPU is present. Integer addition retains Nivren overflow behavior while building the fallback.
