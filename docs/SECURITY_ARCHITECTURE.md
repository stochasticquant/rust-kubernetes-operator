# Security Architecture

This document explains how the operator is designed to be secure, so that third parties can evaluate
it before installing it in their clusters. It is written before the code on purpose: every component
is built to the controls described here, and the document is kept in step with each release.

Status: design baseline, 2026-10-24. Items marked *(planned)* are not implemented yet.

---

## 1. Why security comes first

An admission controller sits in the path of every create and update request to the Kubernetes API.
If it is compromised, an attacker can:

- **Allow** what should be denied (privileged pods, untrusted images), bypassing the cluster's guardrails.
- **Deny** everything, taking down deployments across the cluster (denial of service).
- **Read** every object sent to it for admission, which can include sensitive configuration.

So the operator is treated as a security-critical component: small, least-privileged, auditable,
and safe when it fails.

## 2. Security principles

| Principle | What it means here |
| --- | --- |
| Least privilege | Each component gets only the RBAC verbs and resources it needs. No wildcard rules. No access to `Secrets`. |
| Fail safe | The default `failurePolicy` is `Ignore`, and rollouts start in `Audit` mode, so an outage of the operator never blocks the cluster unless the user explicitly chooses `Fail`. |
| Defense in depth | Native `ValidatingAdmissionPolicy` where possible (no extra network hop), webhook only where needed; container hardening; NetworkPolicy; signed artifacts. |
| Minimal attack surface | One static Rust binary, distroless image, no shell, no package manager, no outbound network except the Kubernetes API. |
| Memory safety | Written in Rust with `unsafe_code = "forbid"` across the workspace. |
| Secure by default | Safe defaults in the Helm chart; insecure options require explicit opt-in and log a warning. |
| No data leaves the cluster | No telemetry, no phone-home, no external policy downloads at runtime. |
| Transparency | Open source, documented threat model, SBOM and provenance for every release, public security advisories. |

## 3. System overview and trust boundaries

```text
            ┌──────────────────────────── Kubernetes cluster ────────────────────────────┐
            │                                                                            │
 user ──►   │  kube-apiserver ──(1) TLS──► guardian-webhook  (namespace: guardian-system)│
 kubectl    │       │  ▲                        │                                       │
            │       │  │                        │ reads policies (cache)                │
            │       │  └──(2) TLS + SA token ── guardian-operator ──► PolicyReports      │
            │       │                           │   └─► ValidatingAdmissionPolicy objects│
            │       └─(3) native VAP enforcement (in-process, no webhook hop)            │
            │                                                                            │
            └────────────────────────────────────────────────────────────────────────────┘
   TB-1: API server ↔ webhook         TB-2: operator ↔ API server
   TB-3: policy authors ↔ cluster     TB-4: release pipeline ↔ user's registry
   TB-5 (post-1.0): member clusters ↔ multi-cluster hub
```

| Boundary | Who is on each side | Main risks |
| --- | --- | --- |
| TB-1 API server → webhook | Kubernetes control plane → our HTTPS server | Spoofed admission requests, interception, DoS |
| TB-2 Operator → API server | Our ServiceAccount → Kubernetes API | Over-broad RBAC, token theft |
| TB-3 Policy authors → cluster | Cluster/namespace admins writing policies | Malicious or weakening policies, expensive CEL |
| TB-4 Release pipeline → user | Our CI → user's cluster | Tampered images, compromised dependencies |
| TB-5 Members → hub *(planned)* | Remote clusters → aggregation hub | Credential sprawl, forged reports |

## 4. Threat model (STRIDE)

