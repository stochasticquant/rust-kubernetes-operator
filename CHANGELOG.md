# Changelog

All notable changes to Ferrogate are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/). Before 1.0, minor versions may contain breaking changes.

## [Unreleased]

### Added

- Project name **Ferrogate** and API group `ferrogate.stochasticquant.io`.
- `ClusterGatePolicy` custom resource (`v1alpha1`, short name `cgp`) with `mode` (`Audit` by
  default, `Warn`, `Enforce`), `severity` and `requireLabels`.
- `ferrogate operator` runs the controller against the current cluster and shuts down cleanly on
  SIGTERM or Ctrl-C. It logs each policy it sees; enforcement arrives in a later release.
- `ferrogate crd` prints the custom resource definitions; the generated YAML is in `config/crd/`.
- Structured logging with text or JSON output (`--log-format`, `FERROGATE_LOG` for the level).
- First rule in the engine: required labels.
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

[Unreleased]: https://github.com/stochasticquant/rust-kubernetes-operator/commits/main
