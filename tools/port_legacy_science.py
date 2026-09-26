#!/usr/bin/env python3
"""Port the source-built musl scientific stack into v1beta1 manifests.

Reads the historical manifests without modifying that repository. Archived
outputs remain file artifacts; consumers explicitly unpack them in /workspace.
"""

from pathlib import Path
import copy
import re
import sys
import yaml


GROUPS = (
    "sources", "tools", "musl", "binutils", "gcc", "llvm", "python",
    "openblas", "numpy", "scipy", "mldtypes",
)
EXCLUDE_TASKS = {"fetch-gitlab-archive", "toolcheck", "toolcheck-plan"}
EXCLUDE_RECIPES = {
    "fetch-fase-source", "toolcheck", "fetch-jax-0-11-2",
    "fetch-jaxlib-manylinux-0-11-2", "fetch-xla-stacktrace-header-91888df6",
}


def convert_ref(value):
    if isinstance(value, list):
        return [convert_ref(item) for item in value]
    if isinstance(value, dict):
        return {
            {"planInput": "recipeInput", "step": "task"}.get(key, key): convert_ref(item)
            for key, item in value.items()
        }
    return value


def convert_task(document):
    spec = document["spec"]
    inputs = spec.get("inputs", {}).get("artifacts", [])
    archived = []
    for port in inputs:
        old_path = port["path"]
        if port.pop("format") == "archive":
            archived.append((old_path, old_path + ".tar.zst"))
            port["path"] = old_path + ".tar.zst"
        port["kind"] = "file"
    for port in spec.get("outputs", {}).get("artifacts", []):
        port.pop("format")
        port["kind"] = "file"

    if archived:
        prelude = [
            "set -eu",
            "if ! command -v zstd >/dev/null 2>&1; then",
            "  apt-get update -qq >/dev/null",
            "  apt-get install -y -qq --no-install-recommends zstd >/dev/null",
            "fi",
            'mkdir -p /workspace/inputs',
        ]
        for old_path, new_path in archived:
            prelude.extend([
                f'mkdir -p "/workspace/inputs/{old_path}"',
                f'zstd -dc "/in/{new_path}" | tar -xf - -C "/workspace/inputs/{old_path}"',
            ])
        for port in inputs:
            if port["path"].endswith(".tar.zst") and any(
                port["path"] == new for _, new in archived
            ):
                continue
            prelude.append(
                f'ln -s "/in/{port["path"]}" "/workspace/inputs/{port["path"]}"'
            )
        prelude.append('export FASE_INPUT_ROOT=/workspace/inputs')
        spec["script"] = "\n".join(prelude) + "\n" + spec["script"]

    spec["script"] = re.sub(r"(?<!/workspace)/build(?![A-Za-z0-9_-])", "/workspace/build", spec["script"])
    resources = spec.get("resources", {})
    limits = resources.get("limits", {})
    if int(str(limits.get("cpu", "1"))) > 2 or archived:
        spec["workspace"] = {
            "taskSizeLimit": "300Gi",
            "inputSizeLimit": "32Gi",
            "outputSizeLimit": "32Gi",
            "transferSizeLimit": "32Gi",
        }
        spec["nodeSelector"] = {"kubernetes.io/hostname": "node1"}
        resources["requests"] = {
            "cpu": "32", "memory": "64Gi", "ephemeral-storage": "200Gi"
        }
        resources["limits"] = {
            "cpu": "32", "memory": "64Gi", "ephemeral-storage": "300Gi"
        }
        spec["resources"] = resources
    document["kind"] = "Task"
    return document


def convert_recipe(document):
    spec = document["spec"]
    spec["tasks"] = spec.pop("steps")
    for task in spec["tasks"]:
        task["taskSelector"] = task.pop("stepSelector")
    document["spec"] = convert_ref(spec)
    document["kind"] = "Recipe"
    return document


def main():
    source = Path(sys.argv[1]) / "manifests/base"
    output = Path(sys.argv[2])
    documents = []
    for group in GROUPS:
        for path in sorted((source / group).glob("*.yaml")):
            for item in yaml.safe_load_all(path.read_text()):
                if not isinstance(item, dict):
                    continue
                name = item.get("metadata", {}).get("name")
                kind = item.get("kind")
                if kind == "Step" and name not in EXCLUDE_TASKS:
                    documents.append(convert_task(copy.deepcopy(item)))
                elif kind == "Plan" and name not in EXCLUDE_RECIPES:
                    documents.append(convert_recipe(copy.deepcopy(item)))
    for item in documents:
        item["apiVersion"] = "skyw.top/v1beta1"
        item["metadata"].pop("namespace", None)
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(yaml.safe_dump_all(documents, sort_keys=False, width=110))
    print(f"wrote {len(documents)} Tasks and Recipes to {output}")


if __name__ == "__main__":
    main()
