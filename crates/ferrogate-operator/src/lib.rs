// SPDX-License-Identifier: Apache-2.0

//! Ferrogate controller.
//!
//! Watches `ClusterGatePolicy` resources and reconciles them. Phase A only logs each policy;
//! compiling policies and writing status arrive in Phase B (see `docs/ENTERPRISE_PLAN.md`).

use std::sync::Arc;
use std::time::Duration;

use ferrogate_crd::ClusterGatePolicy;
use futures::StreamExt;
use kube::runtime::controller::{Action, Controller};
use kube::runtime::watcher;
use kube::{Api, Client, ResourceExt};

/// How often a healthy policy is checked again even without changes.
const RESYNC: Duration = Duration::from_secs(300);
/// How long to wait before retrying after an error.
const RETRY: Duration = Duration::from_secs(30);

/// Errors from reconciling a policy.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Kubernetes API error: {0}")]
    Kube(#[from] kube::Error),
}

/// Shared state passed to every reconcile call.
pub struct Context {
    pub client: Client,
}

/// Runs the controller until the process receives SIGTERM or Ctrl-C.
pub async fn run(client: Client) {
    let policies: Api<ClusterGatePolicy> = Api::all(client.clone());
    let context = Arc::new(Context { client });

    Controller::new(policies, watcher::Config::default())
        .shutdown_on_signal()
        .run(reconcile, error_policy, context)
        .for_each(|result| async move {
            match result {
                Ok((object, _)) => tracing::debug!(policy = %object.name, "reconciled"),
                Err(error) => tracing::warn!(%error, "reconcile failed"),
            }
        })
        .await;

    tracing::info!("controller stopped");
}

async fn reconcile(policy: Arc<ClusterGatePolicy>, _ctx: Arc<Context>) -> Result<Action, Error> {
    tracing::info!(
        policy = %policy.name_any(),
        mode = ?policy.spec.mode,
        severity = ?policy.spec.severity,
        "reconciling policy"
    );
    Ok(Action::requeue(RESYNC))
}

// The signature is fixed by `Controller::run`.
#[allow(clippy::needless_pass_by_value)]
fn error_policy(policy: Arc<ClusterGatePolicy>, error: &Error, _ctx: Arc<Context>) -> Action {
    tracing::warn!(policy = %policy.name_any(), %error, "will retry");
    Action::requeue(RETRY)
}
