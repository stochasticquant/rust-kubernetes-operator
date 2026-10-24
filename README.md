# Ferrogate

[![CI](https://github.com/stochasticquant/rust-kubernetes-operator/actions/workflows/ci.yml/badge.svg)](https://github.com/stochasticquant/rust-kubernetes-operator/actions/workflows/ci.yml)
[![CodeQL](https://github.com/stochasticquant/rust-kubernetes-operator/actions/workflows/codeql.yml/badge.svg)](https://github.com/stochasticquant/rust-kubernetes-operator/actions/workflows/codeql.yml)
[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

**A lightweight, Rust-native Kubernetes policy operator.** Ferrogate audits first, enforces safely,
and compiles policies to native Kubernetes admission where it can.

> **Status: early development (pre-0.1).** The project foundation is in place and the operator runs
> against a cluster, but it does not enforce anything yet. Do not rely on it to protect a cluster.
> See [What works today](#what-works-today) and the [Roadmap](#roadmap).

## Why Ferrogate

- **Small and fast.** One static Rust binary, no `unsafe` code, minimal container image.
- **Audit before enforce.** Policies start in `Audit` mode and report violations without blocking
  anything. Move to `Warn`, then `Enforce`, when you are ready.
- **Native admission first.** Planned: rules that Kubernetes can evaluate itself are compiled to
  `ValidatingAdmissionPolicy`, so there is no webhook in the request path for them.
- **Secure by design.** Least-privilege RBAC, fail-open by default, no telemetry, signed releases.
  See the [security architecture](docs/SECURITY_ARCHITECTURE.md).

## What works today

- `ClusterGatePolicy` custom resource (`ferrogate.stochasticquant.io/v1alpha1`, short name `cgp`)
  with `mode`, `severity` and `requireLabels`
- `ferrogate operator`: watches policies on the current cluster and logs each one; graceful shutdown
- `ferrogate crd`: prints the CRD YAML (also committed in [`config/crd/`](config/crd/))
- Policy engine with a first rule (required labels), fully unit-tested
- Text or JSON logs
- CI: formatting, lints (warnings are errors), tests, minimum Rust version, CRD drift check,
  dependency vulnerabilities and licences (`cargo deny`), CodeQL, dependency review

Not yet: admission webhook, enforcement, policy reports, Helm chart, container image, releases.

## Quick start (development)

Requirements: Rust 1.89+ and access to a Kubernetes 1.34+ cluster (for example
[kind](https://kind.sigs.k8s.io/)).

```bash
git clone https://github.com/stochasticquant/rust-kubernetes-operator ferrogate
cd ferrogate
cargo test --workspace

kubectl apply -f config/crd/
kubectl apply -f config/samples/require-team-label.yaml
cargo run -p ferrogate -- operator
```

In another terminal:

```console
$ kubectl get cgp
NAME                 MODE    SEVERITY   AGE
require-team-label   Audit   Medium     5s
```

Remove everything with `kubectl delete -f config/samples/ -f config/crd/`.

Example policy:

```yaml
apiVersion: ferrogate.stochasticquant.io/v1alpha1
kind: ClusterGatePolicy
metadata:
  name: require-team-label
spec:
  mode: Audit        # Audit (default) | Warn | Enforce
  severity: Medium   # Low | Medium | High | Critical
  requireLabels:
    - team
```

## Architecture

```text
ferrogate (binary) ──► ferrogate-operator ──► ferrogate-crd ──► ferrogate-core
                   └─► ferrogate-webhook  ──────────────────────► ferrogate-core
```

`ferrogate-core` holds the policy engine and has no Kubernetes I/O, so it is fast to test and can be
reused by the webhook and a future `kubectl` plugin. Details: [`docs/architecture.md`](docs/architecture.md).
Design decisions: [`docs/decisions/`](docs/decisions/).

## Roadmap

| Phase | Contents | Status |
| --- | --- | --- |
| A — Foundation | Licence, security baseline, workspace, CI, docs | in progress |
| B — Working product | Full CRD, controller status, ValidatingAdmissionPolicy compiler, webhook with TLS, policy reports, metrics, e2e tests | planned (`v0.1.0`) |
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
