#!/usr/bin/env bash
# End-to-end test against a real cluster (kind, k3s, ...). Requires kubectl, cert-manager in the
# cluster, and the image `ferrogate:dev` available to the nodes (see tests/e2e/README.md).
#
#   tests/e2e/run.sh                 install with Kustomize, test, uninstall
#   INSTALL=helm tests/e2e/run.sh    install with the Helm chart instead
#   KEEP=1 tests/e2e/run.sh          leave Ferrogate installed afterwards
#
# HELM_ARGS adds arguments to `helm install` (for example node selectors).
#
# Only the namespace `ferrogate-e2e` is affected by the test policies.
set -euo pipefail
cd "$(dirname "$0")"

NS=ferrogate-e2e
OVERLAY=${OVERLAY:-overlay}
INSTALL=${INSTALL:-kustomize}
pass=0
fail=0

ok()   { echo "  PASS  $*"; pass=$((pass + 1)); }
bad()  { echo "  FAIL  $*"; fail=$((fail + 1)); }

cleanup() {
  kubectl delete namespace "$NS" --ignore-not-found --wait=false >/dev/null
  kubectl delete -f policies.yaml --ignore-not-found >/dev/null
  if [[ -z "${KEEP:-}" ]]; then
    if [[ $INSTALL == helm ]]; then
      helm uninstall ferrogate -n ferrogate-system --wait >/dev/null 2>&1 || true
      # Helm never deletes CRDs; the test does, to leave the cluster as it found it.
      kubectl delete -f ../../charts/ferrogate/crds/ --ignore-not-found >/dev/null 2>&1 || true
      kubectl delete namespace ferrogate-system --ignore-not-found >/dev/null 2>&1 || true
    else
      kubectl delete -k "$OVERLAY" --ignore-not-found >/dev/null 2>&1 || true
    fi
  fi
}
trap cleanup EXIT

echo "==> Installing Ferrogate ($INSTALL)"
if [[ $INSTALL == helm ]]; then
  kubectl create namespace ferrogate-system --dry-run=client -o yaml | kubectl apply -f - >/dev/null
  kubectl label namespace ferrogate-system pod-security.kubernetes.io/enforce=restricted --overwrite >/dev/null
  # shellcheck disable=SC2086 # HELM_ARGS holds several arguments.
  helm upgrade --install ferrogate ../../charts/ferrogate -n ferrogate-system \
    --set image.repository=docker.io/library/ferrogate --set image.tag=dev --set image.pullPolicy=Never \
    ${HELM_ARGS:-} >/dev/null
else
  kubectl apply -k "$OVERLAY" >/dev/null
fi
kubectl -n ferrogate-system wait certificate/ferrogate-webhook --for=condition=Ready --timeout=120s >/dev/null
kubectl -n ferrogate-system rollout status deploy/ferrogate-webhook --timeout=180s >/dev/null
kubectl -n ferrogate-system rollout status deploy/ferrogate-operator --timeout=180s >/dev/null
# Give the API server a moment to pick up the injected CA bundle.
for _ in $(seq 1 30); do
  bundle=$(kubectl get validatingwebhookconfiguration ferrogate -o jsonpath='{.webhooks[0].clientConfig.caBundle}')
  [[ -n "$bundle" ]] && break
  sleep 2
done
[[ -n "$bundle" ]] && ok "CA bundle injected" || bad "CA bundle not injected"

holder=""
for _ in $(seq 1 30); do
  holder=$(kubectl -n ferrogate-system get lease ferrogate-operator -o jsonpath='{.spec.holderIdentity}' 2>/dev/null || true)
  [[ -n "$holder" ]] && break
  sleep 1
done
kubectl -n ferrogate-system get pod "$holder" >/dev/null 2>&1 \
  && ok "one operator replica holds the leader lease ($holder)" || bad "no leader elected"

echo "==> Applying test policies"
# A previous run may still be deleting the namespace.
kubectl wait --for=delete "namespace/$NS" --timeout=180s >/dev/null 2>&1 || true
kubectl create namespace "$NS" --dry-run=client -o yaml | kubectl apply -f - >/dev/null
kubectl apply -f policies.yaml >/dev/null
for p in e2e-enforce e2e-warn e2e-invalid e2e-native; do
  kubectl wait "clustergatepolicy/$p" --for=jsonpath='{.status.observedGeneration}'=1 --timeout=60s >/dev/null
done
sleep 3  # webhook caches pick up the policies

echo "==> Status"
[[ $(kubectl get cgp e2e-enforce -o jsonpath='{.status.conditions[?(@.type=="Ready")].status}') == True ]] \
  && ok "valid policy is Ready" || bad "valid policy is not Ready"
