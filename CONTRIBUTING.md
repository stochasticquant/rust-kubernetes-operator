# Contributing to Ferrogate

Thank you for helping. This guide explains how to set up, make a change, and get it merged.

By taking part you agree to follow the [Code of Conduct](CODE_OF_CONDUCT.md). Security problems are
**not** reported as issues: see [SECURITY.md](SECURITY.md).

## Ways to help

- Report a bug or ask for a feature with the issue forms.
- Propose a new policy for the built-in packs ("Policy request" form).
- Pick an issue labelled `good first issue` or `help wanted`. Comment first so work is not duplicated.
- Improve the documentation.

For larger changes, open an issue to discuss the design before writing code. Decisions that shape the
project are recorded in [`docs/decisions/`](docs/decisions/).

## Development setup

You need:

- Rust 1.99 (pinned in `rust-toolchain.toml`; `rustup` installs it automatically). The minimum
  supported version for users building from source is 1.89.
- `cargo-deny` (`cargo install cargo-deny`) for dependency checks.
- For running against a cluster: `kubectl` and any Kubernetes 1.34+ cluster
  ([kind](https://kind.sigs.k8s.io/) works well).

```bash
cargo build --workspace
cargo test --workspace
```

### Repository layout

| Path | What it is |
| --- | --- |
| `crates/ferrogate-core` | Policy model and evaluation engine. No Kubernetes client, no I/O. |
| `crates/ferrogate-crd` | Custom resource definitions (`ClusterGatePolicy`). |
| `crates/ferrogate-operator` | Controller that reconciles policies. |
| `crates/ferrogate-webhook` | Admission webhook HTTP handlers. |
| `crates/ferrogate` | The `ferrogate` binary. |
| `config/crd` | Generated CRD YAML. Never edit by hand. |
| `config/samples` | Example policies. |

See [`docs/architecture.md`](docs/architecture.md) for which crate may depend on which.

### Trying it on a cluster

```bash
kubectl apply -f config/crd/
kubectl apply -f config/samples/require-team-label.yaml
cargo run -p ferrogate -- operator          # uses your current kubeconfig context
kubectl get clustergatepolicies             # short name: cgp
```

Clean up with `kubectl delete -f config/samples/ -f config/crd/`.

## Before you open a pull request

Run the same checks as CI:

```bash
cargo fmt --all
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo deny check
```

If you changed anything in `crates/ferrogate-crd`, regenerate the CRDs:

```bash
cargo run -p ferrogate -- crd > config/crd/clustergatepolicies.ferrogate.stochasticquant.io.yaml
```

## Coding rules

- No `unsafe` code (enforced by the compiler).
- No `unwrap()`, `expect()` or `panic!` outside tests: return errors instead (enforced by clippy).
  A panic in the webhook would fail admission requests across the cluster.
- Log with `tracing`, never `println!`. Never log the body of a Kubernetes object: it may contain
  secrets.
- Libraries use `thiserror` for errors; binaries use `anyhow`.
- Keep `ferrogate-core` free of Kubernetes I/O so it stays fast to test.
- Every new source file starts with `// SPDX-License-Identifier: Apache-2.0`.
- Add tests for new behaviour. Bug fixes come with a test that fails without the fix.

## Commits and pull requests

- Branch from `main`; name branches like `feat/require-labels`, `fix/webhook-timeout`, `docs/install`.
- Use [Conventional Commits](https://www.conventionalcommits.org/): `feat:`, `fix:`, `docs:`, `ci:`,
  `chore:`, `refactor:`, `test:`. Add `!` for breaking changes (`feat!:`). The changelog is generated
  from these.
- Sign off each commit to certify the
  [Developer Certificate of Origin](https://developercertificate.org/): `git commit -s`.
- Keep pull requests focused. Fill in the template, link the issue, and update docs and
  `CHANGELOG.md` for user-visible changes.
- Changes to the webhook, TLS, RBAC, Helm chart or CI workflows get an extra security review from a
  maintainer (see `.github/CODEOWNERS`).

## Licence

Ferrogate is licensed under [Apache-2.0](LICENSE). By contributing you agree that your contributions
are licensed under the same terms.
