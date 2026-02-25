use kube::runtime::controller::Action;
use kube::ResourceExt;
use std::sync::Arc;

use crate::crd::GuardianPolicy;

pub async fn reconcile(
    policy: Arc<GuardianPolicy>,
    _ctx: Arc<()>,
) -> Result<Action, kube::Error> {
    println!("Reconciling: {}", policy.name_any());
    Ok(Action::requeue(std::time::Duration::from_secs(300)))
}

pub fn error_policy(
    _obj: Arc<GuardianPolicy>,
    _error: &kube::Error,
    _ctx: Arc<()>,
) -> Action {
    Action::requeue(std::time::Duration::from_secs(60))
}
