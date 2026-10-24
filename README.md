# Ferrogate

[![CI](https://github.com/stochasticquant/rust-kubernetes-operator/actions/workflows/ci.yml/badge.svg)](https://github.com/stochasticquant/rust-kubernetes-operator/actions/workflows/ci.yml)
[![CodeQL](https://github.com/stochasticquant/rust-kubernetes-operator/actions/workflows/codeql.yml/badge.svg)](https://github.com/stochasticquant/rust-kubernetes-operator/actions/workflows/codeql.yml)
[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

**A lightweight, Rust-native Kubernetes policy operator.** Ferrogate audits first, enforces safely,
and compiles policies to native Kubernetes admission where it can.

> **Status: pre-release (heading to `v0.1.0`).** Policies, admission and background scans work and
> are tested end to end on a real cluster, but the API is `v1alpha1` and there is no tagged release
> or signed image yet. Evaluate it in `Audit` mode before enforcing anything.
> See [What works today](#what-works-today) and the [Roadmap](#roadmap).

## Why Ferrogate

- **Small and fast.** One Rust binary, no `unsafe` code, distroless non-root image.
- **Audit before enforce.** Policies start in `Audit` mode and report violations without blocking
  anything. Move to `Warn`, then `Enforce`, when the results look right.
- **One engine everywhere.** Admission, background scans and the offline `ferrogate check` command
  share the same evaluation code, so they always agree.
- **Native admission when you want it.** Mark a policy `enforcement: Native` and Ferrogate compiles
  it to a `ValidatingAdmissionPolicy`, so the API server enforces it with no webhook in the path.
- **Secure by design.** Least-privilege RBAC, HTTPS-only webhook, fail-open by default, no
  telemetry. See the [security architecture](docs/SECURITY_ARCHITECTURE.md).

## What works today

- **`ClusterGatePolicy`** (`ferrogate.stochasticquant.io/v1alpha1`, short name `cgp`) with
  `mode`, `severity`, `match` (kinds, namespaces, label selectors), `exclude` (namespaces, labels,
  users, groups) and `rules`. Invalid rules are rejected by the API server.
- **15 built-in checks:** required labels and annotations, allowed registries, no `latest` tag,
  resource limits and requests, and the Pod Security Standards baseline and restricted checks.
  Violations name the field, for example `spec.template.spec.containers[0].image`.
- **Native enforcement (opt-in):** `enforcement: Native` compiles a policy to a
  `ValidatingAdmissionPolicy` that the API server evaluates itself, with no webhook in the path.
  Verified against every pack fixture on a real API server.
- **Admission webhook:** `Enforce` denies with a clear message, `Warn` shows a warning in
  `kubectl`, `Audit` writes to the API server audit log. HTTPS with cert-manager, certificate
  reloaded without restarts, request size, time and concurrency limits.
- **Operator:** `Ready`/`Scanned` conditions, background scans of existing workloads with violation
  counts and sample results, Kubernetes Events, exponential back-off, leader election (2 replicas).
- **Policy packs:** [`pss-baseline`, `pss-restricted`, `best-practices`](policies/), each with
  tested fixtures.
- **`ferrogate check`:** test policies against local YAML in CI, without a cluster.
- **Operations:** Prometheus metrics, `/healthz` and `/readyz`, JSON logs, graceful shutdown.
- **Install:** [Helm chart](charts/ferrogate/) with a validated values schema and optional
  Prometheus `ServiceMonitor`, or Kustomize manifests. Least-privilege RBAC, PDBs, NetworkPolicy,
  hardened restricted-PSS pods.

Not yet: custom CEL rules, PolicyReports, namespaced policies, published images and signed releases.

## Quick start

Requirements: a Kubernetes 1.30+ cluster with [cert-manager](https://cert-manager.io), Docker, and
Rust 1.89+ to build from source.

```bash
git clone https://github.com/stochasticquant/rust-kubernetes-operator ferrogate
cd ferrogate
docker build -t ferrogate:dev .          # then load it into your cluster (kind load, ctr import, ...)
helm install ferrogate charts/ferrogate -n ferrogate-system --create-namespace \
  --set image.repository=docker.io/library/ferrogate --set image.tag=dev
kubectl apply -f policies/pss-restricted/policy.yaml
```

```console
$ kubectl get cgp
NAME             MODE    SEVERITY   READY   VIOLATIONS   AGE
pss-restricted   Audit   High       True    35           40s
```

Full instructions, modes and settings: [docs/install.md](docs/install.md).

Without a cluster:

```console
$ cargo run -p ferrogate -- check -p policies/pss-baseline policies/pss-baseline/fixtures
FAIL  Pod default/root (policies/pss-baseline/fixtures/fail/privileged--pod.yaml)
      pss-baseline/privileged [Audit High] container "shell" must not be privileged (spec.containers[0].securityContext.privileged)
...
```

Example policy:

```yaml
apiVersion: ferrogate.stochasticquant.io/v1alpha1
kind: ClusterGatePolicy
metadata:
  name: pinned-images
spec:
  mode: Enforce      # Audit (default) | Warn | Enforce
  severity: Medium   # Low | Medium | High | Critical
  match:
    resources:
      - apiGroups: [apps]
        kinds: [Deployment, StatefulSet]
  exclude:
    namespaces: [kube-system]
  rules:
    - name: no-latest
      disallowLatestTag: true
      message: "pin images to a version or digest"
```

## Architecture

```text
ferrogate (binary) ──► ferrogate-operator ──► ferrogate-crd ──► ferrogate-core
                   └─► ferrogate-webhook  ──────────────────────► ferrogate-core
```

`ferrogate-core` holds the policy engine and has no Kubernetes I/O, so it is fast to test and is
shared by the webhook, the background scanner and `ferrogate check`. Details: [`docs/architecture.md`](docs/architecture.md).
Design decisions: [`docs/decisions/`](docs/decisions/).

## Roadmap

| Phase | Contents | Status |
| --- | --- | --- |
| A — Foundation | Licence, security baseline, workspace, CI, docs | done |
| B — Working product | Full CRD, controller status, webhook with TLS, native enforcement (VAP), metrics, policy packs, HA, e2e tests (done); CEL rules, policy reports (next) | in progress (`v0.1.0`) |
| C — Enterprise readiness | Hardened image, Helm chart, signed releases, SBOM, provenance, HA | planned (`v0.5.0`) |
| D — Community | Docs site, policy packs (PSS, CIS, NSA), benchmarks | planned |
| E — Advanced | Multi-cluster compliance, mutation, exceptions, WASM plugins | after 1.0 |

The full plan: [`docs/ENTERPRISE_PLAN.md`](docs/ENTERPRISE_PLAN.md). Changes per version:
[`CHANGELOG.md`](CHANGELOG.md).

## Security

Ferrogate sits in the path of changes to your cluster, so security comes first. Read the
[security architecture](docs/SECURITY_ARCHITECTURE.md) and [threat model](docs/security/threat-model.md).
Report vulnerabilities privately as described in [SECURITY.md](SECURITY.md).

## Contributing

Contributions are welcome. Start with [CONTRIBUTING.md](CONTRIBUTING.md) and the
[Code of Conduct](CODE_OF_CONDUCT.md). How the project is run: [GOVERNANCE.md](GOVERNANCE.md).

## Licence

[Apache License 2.0](LICENSE).
