#!/usr/bin/env python3
"""Generate an offline CPU verification recipe for the pinned PyMC stack."""

from pathlib import Path
import json
import yaml


ROOT = Path(__file__).resolve().parent.parent
IMAGE = "quay.io/pypa/manylinux_2_28_x86_64@sha256:407f771c51a2c3e83ebe5a7970b4289ead3a6db21d9b9c089168775cad11d328"
CORE = {
    "python": ("python-glibc", "3.13.15"),
    "numpy": ("numpy-glibc-wheels", "2.5.3"),
    "scipy": ("scipy-glibc-wheels", "1.18.1"),
    "pandas": ("pandas-glibc-wheels", "3.0.6"),
    "ml-dtypes": ("ml-dtypes-glibc-wheels", "0.6.0"),
    "llvmlite": ("llvmlite-glibc-wheels", "0.49.0"),
    "numba": ("numba-glibc-wheels", "0.67.0"),
    "filelock": ("filelock-glibc-wheels", "3.20.3"),
    "pytensor": ("pytensor-glibc-wheels", "3.3.2"),
    "pymc": ("pymc-glibc-wheels", "6.3.2"),
    "jax": ("jax-cpu-wheels", "0.11.1"),
}


class Literal(str):
    pass


class Dumper(yaml.SafeDumper):
    pass


Dumper.add_representer(Literal, lambda dumper, value: dumper.represent_scalar("tag:yaml.org,2002:str", value, style="|"))


def main():
    lock = json.loads((ROOT / "scientific/pure-lock.json").read_text())
    dependencies = dict(CORE)
    dependencies.update((package, (f"{package}-glibc-wheels", pin["version"])) for package, pin in lock.items())
    artifacts = [{"name": name, "path": f"deps/{name}", "kind": "tree"} for name in sorted(dependencies)]
    script = Literal("""set -Eeuo pipefail
umask 022
export HOME=/workspace/home TMPDIR=/workspace/tmp
mkdir -p "$HOME" "$TMPDIR" /workspace/wheelhouse
find "$FASE_INPUT_ROOT/deps" -mindepth 2 -maxdepth 2 -type f -name '*.whl' -exec cp -t /workspace/wheelhouse {} +
python="$FASE_INPUT_ROOT/deps/python/bin/python3.13"
export LD_LIBRARY_PATH="$FASE_INPUT_ROOT/deps/python/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
"$python" -m venv /workspace/venv
/workspace/venv/bin/python -m pip install --disable-pip-version-check \\
  --no-index --find-links=/workspace/wheelhouse \\
  'pymc==6.3.2' 'pytensor==3.3.2' 'jax==0.11.1' 'numba==0.67.0'
/workspace/venv/bin/python -m pip check
/workspace/venv/bin/python - <<'PY'
import numpy as np
import scipy.linalg as la
import jax
import jax.numpy as jnp
import pytensor
import pytensor.tensor as pt
import pymc as pm
from numba import njit

@njit
def add(a, b):
    return a + b

assert add(2, 3) == 5
assert np.allclose(la.solve([[3., 1.], [1., 2.]], [9., 8.]), [2., 3.])
assert jax.devices('cpu')
assert float(jnp.dot(jnp.array([1., 2.]), jnp.array([3., 4.]))) == 11.
x = pt.dscalar('x')
assert float(pytensor.function([x], x * x)(3.)) == 9.
with pm.Model():
    variable = pm.Normal('value', mu=0., sigma=1.)
    sample = pm.draw(variable, draws=2, random_seed=7)
assert len(sample) == 2
print('NumPy', np.__version__, 'JAX', jax.__version__, 'PyTensor', pytensor.__version__, 'PyMC', pm.__version__)
print('offline CPU smoke passed')
PY
{
  printf 'All target wheels installed offline from source-built Fase artifacts.\\n'
  /workspace/venv/bin/python -m pip freeze | sort
} > "$FASE_OUTPUT_ROOT/pymc-cpu-verified.txt"
""")
    task = {
        "apiVersion": "skyw.top/v1beta1", "kind": "Task",
        "metadata": {"name": "verify-pymc-cpu-offline", "labels": {"capability": "verify-pymc-cpu-offline"}},
        "spec": {
            "image": IMAGE, "command": ["/bin/bash"],
            "nodeSelector": {"kubernetes.io/hostname": "node1"},
            "securityContext": {"runAsUser": 0, "runAsNonRoot": False,
                                "readOnlyRootFilesystem": False, "allowPrivilegeEscalation": False,
                                "seccompProfile": {"type": "RuntimeDefault"}},
            "resources": {"requests": {"cpu": "4", "memory": "16Gi", "ephemeral-storage": "32Gi"},
                          "limits": {"cpu": "16", "memory": "32Gi", "ephemeral-storage": "64Gi"}},
            "workspace": {"taskSizeLimit": "64Gi", "inputSizeLimit": "32Gi",
                          "outputSizeLimit": "2Gi", "transferSizeLimit": "32Gi"},
            "inputs": {"artifacts": artifacts},
            "outputs": {"artifacts": [{"name": "report", "path": "pymc-cpu-verified.txt", "kind": "file"}]},
            "script": script,
        },
    }
    recipe = {
        "apiVersion": "skyw.top/v1beta1", "kind": "Recipe",
        "metadata": {"name": "verify-pymc-cpu-offline"},
        "spec": {
            "inputs": {"artifacts": [{"name": name, "artifactSelector": {"matchLabels": {
                "name": target_name, "version": version,
            }}} for name, (target_name, version) in sorted(dependencies.items())]},
            "tasks": [{
                "name": "verify", "taskSelector": {"matchLabels": {"capability": "verify-pymc-cpu-offline"}},
                "inputs": {"artifacts": [{"name": name, "from": {"recipeInput": name}}
                                         for name in sorted(dependencies)]},
                "outputs": [{"name": "report"}],
            }],
            "outputs": {"artifacts": [{
                "name": "report", "from": {"task": "verify", "artifact": "report"},
                "labels": {"name": {"value": "pymc-cpu-verified"},
                           "version": {"value": "6.3.2"}, "python": {"value": "cp313"}},
            }]},
        },
    }
    output = ROOT / "scientific/verify.yaml"
    output.write_text(yaml.dump_all([task, recipe], Dumper=Dumper, sort_keys=False, width=110))
    print(f"wrote {len(dependencies)} offline wheel dependencies to {output}")


if __name__ == "__main__":
    main()
