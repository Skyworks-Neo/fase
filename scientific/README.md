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

On 2026-09-26, `champion-glibc-pymc-verify` succeeded in Run
`run-ba70d8b70258f8fccf119f11e6feaffd`. All 31 artifacts installed
offline, `pip check` found no broken requirements, and the SciPy, Numba,
JAX CPU, PyTensor, and PyMC smoke checks passed. The tested versions include
CPython 3.13.15, OpenBLAS, NumPy 2.5.3, SciPy 1.18.1, Numba 0.67.0,
JAX and jaxlib 0.11.1, PyTensor 3.3.2, and PyMC 6.3.2.

The pandas and ml_dtypes Requests start after the shared glibc NumPy artifact
is available.

The ROCm line pins TheRock
7.14.1 and the ROCm/JAX 0.11.1 fork, targeting gfx942. The PyMC and ROCm SDK
Requests start these two independent dependency lines. The ROCm/JAX plugin
Request follows after JAX CPU and the SDK have both completed. ROCm validation
checks produced archives and wheels; this cluster has no AMD GPU for runtime
testing.

Build tools use the Tsinghua PyPI mirror, and the Ubuntu ROCm builder uses its
Ubuntu mirror. Pinned target source archives still carry SHA256 digests.

Large Tasks use `node1` or `worker1.adamanteye.cc`, with up to 32 CPUs, 64Gi
memory, 300Gi ephemeral storage and a 24-hour controller timeout. The glibc
NumPy, SciPy, LLVM, and JAX CPU builds run on `worker1.adamanteye.cc` so they
can progress alongside the ROCm SDK build on `node1`. Transfer staging volumes
are 32Gi; the artifact ceiling is 8Gi per artifact.
