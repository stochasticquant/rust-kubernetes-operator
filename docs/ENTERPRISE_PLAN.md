# Enterprise and Open-Source Plan

This plan takes the project from a learning scaffold to a professional, open-source Kubernetes policy
operator that companies can adopt. It borrows the practices that work well in
[WorriorVex](https://github.com/cloudworrior-labs/worriorvex): an honest README, tag-driven releases,
layered tests, written architecture and decision records, and a user-facing changelog.

Status: Phase A in progress, 2026-10-24. Section 1 describes the project as it was before Phase A.

---

## 1. Where the project is today

| Area | Current state | Problem |
| --- | --- | --- |
| Code | ~100 lines. `main.rs` prints the config path and exits. Controller and webhook are never started (`#[allow(dead_code)]`). The webhook handler returns nothing. `GatePolicySpec` has one field, `severity: String`. | Nothing runs against a cluster yet. |
| README | Describes `config.rs`, `metrics.rs`, `multi_cluster.rs`, `tests/`, a Dockerfile, a Helm chart and Prometheus metrics. | None of these exist. Visitors lose trust when the README overstates the code. |
| License | None. README says "for educational and professional development purposes". | Companies cannot legally adopt or contribute. |
| Toolchain | `edition = "2024"` (needs Rust 1.85+) while the README says 1.75+. `kube 0.98`, `schemars 0.8`. | Inconsistent and out of date. |
| Repository | No description, topics, community files or templates. `docs/` holds learning guides. | Looks like a personal exercise, not a product. |
| Identity | API group `guardian.io` uses a domain we do not own. The name "kube-guardian" may clash with an existing open-source Kubernetes project. | Risk of confusion or a forced rename later. |
| CI | fmt, clippy, test, build on Ubuntu. | Good start; no e2e, security, image or release pipeline. |

## 2. Security first

The operator sits in the path of every create and update request to the Kubernetes API, so companies
will ask "how secure is it?" before anything else. Security is therefore designed before the features,
not added at the end. The full design is in [`SECURITY_ARCHITECTURE.md`](SECURITY_ARCHITECTURE.md).

Summary of the security architecture:

| Layer | Control |
| --- | --- |
| Code | Rust, `unsafe_code = "forbid"`, no `unwrap()` in request paths, fuzzed input parsing |
| Admission path | Native `ValidatingAdmissionPolicy` where possible; webhook over TLS 1.3 (`rustls`), size limits, timeouts, CEL cost limits |
| Failure behaviour | `failurePolicy: Ignore` and `Audit` mode by default; system namespaces excluded |
| Access | Least-privilege RBAC per component, no `Secrets` access, no wildcards, every permission justified in `docs/rbac.md` |
| Runtime | Distroless, non-root, read-only filesystem, all capabilities dropped, seccomp, NetworkPolicy |
| Data | No telemetry, no outbound calls, object bodies never logged, structured audit log of every decision |
| Supply chain | `cargo audit`/`cargo deny`, image scanning, cosign signatures, SBOM, SLSA provenance, OpenSSF Scorecard |
| Process | Threat model (STRIDE) per release, security review on sensitive PRs, private disclosure via `SECURITY.md`, independent audit before 1.0 |

Security work is spread across the phases below, but the baseline lands in Phase A, before any
feature code.

## 3. Product vision

**A lightweight, Rust-native Kubernetes governance operator that audits first, enforces safely, and
compiles policies to native Kubernetes admission where it can.**

### Positioning

The space already has Kyverno, OPA Gatekeeper, Kubewarden and the built-in
`ValidatingAdmissionPolicy` (CEL). To be worth adopting, the project needs clear differentiators:

1. **Small and fast.** One static binary, distroless image, target < 20 MB RSS idle, sub-second startup.
   Publish reproducible benchmarks against Kyverno and Gatekeeper.
2. **Compile to native admission.** Translate a `GatePolicy` into `ValidatingAdmissionPolicy` +
   `ValidatingAdmissionPolicyBinding` so the API server enforces it with no webhook in the request
   path. The webhook remains only for rules CEL cannot express.
3. **Audit before enforce.** `mode: Audit | Warn | Enforce`. Background scans of existing workloads.
   Results written as standard `PolicyReport` / `ClusterPolicyReport` (wg-policy) so existing tools such
   as Policy Reporter display them.
4. **Policy packs included.** Pod Security Standards (baseline, restricted), CIS Benchmark controls,
   NSA/CISA hardening, each with test fixtures.
5. **Shift-left CLI.** `kubectl ferrogate test` evaluates local manifests in CI pipelines;
   `kubectl ferrogate explain` shows why a resource was denied.
6. **Multi-cluster compliance (post-1.0).** Aggregate reports across clusters into a compliance score
   per cluster and namespace, with history.

### Principles (stated in the README, like WorriorVex's promises)

- No telemetry. No outbound network calls except to the Kubernetes API.
- Fail safe and configurable: operators choose `failurePolicy`; the default never blocks the cluster
  when the operator is down.
- Least privilege: RBAC limited to what each component needs.
- The README only lists what works today. Plans go in the Roadmap.

## 4. Naming and identity decisions (do first)

- [x] Search GitHub and crates.io for the name: "kube-guardian" was taken, renamed to **Ferrogate**
      ([decision 0004](decisions/0004-project-name.md)). Artifact Hub and Docker Hub still to check.
- [ ] Rename the repository to the product name (GitHub redirects the old URL).
- [x] API group `ferrogate.stochasticquant.io`
      ([decision 0003](decisions/0003-api-group-and-versioning.md)).
- [x] Start CRDs at `v1alpha1`, promote to `v1beta1` then `v1` with conversion webhooks.
- [ ] Logo/icon in `assets/`, repo description, topics (`kubernetes`, `operator`, `rust`,
      `policy-as-code`, `admission-controller`, `kube-rs`, `security`, `compliance`).

## 5. Target architecture

### Repository layout

```text
.
├── Cargo.toml                  # workspace
├── crates/
│   ├── ferrogate-core/          # policy model + evaluation engine; no Kubernetes client dependency
│   ├── ferrogate-crd/           # CRD types (kube-derive), status, conditions
│   ├── ferrogate-operator/      # controller, reconcilers, VAP compiler, background scanner
│   ├── ferrogate-webhook/       # axum admission server, TLS
│   └── kubectl-ferrogate/       # CLI plugin: test, explain, report
├── charts/ferrogate/            # Helm chart
├── config/                     # generated CRD YAML, Kustomize bases, samples
├── policies/                   # policy packs (pss-baseline, pss-restricted, cis, nsa) + fixtures
├── tests/e2e/                  # kind-based end-to-end tests
├── docs/                       # user docs, architecture, decisions
├── Dockerfile
├── LICENSE, CONTRIBUTING.md, CODE_OF_CONDUCT.md, SECURITY.md, GOVERNANCE.md, CHANGELOG.md
└── .github/                    # workflows, issue/PR templates, CODEOWNERS, dependabot/renovate
```

### Dependency rules (enforced in review, documented in `docs/architecture.md`)

| Crate | Responsibility | May depend on |
| --- | --- | --- |
| `ferrogate-core` | Rule model, matching, evaluation, verdicts | serde only — no `kube` |
| `ferrogate-crd` | CRD structs, status, conditions | core |
| `ferrogate-operator` | Watches, reconciles, compiles VAP, scans, writes reports | core, crd, kube |
| `ferrogate-webhook` | AdmissionReview in/out, TLS | core, crd |
| `kubectl-ferrogate` | Offline evaluation of local YAML | core, crd |

Keeping the engine free of Kubernetes I/O makes it fast to unit test and reusable from the CLI.

### Runtime

- One binary with subcommands: `ferrogate operator`, `ferrogate webhook`, `ferrogate crd` (can run together or as separate
  Deployments).
- Leader election via a `coordination.k8s.io/Lease` for the controller; webhook replicas are stateless.
- Graceful shutdown on SIGTERM; drain in-flight admission requests.

## 6. CRD design (v1alpha1)

```yaml
apiVersion: ferrogate.stochasticquant.io/v1alpha1
kind: ClusterGatePolicy        # plus namespaced GatePolicy
metadata:
  name: require-team-label
spec:
  mode: Audit                      # Audit | Warn | Enforce
  severity: High                   # Low | Medium | High | Critical (enum, not String)
  match:
    resources:
      - apiGroups: ["apps"]
        kinds: ["Deployment", "StatefulSet"]
    namespaceSelector:
      matchExpressions:
        - { key: kubernetes.io/metadata.name, operator: NotIn, values: [kube-system] }
  exclude:
    subjects: [{ kind: Group, name: system:masters }]
  rules:
    - name: team-label
      requireLabels: ["team"]
    - name: no-privileged
      cel: "!object.spec.template.spec.containers.exists(c, c.securityContext.privileged == true)"
      message: "Privileged containers are not allowed"
status:
  observedGeneration: 3
  conditions:
    - type: Ready
      status: "True"
      reason: Compiled
  enforcement: NativeVAP           # NativeVAP | Webhook | Mixed
  violations: 12
```

Requirements:
- Typed enums, validation via `schemars` + `x-kubernetes-validations` (CEL) on the CRD itself.
- Status subresource, printer columns (`kubectl get cgp` shows mode, severity, violations, age).
- CRD YAML generated from Rust by `ferrogate crd`; CI fails if `config/crd/` is out of date.

## 7. Phased plan

Each phase ends with a tagged pre-release and an updated "What works today" section in the README.

### Phase A — Foundation and open-source hygiene (1–2 weeks) — in progress

Security baseline (first)
- [x] Publish `docs/SECURITY_ARCHITECTURE.md` and `docs/security/threat-model.md` (STRIDE, with IDs and status).
- [x] `SECURITY.md` with private reporting via GitHub Security Advisories and response targets.
- [x] Enable secret scanning, push protection, Dependabot alerts and security updates, private
      vulnerability reporting.
- [x] CodeQL (Rust and GitHub Actions) and dependency review on pull requests.
- [ ] Branch protection on `main`: pull requests and green CI required, no force-push (after the
      Phase A PR, so the new CI check names exist).
- [ ] Signed commits required (once maintainers have commit signing set up).
- [x] `cargo deny` in CI (advisories, licences, bans, sources) plus a weekly advisory scan; it covers
      what `cargo audit` checks. GitHub Actions pinned by commit SHA, least-privilege tokens.
- [x] Workspace lint `unsafe_code = "forbid"`; clippy denies `unwrap`/`expect`/`panic`/`print` outside tests.
- [x] CODEOWNERS for webhook, CRD, config, workflows and security docs.

Open-source hygiene
- [x] `LICENSE` — Apache-2.0, `NOTICE`, SPDX headers in every source file.
- [x] `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md` (Contributor Covenant 2.1), `GOVERNANCE.md`,
      `MAINTAINERS.md`.
- [x] `.github/`: issue forms (bug, feature, policy request), PR template with security section,
      `CODEOWNERS`, Dependabot for Cargo and Actions.
- [x] Rewrite `README.md`: tagline, badges, honest status line, "What works today", quick start,
      roadmap, links to docs.
- [x] Move learning guides to `docs/learning/`.
- [x] Cargo workspace (section 5) with `rust-version = 1.89` (MSRV, checked in CI) and
      `rust-toolchain.toml` (1.99).
- [x] Upgrade to kube 4.2, k8s-openapi 0.28, schemars 1.2. Lockfile refreshed, fixing nine RustSec
      advisories in rustls/rustls-webpki.
- [x] Error handling: `thiserror` in libraries, `anyhow` in the binary; `tracing` with text/JSON output.
- [x] Clippy `pedantic`, warnings as errors in CI.
- [x] `CHANGELOG.md` (Keep a Changelog).
- [x] Decision records 0001 licence, 0002 workspace layout, 0003 API group and versioning, 0004 name.
- [x] Rename to Ferrogate; API group `ferrogate.stochasticquant.io`; `ClusterGatePolicy` `v1alpha1`;
      `ferrogate crd` generates `config/crd/` and CI fails on drift.
- [x] `ferrogate operator` starts the controller, reconciles policies and shuts down gracefully;
      smoke-tested on the homelab k3s cluster (v1.36).
- [ ] Repository renamed to `ferrogate`; description and topics set.

**Exit:** clean repo, all community files, workspace builds, CI green.

### Phase B — Working core product (3–5 weeks) — in progress

Engine (`ferrogate-core`)
- [x] Rule types: required labels/annotations, image registry allow-list, disallow `:latest`,
      resource limits/requests required, Pod Security Standards baseline and restricted checks.
- [ ] CEL expressions (`cel` crate).
- [x] Match/exclude logic (kinds, namespaces, selectors, subjects).
- [x] Verdict type with rule name, message, severity, field path.

Controller (`ferrogate-operator`)
- [x] Reconcile policies: validate, compile, write `status.conditions`, emit Kubernetes Events.
- [ ] Finalizers to clean up generated VAP objects and reports.
- [ ] Compile CEL-expressible rules to `ValidatingAdmissionPolicy` + binding; mark the rest for webhook.
- [x] Background scanner: evaluate existing workloads on a schedule and on policy change; results
      in policy status.
- [ ] Write `PolicyReport` / `ClusterPolicyReport`.
- [x] Exponential backoff in `error_policy`, shared context (client, metrics, config), graceful shutdown.
- [ ] Leader election (the operator runs as a single replica until then).

Webhook (`ferrogate-webhook`)
- [x] Real `AdmissionReview` v1 handling: allow/deny with message, `warnings` for `Warn` mode,
      `auditAnnotations` for `Audit` mode.
- [x] TLS via cert-manager (CA chain; serving certificate reloaded without restart; rotation tested
      under load with no fail-open window).
- [ ] Built-in self-signed CA with rotation and `caBundle` patching (for clusters without cert-manager).
- [x] Timeouts, request size limits, concurrency limit, `failurePolicy: Ignore` by default.

Observability
- [x] Prometheus `/metrics`: reconcile count/duration/errors, admission requests by verdict and
      latency histogram, violations by policy/severity.
- [x] `/healthz`, `/readyz` (ready only after caches sync and certificates load).
- [x] JSON logs.
- [ ] OpenTelemetry tracing (OTLP exporter, off by default).

Testing
- [ ] Unit tests in `ferrogate-core` (target > 80% line coverage, measured with `cargo llvm-cov`).
- [x] Golden-file tests: `policies/<pack>/fixtures/{pass,fail}/*.yaml` with expected verdicts.
- [ ] Controller tests with a mocked API server (`tower-test`).
- [ ] End-to-end tests on **kind** in CI, and against a real multi-node cluster (homelab k3s) before
      each release: install chart, apply policies, assert allowed/denied
      resources and generated reports. Test against the three most recent Kubernetes minor versions.

**Exit:** `v0.1.0` — install on kind in five minutes, audit and enforce a PSS-restricted pack.

### Phase C — Enterprise readiness (2–3 weeks)

Packaging
- [ ] Multi-stage `Dockerfile` → distroless/static, non-root UID, read-only root filesystem,
      multi-arch (amd64, arm64) via `docker buildx`.
- [ ] Helm chart in `charts/`: values schema (`values.schema.json`), PodDisruptionBudget,
      anti-affinity, resource requests/limits, NetworkPolicy, ServiceMonitor, optional cert-manager.
      Published as an OCI chart to GHCR. Chart tests with `helm unittest` and `ct lint`.
- [ ] Kustomize bases under `config/` for non-Helm users.

Security and supply chain
- [ ] Least-privilege `ClusterRole` per component; document each permission in `docs/rbac.md`.
- [ ] `cargo deny` (licenses, advisories, bans), `cargo audit`, Trivy/Grype image scanning.
- [ ] Sign images and charts with cosign (keyless, GitHub OIDC); attach SBOM (syft) and SLSA
      provenance (`slsa-github-generator`).
- [ ] Renovate or Dependabot for Cargo, GitHub Actions and the Dockerfile; pin actions by SHA.
- [ ] OpenSSF Scorecard workflow and badge; aim for the OpenSSF Best Practices badge.
- [ ] Threat model in `docs/security/threat-model.md`.

Releases (tag-driven, like WorriorVex)
- [ ] Pushing `vX.Y.Z` runs: tests → binaries (linux/macOS/windows, amd64/arm64) → image → chart →
      `kubectl-ferrogate` → GitHub release with checksums, SBOM and signatures.
- [ ] `release-plz` (or `cargo-dist`) generates the changelog from Conventional Commits and bumps versions.
- [ ] Distribute the CLI through Krew and a Homebrew tap.
- [ ] Version shown in `ferrogate --version` and logs comes from the tag.
- [ ] `docs/release.md` describing the process and what is built.

Operations
- [ ] `docs/operations/`: upgrade guide, HA setup, sizing, failure modes, runbooks for common alerts.
- [ ] Example Grafana dashboard and PrometheusRule alerts in the chart.
- [ ] Support policy: last two minor versions receive fixes; Kubernetes version support matrix.

**Exit:** `v0.5.0` — signed, scanned, HA-capable release with documented operations.

### Phase D — Community and growth (ongoing)

- [ ] Documentation site (mdBook or Docusaurus on GitHub Pages): quickstart, concepts, policy
      reference, policy pack catalog, CLI reference, operations, FAQ, comparison with alternatives.
- [ ] Publish benchmarks (memory, admission latency p50/p99, startup) with reproducible scripts.
- [ ] List on Artifact Hub and OperatorHub.io (OLM bundle).
- [ ] Public roadmap via GitHub Projects; monthly release cadence; `good first issue` backlog.
- [ ] Blog post / KubeCon lightning talk; post in the kube-rs community.
- [ ] Apply to the CNCF Sandbox once there are external contributors and adopters (`ADOPTERS.md`).

### Phase E — Advanced features (post-1.0)

- [ ] Multi-cluster: hub component collects `PolicyReport`s from member clusters; compliance score
      per cluster/namespace with history; optional read-only web UI.
- [ ] Mutation policies (add default labels, set `runAsNonRoot`), generated via
      `MutatingAdmissionPolicy` where available.
- [ ] Exceptions CRD (`GatePolicyException`) with expiry dates and approvers.
- [ ] WASM policy plugins for custom logic.
- [ ] Image signature verification (cosign) as a rule type.
- [ ] CRD promotion `v1alpha1` → `v1beta1` → `v1` with conversion webhooks.

## 8. CI/CD overview

| Workflow | Trigger | Jobs |
| --- | --- | --- |
| `ci.yml` | push to `main`, PRs | fmt, clippy, unit tests (Linux/macOS/Windows), MSRV build, `cargo deny`, coverage, CRD drift check, docs build |
| `e2e.yml` | PRs, nightly | kind clusters on three Kubernetes versions; install chart; run e2e suite |
| `security.yml` | weekly, PRs | `cargo audit`, image scan, OpenSSF Scorecard, CodeQL |
| `release.yml` | tag `v*` | build, sign, SBOM, provenance, publish image/chart/binaries, GitHub release |
| `release-plz.yml` | push to `main` | open release PR with version bump and changelog |

## 9. Definition of "enterprise ready" (1.0 checklist)

- [ ] Apache-2.0 license, community files, security policy with response time commitment
- [ ] Independent third-party security audit completed and report published
- [ ] Stable `v1` CRDs with documented upgrade path
- [ ] HA: leader election, multiple webhook replicas, PDB, graceful shutdown
- [ ] Safe defaults: audit mode, `failurePolicy: Ignore`, system namespaces excluded
- [ ] Least-privilege RBAC, non-root read-only container, NetworkPolicy
- [ ] Signed artifacts with SBOM and provenance
- [ ] Metrics, dashboards, alerts, structured logs, tracing
- [ ] E2E tests across supported Kubernetes versions
- [ ] Complete docs: install, configure, operate, upgrade, troubleshoot
- [ ] Published benchmarks and comparison page
- [ ] At least one external adopter listed in `ADOPTERS.md`

## 10. Timeline (indicative, one maintainer part-time)

| Milestone | Target | Contents |
| --- | --- | --- |
| Phase A complete | +2 weeks | Hygiene, workspace, honest README |
| `v0.1.0` | +7 weeks | Working audit/enforce on kind, PSS pack |
| `v0.5.0` | +10 weeks | Signed releases, Helm chart, HA, operations docs |
| `v1.0.0` | +16 weeks | Stable CRDs, docs site, benchmarks, all packs |
| Multi-cluster | post-1.0 | Hub and compliance scoring |

## 11. Immediate next steps

Done in Phase A: security baseline, name and API group, licence and community files, workspace split,
dependency upgrades, `v1alpha1` CRD with drift check, controller wired into `ferrogate operator`.

Next:
1. Merge the Phase A pull request, then turn on branch protection with the new CI check names.
2. Rename the GitHub repository to `ferrogate`; set description and topics.
3. Phase B: full CRD model (section 6), status conditions and events, then the webhook with TLS,
   health endpoints and metrics.
4. Add end-to-end tests: kind in CI, plus a manual/periodic run on the homelab k3s cluster.
