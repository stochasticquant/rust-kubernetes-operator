#!/usr/bin/env bash
# Checks that native enforcement (generated ValidatingAdmissionPolicies) reaches the same verdicts
# as the engine: every policy pack fixture is server-side dry-run against the generated policies.
# Needs only kubectl and a Kubernetes 1.30+ cluster; Ferrogate itself does not have to be installed.
#
#   tests/e2e/native.sh                       uses target/debug/ferrogate
#   FERROGATE="docker run --rm ..." tests/e2e/native.sh
set -euo pipefail
cd "$(dirname "$0")/../.."

FERROGATE=${FERROGATE:-target/debug/ferrogate}
NS=ferrogate-native
WORK=$(mktemp -d "${TMPDIR:-/tmp}/ferrogate-native.XXXXXX")
chmod 755 "$WORK"
pass=0
fail=0
ok()  { echo "  PASS  $*"; pass=$((pass + 1)); }
bad() { echo "  FAIL  $*"; fail=$((fail + 1)); }

cleanup() {
  [[ -f "$WORK/native.yaml" ]] && kubectl delete -f "$WORK/native.yaml" --ignore-not-found >/dev/null 2>&1 || true
  kubectl delete namespace "$NS" --ignore-not-found --wait=false >/dev/null 2>&1 || true
  rm -rf "$WORK"
}
trap cleanup EXIT

kubectl wait --for=delete "namespace/$NS" --timeout=180s >/dev/null 2>&1 || true
kubectl create namespace "$NS" --dry-run=client -o yaml | kubectl apply -f - >/dev/null

# One pack at a time, like the golden test: fixtures only expect their own pack's verdicts.
for pack in policies/*/; do
  name=$(basename "$pack")
  echo "==> $name (native, Enforce, namespace $NS only)"
  sed -e "s/^  name: $name\$/  name: native-$name/" \
      -e 's/^  mode: Audit$/  mode: Enforce/' \
      -e "s/^  match:\$/  match:\n    namespaces: [$NS]/" \
      "$pack/policy.yaml" > "$WORK/policy.yaml"
  # shellcheck disable=SC2086 # FERROGATE may be a command with arguments.
  $FERROGATE native "$WORK/policy.yaml" > "$WORK/native.yaml"
  kubectl apply -f "$WORK/native.yaml" >/dev/null

  vap=validatingadmissionpolicy/ferrogate-native-$name
  for _ in $(seq 1 30); do
    [[ -n $(kubectl get "$vap" -o jsonpath='{.status.observedGeneration}') ]] && break
    sleep 1
  done
  warnings=$(kubectl get "$vap" -o jsonpath='{.status.typeChecking.expressionWarnings}')
  [[ -z "$warnings" ]] && ok "type-checks cleanly" || bad "type-check warnings: $warnings"
  sleep 3  # bindings take effect asynchronously

  for fixture in "$pack"fixtures/pass/*.yaml "$pack"fixtures/fail/*.yaml; do
    sed "s/namespace: default/namespace: $NS/" "$fixture" > "$WORK/fixture.yaml"
    out=$(kubectl apply --dry-run=server -f "$WORK/fixture.yaml" 2>&1) && allowed=1 || allowed=0
    file=$(basename "$fixture" .yaml)
    case "$fixture" in
      */pass/*)
        [[ $allowed == 1 ]] && ok "pass/$file allowed" || bad "pass/$file denied: $out" ;;
      *)
        rule=${file%%--*}
        if [[ $allowed == 0 ]] && grep -q "\[native-$name\] $rule:" <<<"$out"; then
          ok "fail/$file denied by $rule"
        else
          bad "fail/$file: $out"
        fi ;;
    esac
  done
  kubectl delete -f "$WORK/native.yaml" >/dev/null
done
echo
echo "Result: $pass passed, $fail failed"
[[ $fail -eq 0 ]]
