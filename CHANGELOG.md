# Changelog

All notable changes to Ferrogate are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/). Before 1.0, minor versions may contain breaking changes.

## [Unreleased]

### Added

- Project name **Ferrogate** and API group `ferrogate.stochasticquant.io`.
- `ClusterGatePolicy` custom resource (`v1alpha1`, short name `cgp`) with `mode` (`Audit` by
  default, `Warn`, `Enforce`), `severity`, `match`, `exclude` and `rules`. The API server rejects
  rules that do not set exactly one check.
- Rules: required labels and annotations, allowed registries, no `latest` tag, resource limits and
  requests, and the Pod Security Standards baseline and restricted checks (privileged, host
  namespaces, `hostPath`, host ports, capabilities, privilege escalation, non-root, drop `ALL`,
  seccomp). Violations include the field path.
- Matching by kind and API group, namespaces, namespace selector and object selector; exclusions by
  namespace, labels, users and groups.
- Admission webhook (`ferrogate webhook`): `Enforce` denies, `Warn` returns warnings, `Audit` adds
  API server audit annotations. HTTPS only, certificate reloaded on change.
- `ferrogate operator`: `Ready` and `Scanned` conditions, background scans of existing resources
  with violation counts and sample results in `status`, Kubernetes Events, exponential back-off.
- `/healthz`, `/readyz` and Prometheus `/metrics` on port 8080 for both components.
- `ferrogate check`: evaluate policies against local YAML files without a cluster (text or JSON).
- Policy packs in `policies/`: `pss-baseline`, `pss-restricted`, `best-practices`, with fixtures
  tested in CI.
- Container image (distroless, non-root) and Kustomize install in `config/deploy/` with
  least-privilege RBAC, PDB, NetworkPolicy and a cert-manager certificate.
- End-to-end test script in `tests/e2e/`.
- `ferrogate crd` prints the custom resource definitions; the generated YAML is in `config/crd/`.
- Structured logging with text or JSON output (`--log-format`, `FERROGATE_LOG` for the level).
- Apache-2.0 licence, security policy, contributing guide, code of conduct and governance.
- Security architecture, threat model and decision records in `docs/`.

### Changed

- Split into a Cargo workspace: `ferrogate-core`, `ferrogate-crd`, `ferrogate-operator`,
  `ferrogate-webhook` and the `ferrogate` binary.
- Upgraded to kube 4.2, k8s-openapi 0.28 (Kubernetes 1.34 API) and schemars 1.2. Minimum Rust 1.89.

### Security

- No `unsafe` code, and no `unwrap`/`expect`/`panic` outside tests, enforced by lints.
- CI checks dependencies for known vulnerabilities and licence problems (`cargo deny`), runs CodeQL,
  and reviews dependency changes in pull requests. All GitHub Actions are pinned by commit.
- Updated rustls and rustls-webpki to fix RUSTSEC-2026-0285, -0346, -0348, -0349, -0049, -0098,
  -0099, -0104 and -0382 present in the previous lockfile.
- Webhook request size limit, handler timeout and concurrency limit; object contents are never
  logged. Fail-open (`failurePolicy: Ignore`) by default; 503 until caches are synced.

[Unreleased]: https://github.com/stochasticquant/rust-kubernetes-operator/commits/main
