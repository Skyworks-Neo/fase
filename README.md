# Fase v1beta1 cluster manifests

This repository is the GitOps working tree for the v1beta1 deployment in the
`fase-beta` namespace. Its `gitops-beta` branch is pushed to the `fase` GitHub
repository and reconciled by the Flux objects in `bootstrap/flux.yaml`.

The source manifests come from Fase commit `a327a9dcbd359e8cba61cf0f9c8e5816fe8742a1`.
The controller and helper images are pinned to the digests built by that commit.

The cluster already has v1alpha1 data and a controller in `sep`. The three
shared CRDs serve both versions and use v1beta1 as the storage version. Their
v1beta1 storage schemas also accept legacy fields, so old objects retain their
data. Their v1alpha1 API schemas remain unchanged, so old clients do not see
new fields. This compatibility layer is temporary and should be removed only
after the old data and controller have been migrated.

The `fase-ghcr-pull`, `fase-s3-read`, and `fase-s3-write` Secrets are copied
from `sep` into `fase-beta` out of band; secret values are not committed here.
The shared S3 credentials reject a separate prefix, so beta artifacts use the
content-addressed `objects/` path in the existing `fase` bucket.

`crds/`, `runtime/`, and `smoke/` are separate Kustomize roots to give Flux a
dependency order. `smoke/` contains the example two task build Request and its
definitions. The shared CRDs are never pruned by Flux.

After the smoke Request succeeds, `kubectl apply -f tests/cache.yaml` verifies
that a second Request resolves the existing Claim without starting more Jobs.
`kubectl apply -f tests/rerun.yaml` asks for a forced rerun through the labeled
`hello-package` Recipe.