| Threat | Example | Mitigations |
| --- | --- | --- |
| **Spoofing** | A pod in the cluster calls the webhook directly to probe policies | Webhook only accepts TLS; NetworkPolicy allows ingress only from the API server; optional mTLS client-cert check of the API server *(planned)*; the webhook never trusts request content for authorization decisions beyond evaluating it |
| **Tampering** | Altering a policy to weaken enforcement | Policies are Kubernetes objects protected by RBAC; cluster-scoped policies need cluster-admin-level RBAC; every change is in the Kubernetes audit log; `status.observedGeneration` and Events show what is enforced; optional policy signing *(planned)* |
| **Tampering** | Replaced container image or Helm chart | Images and charts signed with cosign (keyless, GitHub OIDC); SLSA provenance; pin by digest in the chart |
| **Repudiation** | "Who allowed this privileged pod?" | Structured audit log of every admission decision (policy, rule, user, resource, verdict); PolicyReports; Kubernetes Events |
| **Information disclosure** | Logging full objects including secrets in env vars | Logs record resource identity and verdict only, never object bodies; operator has no RBAC to read `Secrets`; webhook rules exclude `Secret` and `ConfigMap` by default |
| **Denial of service** | Expensive CEL or huge objects slowing admission | CEL cost limits; request body size limit; per-request timeout (default 5 s, well below the API server's 10 s); concurrency limit; `failurePolicy: Ignore` default; HPA + PDB + multiple replicas |
| **Denial of service** | Webhook outage blocks system components | Default exclusion of `kube-system`, `guardian-system` and other system namespaces; `namespaceSelector` and `objectSelector` on the webhook configuration |
| **Elevation of privilege** | Compromised operator pod used to escalate | Non-root, read-only root filesystem, all capabilities dropped, `seccompProfile: RuntimeDefault`, no privilege escalation; RBAC without `escalate`/`bind`/`impersonate`; ServiceAccount token auto-mount only where needed |

The full, maintained threat model lives in `docs/security/threat-model.md` *(planned)* and is reviewed
on every minor release.

## 5. Component security

### 5.1 Admission webhook

- HTTPS only (TLS 1.2+, TLS 1.3 preferred) using `rustls`; no OpenSSL dependency.
- Certificates from cert-manager, or a built-in self-signed CA with automatic rotation before expiry
  and `caBundle` patching. Private keys stay in memory and in a Kubernetes Secret only the webhook can read.
- Strict request handling: body size limit, timeouts, typed `AdmissionReview` v1 parsing; malformed
  input is rejected, never panics (fuzzed, see section 9).
- Stateless; evaluates against an in-memory policy cache populated by watches.
- Exposed only through a ClusterIP Service; NetworkPolicy restricts ingress.

### 5.2 Operator (controller)

- Talks only to the Kubernetes API, over TLS with its ServiceAccount token.
- Leader election via `Lease`; only the leader writes generated objects.
- Writes only objects it owns (`ValidatingAdmissionPolicy`, bindings, `PolicyReport`), labelled and
  owner-referenced so they can be audited and garbage-collected.

### 5.3 kubectl plugin

- Runs offline against local files; no cluster access needed for `test`.
- When it does talk to a cluster, it uses the user's own kubeconfig and permissions.

### 5.4 Policy evaluation engine

- Pure Rust, no I/O, no network, no file access, deterministic.
- CEL evaluation with cost budgets; no custom code execution in the core.
- WASM plugins *(post-1.0)* run in a sandbox (wasmtime) with no WASI filesystem or network, and with
  memory and fuel limits.

## 6. RBAC (least privilege)

| Component | Resources | Verbs |
| --- | --- | --- |
| Operator | guardian policies (CRDs) | get, list, watch |
| Operator | guardian policies `/status` | update, patch |
| Operator | `validatingadmissionpolicies`, `...bindings` | get, list, watch, create, update, patch, delete — restricted to objects with the operator's label where the API allows it |
| Operator | `policyreports`, `clusterpolicyreports` | get, list, watch, create, update, patch, delete |
| Operator | workloads to scan (pods, deployments, ...) | get, list, watch (read-only) |
| Operator | `events` | create, patch |
| Operator | `leases` in its own namespace | get, create, update |
| Webhook | guardian policies | get, list, watch |
| Webhook | its TLS Secret in its own namespace | get, update (self-signed mode only) |

Never granted: `secrets` cluster-wide, `*` wildcards, `escalate`, `bind`, `impersonate`, `pods/exec`.
Each permission is justified in `docs/rbac.md` *(planned)*, and CI fails if the chart's RBAC grows
without updating that file.

## 7. Workload hardening (Helm chart defaults)

```yaml
securityContext:
  runAsNonRoot: true
  runAsUser: 65532
  allowPrivilegeEscalation: false
  readOnlyRootFilesystem: true
  capabilities: { drop: ["ALL"] }
  seccompProfile: { type: RuntimeDefault }
automountServiceAccountToken: true   # only on pods that need API access
```

- Distroless `static` base image, no shell; image runs as UID 65532.
- Resource requests and limits set; PodDisruptionBudget; anti-affinity across nodes.
- NetworkPolicy: ingress only to the webhook port from the API server, metrics port from the
  monitoring namespace; egress only to the API server and DNS.
- Passes Pod Security Standards **restricted** and the project's own restricted policy pack.
- Metrics endpoint can be protected with kube-rbac-proxy or TLS *(planned)*.

## 8. Supply chain security

| Control | Tool |
| --- | --- |
| Dependency vulnerabilities | `cargo audit` in CI and weekly; GitHub Dependabot alerts |
| Licenses, banned and duplicate crates | `cargo deny` |
| Pinned, reviewed updates | Renovate/Dependabot; `Cargo.lock` committed; GitHub Actions pinned by SHA |
| Image scanning | Trivy or Grype on every build; release blocked on critical CVEs |
| Signing | cosign keyless signatures for images, Helm charts and binaries |
| SBOM | syft (SPDX and CycloneDX) attached to every release and to the image as an attestation |
| Provenance | SLSA level 3 via `slsa-github-generator` |
| Reproducible builds | `--locked`, fixed toolchain (`rust-toolchain.toml`), `SOURCE_DATE_EPOCH` |
| Repository protection | Branch protection, required reviews, signed commits, CODEOWNERS on security-sensitive paths, secret scanning, CodeQL |
| Project health | OpenSSF Scorecard published; OpenSSF Best Practices badge |

Users can verify a release:

```bash
cosign verify ghcr.io/stochasticquant/guardian:v0.1.0 \
  --certificate-identity-regexp 'https://github.com/stochasticquant/.*' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com
```

## 9. Secure development lifecycle

- **Design:** threat model updated for every feature that crosses a trust boundary; security
  review label on PRs touching the webhook, TLS, RBAC or the chart.
- **Code:** `unsafe_code = "forbid"`, clippy with warnings as errors, no `unwrap()`/`expect()` in
  request paths (clippy lint), typed errors.
- **Test:**
  - Fuzzing of AdmissionReview parsing and the policy engine with `cargo-fuzz`, run in CI nightly.
  - Property tests (`proptest`) for match/exclude logic.
  - E2E tests that verify fail-safe behaviour (webhook down → cluster keeps working with `Ignore`).
  - kube-bench/kubescape scans of the installed chart.
- **Release:** signed artifacts, SBOM, release notes listing security fixes.
- **Independent review:** third-party security audit before 1.0 (for example via OSTIF or the CNCF
  security audit programme), with the report published.

## 10. Vulnerability management and disclosure

- Report privately through GitHub Security Advisories ("Report a vulnerability") as described in
  `SECURITY.md`. No public issues for vulnerabilities.
- Response targets: acknowledge within 3 business days, triage within 7, fix critical issues within
  30 days.
- Severity rated with CVSS v3.1; CVEs requested through GitHub.
- Fixes backported to the two most recent minor releases.
- Advisories published with affected versions, fixed versions and mitigations.

## 11. Logging, audit and data handling

- Admission decisions logged as structured JSON: timestamp, request UID, user, resource kind/name/
  namespace, policy, rule, verdict, latency. Object bodies are never logged.
- Kubernetes Events and PolicyReports provide an in-cluster audit trail.
- No personal data is collected; user names come only from admission requests and stay in-cluster.
- No telemetry and no outbound connections.

## 12. Multi-cluster security *(post-1.0)*

- Pull model preferred: member clusters push signed reports to the hub over mTLS; the hub holds no
  admin credentials for members.
- Per-cluster identities, scoped tokens, rotation; the hub has read-only access.
- Reports signed by the member operator so the hub can detect forgery.

## 13. Compliance mapping

The operator helps users meet controls in common frameworks. Mapping documents *(planned)*:

| Framework | How the operator helps |
| --- | --- |
| CIS Kubernetes Benchmark | Policy pack covering workload controls in section 5 |
| NSA/CISA Kubernetes Hardening Guide | Policy pack for pod security, network separation, image provenance |
| Pod Security Standards | Baseline and restricted packs |
| SOC 2 / ISO 27001 | Evidence via PolicyReports and admission audit logs (change management, configuration control) |
| NIST SP 800-190 | Container image and runtime controls |

## 14. What third parties can check

A security review of the project can rely on:

1. This document and the threat model.
2. `SECURITY.md` and published advisories.
3. Signed artifacts, SBOM and SLSA provenance for every release.
4. OpenSSF Scorecard and Best Practices badge.
5. RBAC justification (`docs/rbac.md`) and chart security defaults.
6. CI results: `cargo audit`, `cargo deny`, image scans, fuzzing.
7. The independent audit report (from 1.0 onward).
