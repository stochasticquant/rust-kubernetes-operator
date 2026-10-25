# Ferrogate Threat Model

Status: v0.1 (Phase A baseline), 2026-10-24. Reviewed on every minor release and whenever a change
crosses a trust boundary. The design behind the mitigations is in
[`../SECURITY_ARCHITECTURE.md`](../SECURITY_ARCHITECTURE.md).

Method: STRIDE per component and trust boundary. Each threat has an ID so issues, pull requests and
tests can refer to it. **Status** says whether the mitigation exists in the code today.

| Status | Meaning |
| --- | --- |
| ✅ Implemented | In the code or CI today |
| 🟡 Planned | Designed; scheduled in the [enterprise plan](../ENTERPRISE_PLAN.md) (phase shown) |
| ⚪ Accepted | Residual risk, documented below |

## 1. What we protect (assets)

| ID | Asset | Why it matters |
| --- | --- | --- |
| A1 | Integrity of admission decisions | Wrong "allow" lets unsafe workloads in; wrong "deny" blocks the cluster |
| A2 | Availability of the Kubernetes API | A slow or failing webhook can stall every deployment |
| A3 | Confidentiality of objects under review | Pods and ConfigMaps can contain credentials in env vars or data |
| A4 | Policy definitions | Weakening a policy silently disables protection |
| A5 | Operator credentials (ServiceAccount token, webhook TLS key) | Let an attacker act as Ferrogate |
| A6 | Release artifacts (image, chart, binaries) | A tampered release compromises every user |
| A7 | Audit trail (logs, events, reports) | Needed for investigation and compliance evidence |

## 2. Who might attack (actors)

| ID | Actor | Starting access |
| --- | --- | --- |
| E1 | Namespace user / developer | Can create workloads in some namespaces |
| E2 | Compromised workload | Code running in a pod, network access inside the cluster |
| E3 | Malicious or careless policy author | Can create or edit Ferrogate policies |
| E4 | Supply-chain attacker | Targets dependencies, CI, or the registry |
| E5 | Network attacker on the node network | Can observe or inject traffic between nodes |
| — | Cluster-admin | **Out of scope**: can already do everything Ferrogate prevents |

## 3. Trust boundaries and data flows

```text
 E1 kubectl ──► kube-apiserver ──[TB-1: HTTPS AdmissionReview]──► ferrogate webhook
                      ▲   │                                              │ cache
                      │   └─[native VAP evaluation, in-process]          ▼
                      └──────[TB-2: HTTPS + SA token]──────── ferrogate operator
 E3 policy author ──[TB-3: Kubernetes RBAC]──► ClusterGatePolicy objects
 E4 ──► dependencies / CI ──[TB-4: signed release]──► user's registry / cluster
```

## 4. Threats

### 4.1 TB-1: API server → webhook (Phase B onward)

| ID | STRIDE | Threat | Mitigation | Status |
| --- | --- | --- | --- | --- |
| T-01 | S | A pod calls the webhook directly to probe or confuse it | NetworkPolicy allows ingress only from the control plane; webhook decisions only matter when the API server asks | 🟡 B/C |
| T-02 | T / I | Traffic between API server and webhook is read or changed | HTTPS only (rustls, TLS 1.2+); certificate via cert-manager or rotated self-signed CA | 🟡 B |
| T-03 | D | Huge or deeply nested objects exhaust webhook memory or CPU | Request body size limit, timeouts below the API server's, concurrency limit | 🟡 B |
| T-04 | D | Malformed `AdmissionReview` crashes the webhook | No `unwrap`/`expect`/`panic` outside tests (lint-enforced); typed parsing; fuzzing | ✅ lints / 🟡 B fuzzing |
| T-05 | D | Webhook outage blocks all deployments | `failurePolicy: Ignore` default; system namespaces excluded; multiple replicas + PDB | 🟡 B/C |
| T-06 | E | Webhook answers "allowed" for an endpoint that is not implemented | `/validate` returns HTTP 501 until implemented, so a misconfiguration fails visibly | ✅ |

### 4.2 TB-2: operator → API server

