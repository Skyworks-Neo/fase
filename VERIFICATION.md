# Cluster verification — 2026-09-26

Cluster context: `kubernetes-admin@tunet`. Source revision:
`a327a9dcbd359e8cba61cf0f9c8e5816fe8742a1`. Target namespace: `fase-beta`.

- Flux `fase-beta-bootstrap`, `fase-beta-crds`, `fase-beta-runtime`, and
  `fase-beta-smoke` reconciled successfully from `gitops-beta`.
- `hello-smoke-v2` succeeded with two completed Jobs. It produced two complete
  Artifacts and two ArtifactClaims; the package task read the source Artifact.
- `tests/cache.yaml` succeeded with the same package Artifact and Claim and
  started no new Job.
- `tests/rerun.yaml` retained `spec.rerun: 1`, succeeded, and started a new
  package Job. It resolved to the same content-addressed Artifact and Claim.
- A server-side dry run of `tests/nightly.yaml` accepted the
  RequestGenerator schema. No scheduled generator was installed.
- A server-side dry run by the cluster administrator to create an
  ArtifactClaim was denied by `fase-artifactclaim-integrity`.
- The v1alpha1 controller in `sep` remained Ready. All 110 legacy Requests and
  114 legacy Artifacts were readable; sampled controller logs had no errors.

The shared CRDs serve both versions and store new objects as v1beta1. Their
`status.storedVersions` includes both v1alpha1 and v1beta1 because legacy
objects remain. Do not remove v1alpha1 until the old namespace is migrated.

The three Secrets copied from `sep` are managed outside GitOps. Copy updated
values into `fase-beta` if the source Secrets rotate.
