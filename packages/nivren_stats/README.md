# nivren_stats

Official small-data descriptive statistics for Nivren Edition 3.

The package provides deterministic sum, mean, population variance, minimum, maximum, and min-max normalization over bounded in-memory `[Float]` values. Empty or zero-range operations return typed errors. It uses no ambient capabilities or hidden native code, and numeric conversion remains explicit.

This package is the stable scalar foundation for scientific and data-processing programs; large columnar tables, vectorized kernels, and accelerator integrations remain separate packages so their memory and device policies stay visible.
