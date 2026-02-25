# kube-guardian

A Rust-based Kubernetes governance operator that enforces security policies across clusters through custom resources, admission webhooks, and multi-cluster coordination.

## Overview

kube-guardian is a Kubernetes operator built in Rust that provides:

- **Policy-as-Code governance** — Define and enforce security policies as Kubernetes custom resources
- **Admission control** — Validate and reject non-compliant workloads before they run
- **Multi-cluster support** — Aggregate compliance across multiple Kubernetes clusters
- **Production observability** — Structured logging, Prometheus metrics, and health checks

## Architecture

```
kube-guardian/
├── src/
│   ├── main.rs              # Entry point and orchestration
│   ├── cli.rs               # CLI argument parsing (clap)
│   ├── config.rs            # Configuration management
│   ├── crd.rs               # CustomResourceDefinition (GuardianPolicy)
│   ├── controller.rs        # Kubernetes controller wiring
│   ├── reconcile.rs         # Reconciliation business logic
│   ├── webhook.rs           # Admission webhook HTTP server
│   ├── metrics.rs           # Prometheus metrics and health checks
│   ├── multi_cluster.rs     # Multi-cluster coordination
│   └── governance/
│       ├── mod.rs           # Governance module declarations
│       ├── policies.rs      # Policy data structures
│       └── traits.rs        # PolicyEvaluator trait and implementations
├── tests/
│   └── integration.rs       # Integration tests
├── docs/                    # Project documentation and guides
├── Dockerfile               # Multi-stage container build
└── helm/                    # Helm chart for deployment
```

## Features

### Custom Resource Definition

kube-guardian defines a `GuardianPolicy` CRD under the `guardian.io` API group:

```yaml
apiVersion: guardian.io/v1
kind: GuardianPolicy
metadata:
  name: require-labels
spec:
  severity: High
```

### Admission Webhook

A validating admission webhook intercepts resource creation and modification requests, evaluating them against active `GuardianPolicy` resources. Non-compliant workloads are rejected with descriptive error messages.

### Controller & Reconciliation

The operator follows the standard Kubernetes controller pattern:

- Watches `GuardianPolicy` resources for changes
- Reconciles desired state with actual cluster state
- Handles finalizers for clean resource lifecycle management
- Requeues with configurable backoff on transient failures

### Observability

- **Structured logging** via `tracing` with span context
- **Prometheus metrics** exposed at `/metrics` (reconciliation counts, latencies)
- **Health endpoints** for readiness and liveness probes

### Multi-Cluster

Supports loading multiple kubeconfigs to coordinate governance policies across clusters, aggregating compliance results into a unified view.

## Prerequisites

- [Rust](https://www.rust-lang.org/tools/install) 1.75+
- A Kubernetes cluster (for phases 3+)
- [kubectl](https://kubernetes.io/docs/tasks/tools/) configured with cluster access

## Getting Started

### Build

```sh
cargo build --release
```

### Run (CLI mode)

```sh
cargo run -- --config path/to/config.json
```

### Run Tests

```sh
cargo test
```

### Deploy to Kubernetes

```sh
docker build -t kube-guardian:latest .
helm install kube-guardian ./helm
```

## Development Roadmap

The project is implemented in phases, each on its own feature branch:

| Phase | Branch | Description |
|-------|--------|-------------|
| 0 | `feature/phase-0-foundations` | Rust fundamentals — structs, enums, ownership, error handling |
| 1 | `feature/phase-1-cli-design` | CLI design with clap, traits, module architecture |
| 2 | `feature/phase-2-async` | Async runtime with Tokio, concurrent task scheduling |
| 3 | `feature/phase-3-crd` | Serde serialization, CRD modeling with kube-rs |
| 4 | `feature/phase-4-controller` | Kubernetes controller pattern, reconcile loop, finalizers |
| 5 | `feature/phase-5-webhook` | Admission webhook with axum, TLS configuration |
| 6 | `feature/phase-6-observability` | Tracing, Prometheus metrics, health checks |
| 7 | `feature/phase-7-multicluster` | Multi-cluster client management and state aggregation |
| 8 | `feature/phase-8-enterprise` | Leader election, RBAC hardening, Helm chart, Docker |

## Key Dependencies

| Crate | Purpose |
|-------|---------|
| `clap` | CLI argument parsing |
| `tokio` | Async runtime |
| `kube` | Kubernetes client and CRD derive macros |
| `kube-runtime` | Controller runtime and reconciliation |
| `axum` | HTTP server for webhooks |
| `serde` | Serialization/deserialization |
| `schemars` | JSON Schema generation for CRDs |
| `tracing` | Structured logging |
| `prometheus` | Metrics collection and exposition |

## License

This project is for educational and professional development purposes.