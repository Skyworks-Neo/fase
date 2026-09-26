# Scientific compilation champion

`sources.yaml` is generated from the pinned source fetches in the historical
`../fase-crd` checkout by `../tools/port_science_sources.py`. It retains only
the source tarballs needed by the glibc builds, with pinned SHA256 digests.
Native kernel building and package publication are outside this build graph.

The glibc line defines source builds of CPython, OpenBLAS, NumPy, SciPy,
LLVM 22, llvmlite, Numba, pandas, ml_dtypes, JAX and jaxlib, PyTensor and PyMC.
`pure-lock.json` pins the remaining Python runtime sdists by URL and SHA256;
`gen_pure_science.py` creates their fetch and wheel Recipes. The offline
verification Recipe combines 31 wheel/prefix artifacts, installs with
`--no-index`, then exercises SciPy linear algebra, Numba, JAX CPU, PyTensor,
and PyMC.

The ROCm line pins TheRock
7.14.1 and the ROCm/JAX 0.11.1 fork, targeting gfx942. The PyMC and ROCm SDK
Requests start these two independent dependency lines. The ROCm/JAX plugin
Request follows after JAX CPU and the SDK have both completed. ROCm validation
checks produced archives and wheels; this cluster has no AMD GPU for runtime
testing.

Large Tasks are pinned to `node1` with up to 32 CPUs, 64Gi memory, 300Gi
ephemeral storage and a 24-hour controller timeout. Transfer staging volumes
are 32Gi; the artifact ceiling is 8Gi per artifact.
