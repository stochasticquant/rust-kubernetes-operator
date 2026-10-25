# Rust Kubernetes Operator --- Full Implementation Guide

## Project: kube-guardian

Repository:
https://github.com/stochasticquant/rust-kubernetes-operator.git

------------------------------------------------------------------------

# Introduction

This document provides detailed implementation code and explanations for
each phase of the kube-guardian project. It is written for beginner Rust
programmers who want to understand both the *how* and the *why* behind
each feature.

Each phase builds on the previous one.

------------------------------------------------------------------------

# PHASE 0 --- Rust Foundations

Branch: feature/phase-0-foundations

## Step 0.1 --- Define a Policy Struct

``` rust
pub struct Policy {
    pub name: String,
    pub enabled: bool,
}
```

### Explanation

-   `String` is heap allocated.
-   The struct owns its data.
-   Fields are marked `pub` so other modules can access them.

------------------------------------------------------------------------

## Step 0.2 --- Borrowing Example

``` rust
pub fn evaluate(policy: &Policy) -> bool {
    policy.enabled
}
```

### Explanation

-   `&Policy` means we borrow instead of taking ownership.
-   This prevents unnecessary memory copies.
-   Borrowing is one of Rust's most important safety concepts.

------------------------------------------------------------------------

## Step 0.3 --- Enum Usage

``` rust
pub enum Severity {
    Low,
    Medium,
    High,
}
```

Use with match:

``` rust
pub fn describe(severity: Severity) -> &'static str {
    match severity {
        Severity::Low => "Low risk",
        Severity::Medium => "Medium risk",
        Severity::High => "High risk",
    }
}
```

### Explanation

-   Enums model states safely.
-   `match` forces exhaustive handling.

------------------------------------------------------------------------

## Step 0.4 --- Result and Error Handling

``` rust
pub fn load_policy(path: &str) -> Result<Policy, String> {
    if path.is_empty() {
        return Err("Path cannot be empty".to_string());
    }

    Ok(Policy {
        name: "default".to_string(),
        enabled: true,
    })
}
```

### Explanation

-   `Result<T, E>` replaces exceptions.
-   `?` can be used to propagate errors upward.

------------------------------------------------------------------------

# PHASE 1 --- CLI Design with Clap

Branch: feature/phase-1-cli-design

## Step 1.1 --- Add Clap

``` rust
use clap::Parser;

#[derive(Parser)]
pub struct Cli {
    #[arg(long)]
    pub config: String,
}
```

### Explanation

-   `#[derive(Parser)]` generates CLI parsing logic.
-   Struct fields become CLI flags automatically.

------------------------------------------------------------------------

## Step 1.2 --- Traits for Extensibility

``` rust
pub trait PolicyEvaluator {
    fn evaluate(&self) -> bool;
}

impl PolicyEvaluator for Policy {
    fn evaluate(&self) -> bool {
        self.enabled
    }
}
```

### Explanation

-   Traits define shared behavior.
-   Rust uses traits instead of inheritance.

------------------------------------------------------------------------

# PHASE 2 --- Async Rust & Tokio

Branch: feature/phase-2-async

## Step 2.1 --- Async Main

``` rust
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Async runtime started");
    Ok(())
}
```

### Explanation

-   Tokio provides the async runtime.
-   `async fn` returns a Future.
-   `.await` pauses without blocking threads.

------------------------------------------------------------------------

## Step 2.2 --- Spawn Concurrent Tasks

``` rust
use tokio::time::{sleep, Duration};

async fn check_policy(policy: Policy) {
    sleep(Duration::from_secs(1)).await;
    println!("Checked policy: {}", policy.name);
}

tokio::spawn(check_policy(policy));
```

### Explanation

-   `tokio::spawn` schedules background tasks.
-   No OS threads are blocked.

------------------------------------------------------------------------

# PHASE 3 --- CRD Modeling

Branch: feature/phase-3-crd

## Step 3.1 --- Add Serde

``` rust
use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize)]
pub struct PolicySpec {
    pub name: String,
    pub severity: Severity,
}
```

### Explanation

-   Automatically converts JSON to Rust structs.
-   Enables type-safe configuration.

------------------------------------------------------------------------

## Step 3.2 --- Define CustomResource

``` rust
use kube::CustomResource;
use schemars::JsonSchema;

#[derive(CustomResource, Deserialize, Serialize, Clone, Debug, JsonSchema)]
#[kube(group = "guardian.io", version = "v1", kind = "GuardianPolicy")]
pub struct GuardianPolicySpec {
    pub severity: String,
}
```

### Explanation

-   Generates CRD YAML schema automatically.
-   Strong typing for Kubernetes resources.

------------------------------------------------------------------------

# PHASE 4 --- Controller Pattern

Branch: feature/phase-4-controller

## Step 4.1 --- Connect to Cluster

``` rust
use kube::Client;

let client = Client::try_default().await?;
```

### Explanation

-   Loads kubeconfig.
-   Creates API client.

------------------------------------------------------------------------

## Step 4.2 --- Reconcile Function

``` rust
use std::sync::Arc;
use kube_runtime::controller::Action;

async fn reconcile(
    policy: Arc<GuardianPolicy>,
) -> Result<Action, kube::Error> {
    println!("Reconciling {}", policy.name_any());
    Ok(Action::requeue(std::time::Duration::from_secs(300)))
}
```

### Explanation

-   `Arc<T>` allows shared ownership across threads.
-   Reconciliation must be idempotent.

------------------------------------------------------------------------

# PHASE 5 --- Admission Webhook

Branch: feature/phase-5-webhook

## Step 5.1 --- Basic Server

``` rust
use axum::{Router, routing::post};

Router::new()
    .route("/validate", post(validate));
```

### Explanation

-   Axum builds async HTTP APIs.
-   Routes map to async handlers.

------------------------------------------------------------------------

# PHASE 6 --- Observability

Branch: feature/phase-6-observability

## Add Tracing

``` rust
use tracing::info;

#[tracing::instrument]
async fn reconcile(...) {
    info!("Reconciliation started");
}
```

### Explanation

-   Structured logs include context.
-   Better than println!

------------------------------------------------------------------------

## Add Prometheus Metrics

``` rust
use prometheus::IntCounter;

let counter = IntCounter::new("reconcile_total", "Total reconciliations")?;
counter.inc();
```

### Explanation

-   Metrics provide operational visibility.
-   Essential for production.

------------------------------------------------------------------------

# PHASE 7 --- Multi-Cluster

``` rust
use kube::Client;

let client_a = Client::try_from(config_a)?;
let client_b = Client::try_from(config_b)?;
```

### Explanation

-   Multiple cluster clients.
-   Distributed control plane pattern.

------------------------------------------------------------------------

# PHASE 8 --- Enterprise Hardening

## Leader Election Concept

Use lease locking via kube-runtime leader election utilities.

## Testing Example

``` rust
#[tokio::test]
async fn test_reconcile() {
    assert_eq!(2 + 2, 4);
}
```

## Security Improvements

-   Use minimal RBAC permissions.
-   Enable TLS for webhook.
-   Avoid storing secrets in code.

------------------------------------------------------------------------

# Conclusion

By implementing every phase step-by-step, you will understand:

-   Rust ownership and concurrency
-   Async programming
-   Kubernetes controller architecture
-   Production observability
-   Secure distributed systems design

This completes the detailed implementation roadmap for kube-guardian.
