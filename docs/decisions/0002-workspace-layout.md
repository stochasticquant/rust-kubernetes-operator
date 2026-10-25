# 0002 — Cargo workspace with a Kubernetes-free core

Status: accepted, 2026-10-24.

## Context

The code was one crate with every module side by side. The policy logic, the CRD types, the controller
and the webhook will grow at different speeds and have different risk profiles: the webhook handles
untrusted input on the request path of the cluster, while the engine should be pure logic.

## Decision

Split the code into a workspace under `crates/`:

- `ferrogate-core` — policy model and evaluation. **No Kubernetes client, no I/O.**
- `ferrogate-crd` — custom resource types.
- `ferrogate-operator` — controller.
- `ferrogate-webhook` — admission HTTP handlers.
- `ferrogate` — the single binary, with subcommands.

Shared versions, lints and metadata live in the root `Cargo.toml` (`[workspace.dependencies]`,
`[workspace.lints]`). A `kubectl-ferrogate` plugin crate is added in Phase B when it has real content.

## Why

- The engine can be tested exhaustively (unit, property and fuzz tests) without a cluster.
- The same engine is reused by the webhook, the background scanner and the CLI, so they cannot disagree.
- Dependency direction is visible in `Cargo.toml` files and reviewable.
- Workspace-wide lints (`unsafe_code = "forbid"`, no `unwrap`/`expect`/`panic`) apply to every crate.

## Alternatives considered

- **One crate with modules**: simpler, but nothing stops the engine from depending on the Kubernetes
  client, and lints and features apply to everything at once.
- **Separate binaries for operator and webhook**: more images to build, sign and scan. One binary with
  subcommands is easier to ship; each can still run as its own Deployment.
- **`cargo xtask` for CRD generation**: the `ferrogate crd` subcommand does the same job with less
  machinery, and lets users print CRDs from the released binary.

## Consequences

- New code must go into the crate that matches its responsibility; see `docs/architecture.md`.
- Crates are `publish = false` until the APIs settle.
