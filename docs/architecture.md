# Architecture

Ferrogate is a Kubernetes policy operator written in Rust. One binary, `ferrogate`, runs as two
Deployments: the **operator** (policy status and background scans) and the **webhook** (admission).
The code is a Cargo workspace whose crates have strict dependency rules.

```text
 ferrogate                 binary: CLI, logging, wiring, shutdown
    │
    ├──► ferrogate-operator   controller, background scans, watch caches        (kube)
    │        │
    │        ▼
    │    ferrogate-crd        ClusterGatePolicy types and status                 (kube types)
    │        │
    ├──► ferrogate-webhook    AdmissionReview, TLS, /healthz /readyz /metrics    (axum, rustls)
    │        │
    ▼        ▼
         ferrogate-core       policy model, matching, rules, decisions          (no I/O)
```

| Crate | Responsibility | May depend on |
| --- | --- | --- |
| `ferrogate-core` | Rule definitions, matching, evaluation, decisions | serde, schemars, thiserror — no `kube`, no I/O |
| `ferrogate-crd` | `ClusterGatePolicy` and its status | core, kube (types only) |
| `ferrogate-operator` | Controller, scans, watch caches | core, crd, kube runtime |
| `ferrogate-webhook` | Admission and operations HTTP servers | core (not kube: it gets policies through the `PolicySource` trait) |
| `ferrogate` | Command line, wiring, offline `check` | all of the above |

Why this split: [decision 0002](decisions/0002-workspace-layout.md).

## Runtime

```text
                 ┌──────────────────────── kube-apiserver ────────────────────────┐
                 │ AdmissionReview (HTTPS)               watch / list / status    │
                 ▼                                              ▲                 │
 ferrogate-webhook (2 replicas)                       ferrogate-operator (1)      │
   caches: policies (compiled), namespace labels        reconcile ClusterGatePolicy
   POST /validate → decide() → allow / warn / deny      validate → scan → status, Events
   :8080 /healthz /readyz /metrics                      :8080 /healthz /readyz /metrics
```

- **Same engine everywhere.** The webhook, the background scan and `ferrogate check` all call
  `ferrogate_core::decide`, so they cannot disagree.
- **Webhook caches.** Policies are watched and compiled once per change; namespaces are watched as
  metadata only. Until both caches have synced, `/readyz` fails and `/validate` answers 503, so the
  API server applies the configured `failurePolicy`.
- **Operator.** Reconciles each policy: compile (invalid → `Ready=False`, Event), scan the targeted
  kinds through API discovery (paginated, capped, objects owned by a checked kind are reported
  through their owner), write `status` and metrics. Work is skipped while `observedGeneration` is
  current and the last scan is newer than the scan interval, so status writes do not loop.
  Errors back off exponentially per policy (5 s to 5 min).
- **Shutdown.** SIGTERM stops accepting connections and drains in-flight requests (10 s).

## Commands

| Command | What it does |
| --- | --- |
| `ferrogate operator` | Runs the controller (flags: `--scan-interval`, `--no-scan`, `--max-scan-objects`, `--metrics-addr`) |
| `ferrogate webhook` | Runs the admission server on `:8443` (HTTPS) and the operations server on `:8080` |
| `ferrogate check -p <policies> <resources>` | Evaluates local YAML without a cluster; exit code 1 on failures; `-o json` |
| `ferrogate crd` | Prints the CRD YAML. `config/crd/` holds the committed copy; CI fails if they differ |

Logging: `--log-format text|json` (or `FERROGATE_LOG_FORMAT`); level via `FERROGATE_LOG`
(for example `FERROGATE_LOG=debug` or `FERROGATE_LOG=ferrogate_operator=debug,info`).
Installation and all settings: [install.md](install.md).

## Policy model

```yaml
apiVersion: ferrogate.stochasticquant.io/v1alpha1
kind: ClusterGatePolicy
metadata:
  name: no-latest
spec:
  mode: Warn                 # Audit (default) | Warn | Enforce
  severity: Medium           # Low | Medium (default) | High | Critical
  match:
    resources:               # required: kinds the policy applies to
      - apiGroups: [apps]    # empty = any group
        kinds: [Deployment]  # "*" = any kind (admission only, not scanned)
    namespaces: []           # optional allow-list
    namespaceSelector: {}    # optional label selector on the namespace
    objectSelector: {}       # optional label selector on the object
  exclude:                   # optional
    namespaces: [kube-system]
    usernames: []
    groups: [system:masters]
    objectSelector: {}
  rules:                     # 1–64 rules, each with exactly one check
    - name: pinned
      disallowLatestTag: true
      message: "pin images to a version or digest"   # optional
```

Checks: `requireLabels`, `requireAnnotations`, `allowedRegistries`, `disallowLatestTag`,
`requireResourceLimits`, `requireResourceRequests`, `disallowPrivileged`,
`disallowHostNamespaces`, `disallowHostPath`, `disallowHostPorts`, `allowedCapabilities`,
`disallowPrivilegeEscalation`, `requireRunAsNonRoot`, `requireDropAllCapabilities`,
`requireSeccompProfile`. Pod-level checks find the pod spec in Pods, workload templates and CronJobs
and report paths such as `spec.template.spec.containers[0].image`.

Status:

```console
$ kubectl get cgp
NAME             MODE    SEVERITY   READY   VIOLATIONS   AGE
pss-restricted   Audit   High       True    35           2m
```

`status.conditions` has `Ready` (`Active` / `InvalidPolicy`) and `Scanned` (`Complete`,
`Incomplete`, `Truncated`, `WildcardKinds`); `status.results` holds up to 20 failing resources.

## Metrics

| Metric | Labels |
| --- | --- |
| `ferrogate_admission_requests_total` | `outcome` = skipped, allowed, warned, denied |
| `ferrogate_admission_duration_seconds` | `outcome` |
| `ferrogate_admission_violations_total` | `policy`, `mode`, `severity` |
| `ferrogate_admission_errors_total` | |
| `ferrogate_reconciles_total` | `result` = ok, invalid, error |
| `ferrogate_reconcile_duration_seconds` | |
| `ferrogate_policy_ready`, `ferrogate_policy_violations`, `ferrogate_policy_scanned_resources` | `policy` |

## TLS

Both the Kubernetes client and the webhook server use rustls with the `ring` provider; no OpenSSL.
The webhook accepts TLS 1.2 and 1.3 only and reloads its certificate when the files change (cert-manager
renews them in place). A test in the `ferrogate` crate fails if no client crypto provider is compiled
in, because that would only show up as a crash when connecting to a cluster.

## Security

How each part is secured: [SECURITY_ARCHITECTURE.md](SECURITY_ARCHITECTURE.md),
[threat model](security/threat-model.md), [RBAC](rbac.md).
