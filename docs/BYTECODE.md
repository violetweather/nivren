# Nivren Bytecode 1

Nivren 0.5 compiles checked source into versioned stack bytecode. The VM never executes a decoded bundle until structural and control-flow verification succeeds.

## Bundle envelope

A bundle begins with the four ASCII bytes `NIVB`, followed by a little-endian bytecode chunk. Every chunk contains:

1. a little-endian 16-bit bytecode version;
2. a little-endian 32-bit instruction count;
3. source line and column metadata plus an encoded operation for each instruction.

Strings are UTF-8 prefixed by a little-endian 32-bit byte length. Nested functions, match arms, iterators, and modules contain recursively encoded chunks. Decoder limits cap individual counts and nesting depth, reject integer overflow and truncation, and reject trailing bytes.

## Verification

The verifier rejects unsupported versions, unknown or invalid operands, out-of-range jumps, stack underflow, scope underflow, unclosed scopes, and control-flow joins with inconsistent stack or scope depths. It recursively verifies every nested chunk.

## Runtime and memory

`niv run application.nivb` decodes, verifies, and executes a self-contained bundle. `niv check application.nivb` verifies it without execution, and `niv disasm application.nivb` prints deterministic structured assembly with source positions.

Closure environments are managed by a precise reachability collector behind the runtime collector interface. `NIVREN_GC_STRESS=1` collects after every safe bytecode instruction and is exercised by the conformance suite.
