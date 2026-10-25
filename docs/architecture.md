# Architecture

Ferrogate is a Kubernetes operator written in Rust. One binary, `ferrogate`, runs the controller (and,
from Phase B, the admission webhook). The code is a Cargo workspace whose crates have strict
dependency rules.

```text
 ferrogate                 binary: CLI, logging setup, wiring
    │
    ├──► ferrogate-operator   controller: watches and reconciles policies
    │        │
    │        ▼
    │    ferrogate-crd        custom resource types (kube-derive), status
    │        │
    └──► ferrogate-webhook    admission HTTP handlers (axum)
             │
             ▼
         ferrogate-core       policy model + evaluation engine; no Kubernetes I/O
```

| Crate | Responsibility | May depend on |
| --- | --- | --- |
| `ferrogate-core` | Modes, severities, rules, evaluation, violations | serde, schemars only — no `kube` |
| `ferrogate-crd` | `ClusterGatePolicy` and its status | core, kube (types only) |
| `ferrogate-operator` | Controller loop, reconcile, error back-off | crd, kube runtime |
| `ferrogate-webhook` | HTTP routes, `AdmissionReview` handling (Phase B) | core, crd |
| `ferrogate` | Command line, logging, connecting to the cluster | all of the above |

Why this split: see [decision 0002](decisions/0002-workspace-layout.md).

## Commands

| Command | What it does |
| --- | --- |
| `ferrogate operator` | Connects using the in-cluster service account or the current kubeconfig and runs the controller until SIGTERM/Ctrl-C |
| `ferrogate crd` | Prints the CRD YAML. `config/crd/` holds the committed copy; CI fails if they differ |

Logging: `--log-format text|json` (or `FERROGATE_LOG_FORMAT`); level via `FERROGATE_LOG`
(for example `FERROGATE_LOG=debug` or `FERROGATE_LOG=ferrogate_operator=debug,info`).

## Custom resources

| Kind | Scope | Short name | Version |
| --- | --- | --- | --- |
| `ClusterGatePolicy` | Cluster | `cgp` | `v1alpha1` |

API group `ferrogate.stochasticquant.io` ([decision 0003](decisions/0003-api-group-and-versioning.md)).
A namespaced `GatePolicy` and the full rule model arrive in Phase B; see section 6 of the
[plan](ENTERPRISE_PLAN.md).

## Reconcile loop

The controller watches `ClusterGatePolicy` objects. Each reconcile currently logs the policy and
requeues after five minutes; failures retry after 30 seconds. Phase B adds validation, status
conditions, events, compilation to `ValidatingAdmissionPolicy`, and background scanning.

## TLS

The Kubernetes client uses rustls with the `ring` crypto provider; no OpenSSL. A test in the
`ferrogate` crate fails if no provider is compiled in, because that would only show up as a crash
when connecting to a cluster.

## Security

How each part is secured: [SECURITY_ARCHITECTURE.md](SECURITY_ARCHITECTURE.md).
