# 0005 — Native enforcement through ValidatingAdmissionPolicy

Status: accepted, 2026-10-25.

## Context

Kubernetes 1.30 made `ValidatingAdmissionPolicy` (VAP) generally available: CEL rules evaluated
inside the API server. A rule enforced this way needs no webhook call, so it adds no network hop,
no latency and no availability risk. The plan's goal is "native admission first".

The risk is divergence: if the generated CEL and the engine disagree, admission and background
scans report different results, which is worse than either alone.

## Decision

- A policy opts in with `spec.enforcement: Native` (default `Webhook`). Nothing changes for
  existing policies.
- **All or nothing.** The operator compiles every rule of a native policy to CEL. If one rule or
  kind cannot be expressed faithfully, the policy is `Ready=False, reason=NativeUnsupported` with
  the reason, instead of splitting it silently between VAP and webhook. Today that excludes
  `allowedRegistries` (image-name normalization in CEL is long and risks the CEL cost budget) and
  pod checks on kinds other than Pods, ReplicationControllers, Deployments, StatefulSets,
  DaemonSets, ReplicaSets, Jobs and CronJobs.
- The operator applies `ValidatingAdmissionPolicy` and `ValidatingAdmissionPolicyBinding` named
  `ferrogate-<policy>` with server-side apply, owned by the `ClusterGatePolicy`, so Kubernetes
  garbage-collects them with it. Switching back to `Webhook`, or an invalid policy, deletes them.
- The webhook skips native policies. Background scans still use the engine.
- Modes map to validation actions: `Enforce` → `Deny`, `Warn` → `Warn`, `Audit` → `Audit`.
  `Enforce` policies use `failurePolicy: Fail` (an evaluation error denies: there is no network
  hop that could fail, so an error means a malformed object); `Warn` and `Audit` use `Ignore` so
  they can never block.
- Match semantics are reproduced with `matchConditions` (namespace lists, label selectors on the
  new object, excluded users and groups), not with `objectSelector`, whose old-or-new-object
  semantics differ from the engine.
- `ferrogate native` prints the generated objects for review or GitOps.

## Verification

`tests/e2e/native.sh` applies each policy pack as a native `Enforce` policy and server-side dry-runs
every pack fixture against the real API server. Each pass fixture must be admitted and each fail
fixture denied by the expected rule. It also checks that the API server type-checks the generated
policies without warnings. It runs in CI on kind and was run on the k3s test cluster (23/23).

## Consequences

- Native denial messages are per rule ("containers must not be privileged") and do not name the
  container or field path; webhook messages do.
- Adding a rule type means adding its CEL translation and fixtures, or marking it unsupported.
- The operator needs RBAC to manage VAP objects (threat T-16).
