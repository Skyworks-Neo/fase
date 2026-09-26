#!/usr/bin/env python3
"""Generate pinned, source-built wheel recipes from scientific/pure-lock.json."""

from pathlib import Path
import json
import yaml


ROOT = Path(__file__).resolve().parent.parent
IMAGE = "quay.io/pypa/manylinux_2_28_x86_64@sha256:407f771c51a2c3e83ebe5a7970b4289ead3a6db21d9b9c089168775cad11d328"


class Literal(str):
    pass


class Dumper(yaml.SafeDumper):
    pass


Dumper.add_representer(Literal, lambda dumper, value: dumper.represent_scalar("tag:yaml.org,2002:str", value, style="|"))


def literal(value):
    return {"value": value}


def main():
    lock = json.loads((ROOT / "scientific/pure-lock.json").read_text())
    documents = [{
        "apiVersion": "skyw.top/v1beta1",
        "kind": "Task",
        "metadata": {"name": "build-pinned-python-sdist", "labels": {"capability": "build-pinned-python-sdist"}},
        "spec": {
            "image": IMAGE,
            "command": ["/bin/bash"],
            "securityContext": {
                "runAsUser": 0, "runAsNonRoot": False, "readOnlyRootFilesystem": False,
                "allowPrivilegeEscalation": False, "seccompProfile": {"type": "RuntimeDefault"},
            },
            "resources": {
                "requests": {"cpu": "1", "memory": "2Gi", "ephemeral-storage": "2Gi"},
                "limits": {"cpu": "4", "memory": "8Gi", "ephemeral-storage": "16Gi"},
            },
            "inputs": {
                "variables": [
                    {"name": "package", "env": "FASE_VAR_PACKAGE", "required": True},
                    {"name": "version", "env": "FASE_VAR_VERSION", "required": True},
                ],
                "artifacts": [{"name": "source", "path": "source.tar.gz", "kind": "file"}],
            },
            "outputs": {"artifacts": [{"name": "wheels", "path": "wheels", "kind": "tree"}]},
            "script": Literal("""set -Eeuo pipefail
umask 022
python=/opt/python/cp313-cp313/bin/python
mkdir -p /tmp/source "$FASE_OUTPUT_ROOT/wheels"
tar -xf "$FASE_INPUT_ROOT/source.tar.gz" -C /tmp/source
src=$(find /tmp/source -mindepth 1 -maxdepth 1 -type d | head -1)
test -n "$src"
build_args=()
if [ "$FASE_VAR_PACKAGE" = cloudpickle ]; then
  "$python" -m pip install --disable-pip-version-check --no-cache-dir 'flit_core==3.12.0'
  build_args+=(--no-build-isolation)
fi
"$python" -m pip wheel --disable-pip-version-check --no-cache-dir --no-deps \\
  "${build_args[@]}" --wheel-dir "$FASE_OUTPUT_ROOT/wheels" "$src"
FASE_WHEELS="$FASE_OUTPUT_ROOT/wheels" "$python" - <<'PY'
import email.parser, glob, os, re, zipfile
wheels = glob.glob(os.environ['FASE_WHEELS'] + '/*.whl')
assert len(wheels) == 1, wheels
with zipfile.ZipFile(wheels[0]) as archive:
    metadata = next(name for name in archive.namelist() if name.endswith('.dist-info/METADATA'))
    package = email.parser.Parser().parsestr(archive.read(metadata).decode())
    normalize = lambda name: re.sub(r'[-_.]+', '-', name).lower()
    assert normalize(package['Name']) == normalize(os.environ['FASE_VAR_PACKAGE']), package['Name']
    assert package['Version'] == os.environ['FASE_VAR_VERSION'], package['Version']
    assert archive.testzip() is None
print(wheels[0], package['Name'], package['Version'])
PY
chmod -R a+rX "$FASE_OUTPUT_ROOT/wheels"
ls -ld "$FASE_OUTPUT_ROOT/wheels" "$FASE_OUTPUT_ROOT/wheels"/*.whl
"""),
        },
    }]
    for package, pin in sorted(lock.items()):
        version = pin["version"]
        source_name = f"{package}-source"
        wheel_name = f"{package}-glibc-wheels"
        documents.append({
            "apiVersion": "skyw.top/v1beta1", "kind": "Recipe",
            "metadata": {"name": f"fetch-{package}-{version}".replace(".", "-")},
            "spec": {
                "tasks": [{
                    "name": "fetch", "taskSelector": {"matchLabels": {"capability": "fetch-url"}},
                    "variables": {
                        "url": literal(pin["url"]), "sha256": literal(pin["sha256"]),
                        "version": literal(version),
                    },
                    "outputs": [{"name": "source"}],
                }],
                "outputs": {"artifacts": [{
                    "name": "source", "from": {"task": "fetch", "artifact": "source"},
                    "labels": {"name": literal(source_name), "version": literal(version),
                               "recipe": literal("fetch-v2")},
                }]},
            },
        })
        documents.append({
            "apiVersion": "skyw.top/v1beta1", "kind": "Recipe",
            "metadata": {"name": f"build-{package}-{version}".replace(".", "-")},
            "spec": {
                "inputs": {"artifacts": [{
                    "name": "source", "artifactSelector": {"matchLabels": {
                        "name": source_name, "version": version, "recipe": "fetch-v2",
                    }},
                }]},
                "tasks": [{
                    "name": "build", "taskSelector": {"matchLabels": {
                        "capability": "build-pinned-python-sdist",
                    }},
                    "variables": {"package": literal(package), "version": literal(version)},
                    "inputs": {"artifacts": [{"name": "source", "from": {"recipeInput": "source"}}]},
                    "outputs": [{"name": "wheels"}],
                }],
                "outputs": {"artifacts": [{
                    "name": "wheels", "from": {"task": "build", "artifact": "wheels"},
                    "labels": {"name": literal(wheel_name), "version": literal(version),
                               "python": literal("cp313"), "libc": literal("glibc")},
                }]},
            },
        })
    output = ROOT / "scientific/pure.yaml"
    output.write_text(yaml.dump_all(documents, Dumper=Dumper, sort_keys=False, width=110))
    print(f"wrote {len(documents)} documents to {output}")


if __name__ == "__main__":
    main()
