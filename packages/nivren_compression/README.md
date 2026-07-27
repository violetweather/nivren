# nivren_compression

Official bounded deterministic gzip and zlib codecs for Nivren Edition 3.

Compression inputs and outputs are capped at 16 MiB. Decompression always requires a caller-selected output ceiling, preventing unbounded expansion. Gzip headers use a fixed zero timestamp so identical input, level, and release produce identical bytes. Invalid streams and limit failures are typed `Result` errors.