| ID | STRIDE | Threat | Mitigation | Status |
| --- | --- | --- | --- | --- |
| T-10 | E | Over-broad RBAC lets a compromised operator take over the cluster | Least-privilege ClusterRole, no Secrets, no wildcards, no `escalate`/`bind`/`impersonate`; each rule justified in `docs/rbac.md` | 🟡 B/C |
| T-11 | I | Operator ServiceAccount token stolen from the pod | Non-root, read-only filesystem, no shell in image, dropped capabilities; short-lived projected tokens | 🟡 C |
| T-12 | T | Operator connects to a spoofed API server | rustls certificate verification against the cluster CA; no option to skip verification in production paths | ✅ |
| T-13 | D | Reconcile errors cause a hot loop against the API server | Fixed back-off (30 s) and periodic resync (5 min); exponential back-off planned | ✅ / 🟡 B |
| T-14 | R | Changes made by the operator cannot be traced | Generated objects labelled and owner-referenced; Kubernetes Events | 🟡 B |

### 4.3 TB-3: policy authors → cluster

| ID | STRIDE | Threat | Mitigation | Status |
| --- | --- | --- | --- | --- |
| T-20 | T | A user weakens or deletes a policy | Cluster-scoped policies need cluster-level RBAC; changes appear in the Kubernetes audit log; status shows the enforced generation | ✅ RBAC model / 🟡 B status |
| T-21 | D | A policy with an expensive CEL expression slows admission | CEL cost budget; evaluation timeout | 🟡 B |
| T-22 | D | A new policy blocks production immediately | `mode` defaults to `Audit` in the CRD schema | ✅ |
| T-23 | T | Invalid policy fields are accepted and silently ignored | Typed enums in the CRD schema (`Audit`/`Warn`/`Enforce`, severities); structural schema validation by the API server | ✅ |

### 4.4 Data handling

| ID | STRIDE | Threat | Mitigation | Status |
| --- | --- | --- | --- | --- |
| T-30 | I | Secrets in reviewed objects end up in logs | Logs record identity and verdict only, never object bodies (contributing rule; review checklist) | ✅ rule / 🟡 B test |
| T-31 | I | Data leaves the cluster | No telemetry, no outbound calls except the Kubernetes API; egress NetworkPolicy | ✅ code / 🟡 C NetworkPolicy |
| T-32 | R | No record of why a request was allowed or denied | Structured decision log, events, PolicyReports | 🟡 B |

### 4.5 TB-4: supply chain

| ID | STRIDE | Threat | Mitigation | Status |
| --- | --- | --- | --- | --- |
| T-40 | T | Vulnerable dependency | `cargo deny` advisories on every PR and weekly; Dependabot alerts and security updates | ✅ |
| T-41 | T | Malicious dependency from an unknown source | `cargo deny` sources: crates.io only; dependency review on PRs; `Cargo.lock` committed and `--locked` builds | ✅ |
| T-42 | T | Compromised GitHub Action | Actions pinned by commit SHA; `contents: read` default permissions; `persist-credentials: false` | ✅ |
| T-43 | T | Licence-incompatible dependency | `cargo deny` licence allow list | ✅ |
| T-44 | T | Secret committed to the repository | Secret scanning and push protection enabled | ✅ |
| T-45 | T | Tampered image or chart | cosign signatures, SBOM, SLSA provenance | 🟡 C |
| T-46 | E | Memory-safety bug in our code | Rust with `unsafe_code = "forbid"` | ✅ |
| T-47 | T | Unreviewed change to security-sensitive code | CODEOWNERS on webhook, CRD, config, workflows; branch protection | ✅ CODEOWNERS / 🟡 protection |

## 5. Accepted risks

| ID | Risk | Why accepted |
| --- | --- | --- |
| R-1 | A cluster-admin can disable Ferrogate | Out of scope: cluster-admin is fully trusted by Kubernetes itself |
| R-2 | With `failurePolicy: Ignore`, an attacker who can take the webhook down can bypass webhook-only rules | Availability is preferred by default; users who need fail-closed can set `Fail`. Rules compiled to native VAP are not affected |
| R-3 | Dependencies contain `unsafe` code (for example `ring`) | Widely audited crates; tracked through RustSec and `cargo deny` |

## 6. Review log

| Date | Version | Change |
| --- | --- | --- |
| 2026-10-24 | 0.1 | Initial threat model for the Phase A foundation |
