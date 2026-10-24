# Ferrogate Threat Model

Status: v0.2 (Phase B core), 2026-10-24. Reviewed on every minor release and whenever a change
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

### 4.1 TB-1: API server → webhook

| ID | STRIDE | Threat | Mitigation | Status |
| --- | --- | --- | --- | --- |
| T-01 | S | A pod calls the webhook directly to probe or confuse it | Webhook decisions only take effect when the API server asks; NetworkPolicy limits ingress to the two ports. Restricting sources to the control plane depends on the CNI | ✅ ports / 🟡 C sources |
| T-02 | T / I | Traffic between API server and webhook is read or changed | HTTPS only (rustls with `ring`, TLS 1.2/1.3); cert-manager certificate (ECDSA P-256, 90 days, key rotated on renewal) reloaded from disk without restart | ✅ |
| T-03 | D | Huge or deeply nested objects exhaust webhook memory or CPU | 7 MiB body limit, 4 s handler timeout (the API server waits 5 s), 512 concurrent requests, bounded list sizes in policies | ✅ |
| T-04 | D | Malformed `AdmissionReview` crashes the webhook | No `unwrap`/`expect`/`panic` outside tests (lint-enforced); typed parsing; property tests feed random bytes, random reviews and random workload-shaped objects to the endpoint and engine | ✅ |
| T-05 | D | Webhook outage blocks all deployments | `failurePolicy: Ignore`; `kube-system`, `kube-node-lease` and `ferrogate-system` excluded; 2 replicas with anti-affinity and a PDB; only workload kinds are intercepted | ✅ |
| T-06 | E | Webhook allows requests before it knows the policies | Until the policy and namespace caches have synced, `/validate` answers 503 so the API server applies the configured `failurePolicy`, and `/readyz` keeps the pod out of the Service | ✅ |

### 4.2 TB-2: operator → API server

| ID | STRIDE | Threat | Mitigation | Status |
| --- | --- | --- | --- | --- |
| T-10 | E | Over-broad RBAC lets a compromised operator take over the cluster | Separate operator and webhook accounts; no Secrets, no wildcards, no `escalate`/`bind`/`impersonate`; scan access is `list` only, through an aggregated role; each rule justified in [`docs/rbac.md`](../rbac.md) | ✅ |
| T-11 | I | Operator ServiceAccount token stolen from the pod | Distroless image (no shell), user 65532, read-only root filesystem, all capabilities dropped, seccomp `RuntimeDefault`, namespace enforces restricted PSS; projected tokens are short-lived by default | ✅ |
| T-12 | T | Operator connects to a spoofed API server | rustls certificate verification against the cluster CA; no option to skip verification in production paths | ✅ |
| T-13 | D | Reconcile errors cause a hot loop against the API server | Per-policy exponential back-off (5 s to 5 min); status writes do not re-trigger work; scans paginated (500) and capped (20 000 objects) | ✅ |
| T-14 | R | Changes made by the operator cannot be traced | Kubernetes Events on transitions; the operator only writes policy status. Generated objects (VAP) will be labelled and owner-referenced | ✅ / 🟡 B VAP |
| T-15 | T | Two operator replicas act at once (split brain) | Lease-based leader election; a leader that cannot renew within 10 s exits before the 15 s lease can be taken over; the operator only writes policy status | ✅ |

### 4.3 TB-3: policy authors → cluster

| ID | STRIDE | Threat | Mitigation | Status |
| --- | --- | --- | --- | --- |
| T-20 | T | A user weakens or deletes a policy | Cluster-scoped policies need cluster-level RBAC; changes appear in the Kubernetes audit log; `status.observedGeneration` and `Ready` show what is enforced | ✅ |
| T-21 | D | A policy with an expensive rule slows admission | Built-in rules are linear in object size; list sizes are capped in the schema (64 rules, 128 entries); CEL rules will get a cost budget | ✅ built-in / 🟡 CEL |
| T-22 | D | A new policy blocks production immediately | `mode` defaults to `Audit` in the CRD schema | ✅ |
| T-23 | T | Invalid policy fields are accepted and silently ignored | Typed CRD schema; the API server rejects rules without exactly one check (CEL validation rule); remaining errors set `Ready=False, reason=InvalidPolicy` with an Event | ✅ |
| T-24 | D | A policy accidentally matches every kind and blocks the cluster | `match.resources` is required; the webhook configuration intercepts workload kinds only | ✅ |

### 4.4 Data handling

| ID | STRIDE | Threat | Mitigation | Status |
| --- | --- | --- | --- | --- |
| T-30 | I | Secrets in reviewed objects end up in logs or status | Admission logs record uid, kind, name, user and verdict only; scan results store kind, name, rule, message and path, never values (messages name only containers, images, volumes and fields) | ✅ / 🟡 log test |
| T-31 | I | Data leaves the cluster | No telemetry, no outbound calls except the Kubernetes API; egress NetworkPolicy | ✅ code / 🟡 C NetworkPolicy |
| T-32 | R | No record of why a request was allowed or denied | Structured decision log per request; `Audit` findings as API server `auditAnnotations`; Events; scan results in status; PolicyReports planned | ✅ / 🟡 reports |

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
| 2026-10-24 | 0.2 | Webhook, TLS, RBAC, scans and hardened deployment implemented; added T-24 |
