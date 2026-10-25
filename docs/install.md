# Installing Ferrogate

Status: pre-release (`v1alpha1`). A Helm chart and signed images arrive in Phase C.

## Requirements

- Kubernetes 1.30 or later (CRD validation rules are GA)
- [cert-manager](https://cert-manager.io) for the webhook certificate
- `kubectl` 1.27+ (for `-k`)

## Install

```sh
kubectl apply -k config/deploy
kubectl -n ferrogate-system rollout status deploy/ferrogate-webhook
kubectl -n ferrogate-system rollout status deploy/ferrogate-operator
```

This creates the CRD, the `ferrogate-system` namespace (restricted Pod Security), RBAC, the
operator (1 replica), the webhook (2 replicas, PDB), a self-signed serving certificate from
cert-manager, and the `ValidatingWebhookConfiguration`.

## First policy

Start with a pack in `Audit` mode; nothing is blocked:

```sh
kubectl apply -f policies/pss-baseline/policy.yaml
kubectl get cgp
# NAME           MODE    SEVERITY   READY   VIOLATIONS   AGE
# pss-baseline   Audit   High       True    17           30s
kubectl get cgp pss-baseline -o jsonpath='{.status.results}' | jq
```

When the results look right, switch to `Warn` (clients see warnings), then `Enforce`.

## How admission works

| Mode | Admission | Background scan |
| --- | --- | --- |
| `Audit` | allowed; finding added to the API server audit log (`auditAnnotations`) | reported |
| `Warn` | allowed with a warning shown by `kubectl` | reported |
| `Enforce` | **denied** with a message and field path | reported |

The webhook configuration:
- intercepts `CREATE` and `UPDATE` of workload kinds (Pods, ReplicationControllers, Deployments,
  StatefulSets, DaemonSets, ReplicaSets, Jobs, CronJobs). To enforce policies on other kinds, add
  them to `config/deploy/webhook-configuration.yaml`;
- skips `kube-system`, `kube-node-lease` and `ferrogate-system`;
- uses `failurePolicy: Ignore`: if Ferrogate is down, requests are allowed (see accepted risk R-2
  in the [threat model](security/threat-model.md)). Set `Fail` if you need fail-closed.

## Configuration

| Flag | Env | Default | |
| --- | --- | --- | --- |
| `operator --scan-interval` | `FERROGATE_SCAN_INTERVAL` | `300` | seconds between scans (min 30) |
| `operator --no-scan` | `FERROGATE_NO_SCAN` | off | disable background scans |
| `operator --max-scan-objects` | `FERROGATE_MAX_SCAN_OBJECTS` | `20000` | per policy and scan |
| `webhook --tls-cert/--tls-key` | `FERROGATE_TLS_CERT/KEY` | `/tls/tls.crt`, `/tls/tls.key` | reloaded on change |
| `webhook --max-body-bytes` | `FERROGATE_MAX_BODY_BYTES` | 7 MiB | |
| `--metrics-addr` | `FERROGATE_METRICS_ADDR` | `0.0.0.0:8080` | `/healthz`, `/readyz`, `/metrics` |
| `--log-format` | `FERROGATE_LOG_FORMAT` | `text` | `json` in the manifests |

## Uninstall

```sh
kubectl delete validatingwebhookconfiguration ferrogate   # stop admission first
kubectl delete -k config/deploy
```
