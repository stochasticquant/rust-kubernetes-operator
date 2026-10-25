# 0003 — API group `ferrogate.stochasticquant.io`, starting at `v1alpha1`

Status: accepted, 2026-10-24.

## Context

The first CRD used the group `guardian.io` at version `v1`. That domain is not ours, so it could clash
with another project, and `v1` promises a stability the API does not have yet. Once users install a
CRD, changing its group means a migration for every one of them, so this must be settled before the
first release.

## Decision

- API group: **`ferrogate.stochasticquant.io`**, a subdomain under the project owner's name.
- First version: **`v1alpha1`**. Promotion path: `v1alpha1` → `v1beta1` → `v1`, following the
  [Kubernetes API versioning conventions](https://kubernetes.io/docs/reference/using-api/#api-versioning).
- First kind: **`ClusterGatePolicy`** (cluster-scoped, short name `cgp`). A namespaced `GatePolicy`
  follows in Phase B.
- Safe defaults in the schema: `mode` defaults to `Audit`.

## Why

- A group under a name we control cannot collide with other projects.
- `alpha` tells users that fields may still change; we can fix mistakes before committing to `v1`.
- Kind names avoid `ClusterPolicy`/`Policy`, which Kyverno uses; clusters often run several policy
  engines and `kubectl get clusterpolicy` would be ambiguous.

## Alternatives considered

- `ferrogate.io` / `ferrogate.dev`: nicer, but we do not own those domains today. If the project later
  owns one, a new group can be introduced alongside a conversion path before `v1`.
- `policy.ferrogate...` subgroup: unnecessary while the project has a single API area.

## Consequences

- Moving to `v1beta1` and `v1` needs conversion handling (webhook or storage-version migration),
  planned for after 1.0 in the [enterprise plan](../ENTERPRISE_PLAN.md).
- Generated CRDs live in `config/crd/`; CI checks they match the Rust types.