[[ $(kubectl get cgp e2e-invalid -o jsonpath='{.status.conditions[?(@.type=="Ready")].reason}') == InvalidPolicy ]] \
  && ok "invalid policy reports InvalidPolicy" || bad "invalid policy not reported"

echo "==> Admission"
pod() { # name image [extra yaml for the container]
  cat <<YAML
apiVersion: v1
kind: Pod
metadata: {name: $1, namespace: $NS, labels: {team: e2e}}
spec:
  securityContext: {runAsNonRoot: true, runAsUser: 65532, seccompProfile: {type: RuntimeDefault}}
  containers:
    - name: app
      image: $2
      command: [sleep, "3600"]
      securityContext: {allowPrivilegeEscalation: false, capabilities: {drop: [ALL]}${3:-}}
YAML
}

if out=$(pod denied busybox:latest | kubectl apply -f - 2>&1); then
  bad "latest tag was allowed in Enforce mode"
else
  grep -q "denied by Ferrogate" <<<"$out" && ok "Enforce denies :latest with a message" || bad "unexpected error: $out"
fi

out=$(pod allowed busybox:1.36 | kubectl apply -f - 2>&1) && ok "compliant pod is allowed" || bad "compliant pod denied: $out"

out=$(pod warned busybox:1.36 ', privileged: false' | kubectl apply --dry-run=server -f - 2>&1 || true)
grep -q "Warning: \[e2e-warn\]" <<<"$out" && ok "Warn mode returns a warning" || bad "no warning: $out"

if out=$(cat <<YAML | kubectl apply -f - 2>&1
apiVersion: apps/v1
kind: Deployment
metadata: {name: denied-deploy, namespace: $NS}
spec:
  selector: {matchLabels: {app: d}}
  template:
    metadata: {labels: {app: d}}
    spec: {containers: [{name: app, image: nginx}]}
YAML
); then
  bad "Deployment with an untagged image was allowed"
else
  grep -q "spec.template.spec.containers\[0\].image" <<<"$out" && ok "Deployment denied with field path" || bad "unexpected: $out"
fi

echo "==> Native enforcement"
[[ $(kubectl get cgp e2e-native -o jsonpath='{.status.enforcement}') == Native ]] \
  && ok "native policy reports enforcement Native" || bad "native policy status: $(kubectl get cgp e2e-native -o jsonpath='{.status}')"
kubectl get validatingadmissionpolicy ferrogate-e2e-native >/dev/null 2>&1 \
  && ok "ValidatingAdmissionPolicy generated" || bad "no ValidatingAdmissionPolicy"
sleep 3  # bindings take effect asynchronously
if out=$(cat <<YAML | kubectl apply --dry-run=server -f - 2>&1
apiVersion: v1
kind: Pod
metadata: {name: native, namespace: $NS}
spec:
  containers: [{name: app, image: busybox:1.36, securityContext: {privileged: true}}]
YAML
); then
  bad "privileged pod admitted despite the native policy"
else
  grep -q "ValidatingAdmissionPolicy 'ferrogate-e2e-native'" <<<"$out" \
    && ok "API server denies through the generated policy" || bad "unexpected: $out"
fi
kubectl delete cgp e2e-native >/dev/null
# The garbage collector removes owned objects asynchronously; give a busy CI runner time.
for _ in $(seq 1 120); do
  kubectl get validatingadmissionpolicy ferrogate-e2e-native >/dev/null 2>&1 || break
  sleep 1
done
kubectl get validatingadmissionpolicy ferrogate-e2e-native >/dev/null 2>&1 \
  && bad "generated policy left behind" || ok "generated policy removed with its ClusterGatePolicy"

echo "==> Background scan and metrics"
scanned=$(kubectl get cgp e2e-enforce -o jsonpath='{.status.conditions[?(@.type=="Scanned")].status}')
[[ $scanned == True ]] && ok "background scan completed" || bad "scan condition: $scanned"
metrics=""
for pod_name in $(kubectl -n ferrogate-system get pod -l app.kubernetes.io/component=webhook -o jsonpath='{.items[*].metadata.name}'); do
  metrics+=$(kubectl get --raw "/api/v1/namespaces/ferrogate-system/pods/$pod_name:8080/proxy/metrics" 2>/dev/null || true)
  metrics+=$'\n'
done
grep -q '^ferrogate_admission_requests_total{outcome="denied"}' <<<"$metrics" \
  && ok "webhook metrics count denials" || bad "no admission metrics"
echo
echo "Result: $pass passed, $fail failed"
[[ $fail -eq 0 ]]
