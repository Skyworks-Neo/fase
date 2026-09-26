# Fase v1beta1 cluster manifests

This repository is the GitOps working tree for the v1beta1 deployment in the
`fase-beta` namespace. Its `gitops-beta` branch is pushed to the `fase` GitHub
repository and reconciled by the Flux objects in `bootstrap/flux.yaml`.

The source manifests come from Fase commit `a327a9dcbd359e8cba61cf0f9c8e5816fe8742a1`.
The controller and helper images are pinned to the digests built by that commit.

The cluster already has v1alpha1 data and a controller in `sep`. The three
shared CRDs retain v1alpha1 as the storage version and serve v1beta1 too. The
v1alpha1 schema accepts the v1beta1 fields so beta objects survive storage
conversion. This compatibility layer is temporary and should be removed only
after the old data and controller have been migrated.

The `fase-ghcr-pull`, `fase-s3-read`, and `fase-s3-write` Secrets are copied
from `sep` into `fase-beta` out of band; secret values are not committed here.
S3 objects use the `fase-beta/` prefix within the existing `fase` bucket.

`crds/`, `runtime/`, and `smoke/` are separate Kustomize roots to give Flux a
dependency order. `smoke/` contains the example two task build Request and its
definitions. The shared CRDs are never pruned by Flux.
