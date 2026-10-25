# Ferrogate Helm chart

Installs the Ferrogate operator (2 replicas with leader election), the admission webhook (2
replicas), RBAC, the `ClusterGatePolicy` CRD, a cert-manager certificate for the webhook, PDBs,
a NetworkPolicy and optionally a Prometheus `ServiceMonitor`.

Requirements: Kubernetes 1.30+, [cert-manager](https://cert-manager.io) (for the webhook).

```sh
kubectl create namespace ferrogate-system
kubectl label namespace ferrogate-system pod-security.kubernetes.io/enforce=restricted
helm install ferrogate charts/ferrogate -n ferrogate-system
```

Prometheus Operator users (for example kube-prometheus-stack, which selects `release: <name>`):

```sh
helm upgrade --install ferrogate charts/ferrogate -n ferrogate-system \
  --set metrics.serviceMonitor.enabled=true \
  --set metrics.serviceMonitor.labels.release=monitoring
```

Uninstall: `helm uninstall ferrogate -n ferrogate-system`. Helm keeps CRDs; delete them with
`kubectl delete crd clustergatepolicies.ferrogate.stochasticquant.io` (this deletes all policies).

## Values

| Value | Default | |
| --- | --- | --- |
| `image.repository` | `ghcr.io/stochasticquant/ferrogate` | |
| `image.tag` | chart `appVersion` | |
| `image.digest` | `""` | pin by digest, e.g. `sha256:…` (recommended) |
| `logFormat` / `logLevel` | `json` / `info` | |
| `operator.replicas` | `2` | leader election picks one |
| `operator.scan.enabled` | `true` | background scans of existing resources |
| `operator.scan.intervalSeconds` | `300` | minimum 30 |
| `operator.scan.maxObjects` | `20000` | per policy and scan |
| `webhook.enabled` | `true` | `false`: only `enforcement: Native` policies are enforced at admission |
| `webhook.replicas` | `2` | |
| `webhook.failurePolicy` | `Ignore` | `Fail` to deny requests while Ferrogate is unavailable |
| `webhook.timeoutSeconds` | `5` | |
| `webhook.excludedNamespaces` | `[]` | added to `kube-system`, `kube-node-lease` and the release namespace |
| `certManager.enabled` | `true` | required while the webhook is enabled |
| `certManager.duration` | `2160h` | serving certificate validity; renewed 15 days early, reloaded live |
| `podDisruptionBudget.enabled` | `true` | `maxUnavailable: 1` |
| `networkPolicy.enabled` | `true` | ingress only on 8443 (webhook) and 8080 (probes, metrics) |
| `metrics.serviceMonitor.enabled` | `false` | |
| `metrics.serviceMonitor.labels` | `{}` | labels your Prometheus selects on |
| `*.resources`, `*.nodeSelector`, `*.tolerations`, `*.affinity`, `*.podAnnotations` | | per component |

All values are validated by [`values.schema.json`](values.schema.json); unknown keys are rejected.
