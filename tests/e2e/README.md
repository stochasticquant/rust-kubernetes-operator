# End-to-end tests

`run.sh` installs Ferrogate into a real cluster, applies test policies that only match the
`ferrogate-e2e` namespace, and checks:

- the webhook certificate is issued and the CA bundle injected;
- valid policies become `Ready`, invalid ones report `InvalidPolicy`;
- `Enforce` denies a `:latest` Pod and an untagged Deployment (with the field path), `Warn` returns a
  warning, a compliant Pod is admitted;
- the background scan completes and the webhook exposes metrics.

It removes everything afterwards (`KEEP=1` keeps Ferrogate installed).

## kind

```sh
kind create cluster
kubectl apply -f https://github.com/cert-manager/cert-manager/releases/download/v1.21.1/cert-manager.yaml
kubectl -n cert-manager rollout status deploy/cert-manager-webhook
docker build -t ferrogate:dev .
kind load docker-image ferrogate:dev
tests/e2e/run.sh
```

## Other clusters

Import the image on the nodes (k3s: `docker save ferrogate:dev | sudo k3s ctr images import -`),
or push it to a registry and point an overlay at it. Use `OVERLAY=<dir>` to run with your own
kustomize overlay (for example to add a `nodeSelector`).
