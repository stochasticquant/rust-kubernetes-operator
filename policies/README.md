# Policy packs

Ready-to-use `ClusterGatePolicy` definitions. All start in `Audit` mode and skip `kube-system`.

| Pack | What it checks |
| --- | --- |
| [`pss-baseline`](pss-baseline/policy.yaml) | [Pod Security Standards: baseline](https://kubernetes.io/docs/concepts/security/pod-security-standards/#baseline): no privileged containers, host namespaces, `hostPath`, host ports, or capabilities outside the baseline list |
| [`pss-restricted`](pss-restricted/policy.yaml) | [Restricted](https://kubernetes.io/docs/concepts/security/pod-security-standards/#restricted): baseline, plus no privilege escalation, run as non-root, drop `ALL` capabilities (only `NET_BIND_SERVICE` may be added), seccomp `RuntimeDefault` or `Localhost` |
| [`best-practices`](best-practices/policy.yaml) | No `latest`/untagged images, memory limits, CPU and memory requests |

Not yet covered from PSS (planned with CEL rules): `procMount`, AppArmor and SELinux options,
`sysctls`, Windows host process, and the restricted volume-type allow-list.

## Native enforcement

Every pack can run as `enforcement: Native` (a generated `ValidatingAdmissionPolicy`):

```sh
ferrogate native policies/pss-restricted/policy.yaml   # preview the generated objects
```

`tests/e2e/native.sh` checks on a real API server that the native verdicts match the fixtures.

## Testing

Every pack has fixtures that CI checks (`cargo test -p ferrogate --test policy_packs`):

- `fixtures/pass/*.yaml` must not violate any rule;
- `fixtures/fail/<rule>--<description>.yaml` must violate exactly `<rule>`.

Try a pack against your own manifests without a cluster:

```sh
ferrogate check -p policies/pss-restricted ./k8s/
ferrogate check -p policies/ -o json ./k8s/ | jq '.resources[] | select(.passed | not)'
```
