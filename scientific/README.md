# Scientific compilation champion

`legacy.yaml` is generated from the source build recipes in the historical
`../fase-crd` checkout by `../tools/port_legacy_science.py`. Its Tasks and
Recipes bring the musl Python 3.13.15, OpenBLAS 0.3.34, NumPy 2.5.3, SciPy
1.18.1, and ml_dtypes 0.6.0 dependency graph into v1beta1. The initial
Request targets SciPy; ml_dtypes follows after the shared NumPy chain succeeds.

The historical `.tar.zst` packages remain `file` artifacts. Each consuming
Task unpacks them into its 300Gi workspace before running the original build
script. Source tarballs are fetched with pinned SHA256 digests. Native kernel
building and package publication are outside this build graph.

Large Tasks are pinned to `node1` with 32 CPUs, 64Gi memory, 300Gi ephemeral
storage and a 24-hour controller timeout. Transfer staging volumes are 32Gi;
the artifact ceiling is 8Gi per file.
