# RBAC

Ferrogate runs as two service accounts with separate, read-mostly permissions. It has **no access to
Secrets**, uses **no wildcards**, and has none of the `escalate`, `bind` or `impersonate` verbs.
The manifests are in [`config/deploy/rbac.yaml`](../config/deploy/rbac.yaml).

## `ferrogate-operator`

| API group | Resource | Verbs | Why |
| --- | --- | --- | --- |
| `ferrogate.stochasticquant.io` | `clustergatepolicies` | get, list, watch | Reconcile policies |
| `ferrogate.stochasticquant.io` | `clustergatepolicies/status` | get, patch, update | Write conditions and scan results |
| `events.k8s.io` | `events` | create, patch | Report transitions as Kubernetes Events |
| `""` | `namespaces` | list, watch | Namespace labels for `namespaceSelector` (metadata only) |
| via `ferrogate-scan` | workload kinds (below) | list | Background scans |

`ferrogate-scan` is an **aggregated** ClusterRole. The built-in `ferrogate-scan-workloads` grants
`list` on Pods, ReplicationControllers, Deployments, StatefulSets, DaemonSets, ReplicaSets, Jobs and
CronJobs. To scan other kinds, add a ClusterRole with the aggregation label:

```yaml
apiVersion: rbac.authorization.k8s.io/v1
kind: ClusterRole
metadata:
  name: ferrogate-scan-ingresses
  labels:
    ferrogate.stochasticquant.io/aggregate-to-scan: "true"
rules:
  - apiGroups: [networking.k8s.io]
    resources: [ingresses]
    verbs: [list]
```

Without it the scan reports `Scanned=False, reason=Incomplete` with the `Forbidden` kinds; admission
is not affected.

Note: `list` returns full objects, including environment variables. Ferrogate evaluates them in
memory and never logs or stores object contents (threat T-30); status results contain only kind,
namespace, name, rule, message and field path.

## `ferrogate-webhook`

| API group | Resource | Verbs | Why |
| --- | --- | --- | --- |
| `ferrogate.stochasticquant.io` | `clustergatepolicies` | list, watch | Policy cache |
| `""` | `namespaces` | list, watch | Namespace labels (metadata only) |

The webhook receives objects from the API server in `AdmissionReview` requests; it needs no read
access to workloads.
