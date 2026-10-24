# Developer shortcuts. CI runs the same commands; see .github/workflows/ci.yml.
.PHONY: all fmt lint test crds image e2e e2e-helm native help

CRD := config/crd/clustergatepolicies.ferrogate.stochasticquant.io.yaml

all: fmt lint test  ## Format, lint and test

fmt:  ## Format the code
	cargo fmt --all

lint:  ## Clippy with warnings as errors, plus cargo-deny
	cargo clippy --workspace --all-targets -- -D warnings
	cargo deny check

test:  ## Unit, integration, golden and property tests
	cargo test --workspace

crds:  ## Regenerate the CRD (Kustomize and Helm copies)
	cargo run -q -p ferrogate -- crd > $(CRD)
	cp $(CRD) charts/ferrogate/crds/

image:  ## Build the container image ferrogate:dev
	docker build -t ferrogate:dev .

e2e:  ## End-to-end test on the current cluster (Kustomize install)
	tests/e2e/run.sh

e2e-helm:  ## End-to-end test on the current cluster (Helm install)
	INSTALL=helm tests/e2e/run.sh

native: ## Native enforcement equivalence test on the current cluster
	cargo build -q -p ferrogate
	tests/e2e/native.sh

help:  ## List targets
	@grep -E '^[a-z0-9-]+:.*## ' $(MAKEFILE_LIST) | awk -F':.*## ' '{printf "  %-10s %s\n", $$1, $$2}'
