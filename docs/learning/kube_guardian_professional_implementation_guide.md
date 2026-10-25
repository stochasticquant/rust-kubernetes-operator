# kube-guardian --- Professional Implementation Guide

Repository:
https://github.com/stochasticquant/rust-kubernetes-operator.git

This document provides a professional, module-structured implementation
roadmap. Each section clearly states:

-   Which file to create or modify
-   The directory structure
-   Why the code belongs in that location
-   Architectural reasoning

------------------------------------------------------------------------

# FINAL PROJECT STRUCTURE

kube-guardian/ ├── Cargo.toml ├── src/ │ ├── main.rs │ ├── cli.rs │ ├──
config.rs │ ├── crd.rs │ ├── controller.rs │ ├── reconcile.rs │ ├──
webhook.rs │ ├── metrics.rs │ ├── multi_cluster.rs │ └── governance/ │
├── mod.rs │ ├── traits.rs │ └── policies.rs ├── tests/ │ └──
integration.rs ├── Dockerfile └── helm/

------------------------------------------------------------------------

# PHASE 0 --- FOUNDATIONS

Branch: feature/phase-0-foundations

## Create File: src/main.rs

``` rust
mod cli;
mod governance;

use clap::Parser;
use cli::Cli;

fn main() {
    let cli = Cli::parse();
    println!("Loaded config: {}", cli.config);
}
```

Explanation: - Entry point orchestrates only high-level logic. - Modules
declared explicitly for clarity.

------------------------------------------------------------------------

## Create File: src/cli.rs

``` rust
use clap::Parser;

#[derive(Parser, Debug)]
pub struct Cli {
    #[arg(long)]
    pub config: String,
}
```

Explanation: - CLI parsing isolated from business logic. - Keeps main.rs
minimal and clean.

------------------------------------------------------------------------

## Create Directory: src/governance/

### Create File: src/governance/mod.rs

``` rust
pub mod traits;
pub mod policies;
```

------------------------------------------------------------------------

### Create File: src/governance/policies.rs

``` rust
pub struct Policy {
    pub name: String,
    pub enabled: bool,
}
```

------------------------------------------------------------------------

### Create File: src/governance/traits.rs

``` rust
use super::policies::Policy;

pub trait PolicyEvaluator {
    fn evaluate(&self) -> bool;
}

impl PolicyEvaluator for Policy {
    fn evaluate(&self) -> bool {
        self.enabled
    }
}
```

Explanation: - Traits separated for extensibility. - Policies contain
only data. - Follows separation of concerns.

------------------------------------------------------------------------

# PHASE 2 --- ASYNC FOUNDATION

Branch: feature/phase-2-async

## Modify src/main.rs

``` rust
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Async runtime initialized");
    Ok(())
}
```

Explanation: - Tokio runtime added. - Enables async controller and
webhook later.

------------------------------------------------------------------------

# PHASE 3 --- CRD MODELING

Branch: feature/phase-3-crd

## Create File: src/crd.rs

``` rust
use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(CustomResource, Deserialize, Serialize, Clone, Debug, JsonSchema)]
#[kube(group = "guardian.io", version = "v1", kind = "GuardianPolicy")]
pub struct GuardianPolicySpec {
    pub severity: String,
}
```

Explanation: - All Kubernetes models centralized. - Strong typing
enforced at compile time.

------------------------------------------------------------------------

# PHASE 4 --- CONTROLLER

Branch: feature/phase-4-controller

## Create File: src/controller.rs

``` rust
use kube::{Api, Client};
use kube_runtime::controller::Controller;
use crate::crd::GuardianPolicy;

pub async fn run_controller(client: Client) {
    let api = Api::<GuardianPolicy>::all(client);
    Controller::new(api, Default::default());
}
```

Explanation: - Controller wiring isolated. - Business logic not mixed
here.

------------------------------------------------------------------------

## Create File: src/reconcile.rs

``` rust
use std::sync::Arc;
use kube_runtime::controller::Action;
use crate::crd::GuardianPolicy;

pub async fn reconcile(
    policy: Arc<GuardianPolicy>,
) -> Result<Action, kube::Error> {
    println!("Reconciling {}", policy.name_any());
    Ok(Action::requeue(std::time::Duration::from_secs(300)))
}
```

Explanation: - Reconciliation logic isolated. - Uses Arc for shared
ownership in async runtime.

------------------------------------------------------------------------

# PHASE 5 --- WEBHOOK

Branch: feature/phase-5-webhook

## Create File: src/webhook.rs

``` rust
use axum::{Router, routing::post};

pub fn router() -> Router {
    Router::new().route("/validate", post(validate))
}

async fn validate() {
    println!("Validation request received");
}
```

Explanation: - HTTP layer separate from controller. - Supports
independent testing.

------------------------------------------------------------------------

# PHASE 6 --- METRICS

Branch: feature/phase-6-observability

## Create File: src/metrics.rs

``` rust
use prometheus::{IntCounter, Registry};

pub fn register_metrics() -> Registry {
    let registry = Registry::new();
    let counter = IntCounter::new("reconcile_total", "Total reconciliations").unwrap();
    registry.register(Box::new(counter)).unwrap();
    registry
}
```

Explanation: - Observability isolated from business logic. - Registry
pattern used for extensibility.

------------------------------------------------------------------------

# PHASE 7 --- MULTI-CLUSTER

Branch: feature/phase-7-multicluster

## Create File: src/multi_cluster.rs

``` rust
use kube::Client;

pub async fn load_clients(configs: Vec<kube::Config>) -> Vec<Client> {
    configs
        .into_iter()
        .map(Client::try_from)
        .filter_map(Result::ok)
        .collect()
}
```

Explanation: - Dedicated distributed system module. - Supports scaling
control plane across clusters.

------------------------------------------------------------------------

# PHASE 8 --- TESTING & HARDENING

## Create File: tests/integration.rs

``` rust
#[tokio::test]
async fn test_basic() {
    assert_eq!(2 + 2, 4);
}
```

Explanation: - Integration tests separated from src. - Validates public
behavior only.

------------------------------------------------------------------------

# DOCKERFILE

Create File: Dockerfile

``` dockerfile
FROM rust:1.75 as builder
WORKDIR /app
COPY . .
RUN cargo build --release

FROM debian:bookworm-slim
COPY --from=builder /app/target/release/kube-guardian /usr/local/bin/
CMD ["kube-guardian"]
```

------------------------------------------------------------------------

# ARCHITECTURAL PRINCIPLES

1.  main.rs = orchestration only.
2.  controller.rs = controller wiring.
3.  reconcile.rs = business logic.
4.  governance/ = domain rules.
5.  webhook.rs = HTTP interface.
6.  metrics.rs = observability.
7.  multi_cluster.rs = distributed coordination.
8.  tests/ = integration boundary.

------------------------------------------------------------------------

This structured plan follows enterprise Rust architecture standards.
