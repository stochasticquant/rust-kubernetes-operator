// SPDX-License-Identifier: Apache-2.0

//! Ferrogate controller.
//!
//! For each `ClusterGatePolicy` the controller validates the policy, scans existing resources in
//! the background, and writes the result to the policy's status, Kubernetes Events and metrics.
//! It also provides the watch caches the admission webhook reads ([`cache::Caches`]).

pub mod cache;
pub mod metrics;
pub mod scan;

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use ferrogate_crd::{ClusterGatePolicy, ClusterGatePolicyStatus};
use futures::StreamExt;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{Condition, Time};
use k8s_openapi::jiff::Timestamp;
use kube::api::{Patch, PatchParams};
use kube::runtime::controller::{Action, Controller};
use kube::runtime::events::{Event, EventType, Recorder, Reporter};
use kube::runtime::reflector::Store;
use kube::runtime::watcher;
use kube::{Api, Client, Resource, ResourceExt};
use serde_json::{Value, json};

pub use metrics::OperatorMetrics;

/// Condition `Ready`: the policy is valid and active.
pub const CONDITION_READY: &str = "Ready";
/// Condition `Scanned`: the last background scan covered every targeted resource.
pub const CONDITION_SCANNED: &str = "Scanned";

/// Errors from reconciling a policy.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Kubernetes API error: {0}")]
    Kube(#[from] kube::Error),
    #[error("serialization error: {0}")]
    Serialize(#[from] serde_json::Error),
}

/// Controller settings.
#[derive(Clone, Debug)]
pub struct Config {
    /// Run background scans of existing resources.
    pub scan: bool,
    /// Time between scans of the same policy.
    pub scan_interval: Duration,
    /// Maximum resources evaluated per policy and scan.
    pub max_scan_objects: usize,
    /// Name reported on Kubernetes Events.
    pub reporter: String,
    /// Instance reported on Kubernetes Events (usually the pod name).
    pub instance: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            scan: true,
            scan_interval: Duration::from_secs(300),
            max_scan_objects: 20_000,
            reporter: "ferrogate".into(),
            instance: None,
        }
    }
}

/// Shared state passed to every reconcile call.
pub struct Context {
    client: Client,
    config: Config,
    recorder: Recorder,
    metrics: OperatorMetrics,
    discovery: scan::DiscoveryCache,
    store: Store<ClusterGatePolicy>,
    failures: Mutex<HashMap<String, u32>>,
}

/// Runs the controller until the process receives SIGTERM or Ctrl-C.
pub async fn run(client: Client, config: Config, metrics: OperatorMetrics) {
    let policies: Api<ClusterGatePolicy> = Api::all(client.clone());
    let controller = Controller::new(policies, watcher::Config::default());
    let recorder = Recorder::new(
        client.clone(),
        Reporter {
            controller: config.reporter.clone(),
            instance: config.instance.clone(),
        },
    );
    let context = Arc::new(Context {
        store: controller.store(),
        client,
        config,
        recorder,
        metrics,
        discovery: scan::DiscoveryCache::default(),
        failures: Mutex::new(HashMap::new()),
    });

    controller
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

async fn reconcile(policy: Arc<ClusterGatePolicy>, ctx: Arc<Context>) -> Result<Action, Error> {
    let started = Instant::now();
    let result = reconcile_inner(&policy, &ctx).await;
    ctx.metrics
        .reconcile_duration
        .observe(started.elapsed().as_secs_f64());
    let label = match &result {
        Ok((_, Outcome::Invalid)) => "invalid",
        Ok(_) => "ok",
        Err(_) => "error",
    };
    ctx.metrics
        .reconciles
        .get_or_create(&metrics::ResultLabels { result: label })
        .inc();
    if result.is_ok() {
        ctx.failures
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&policy.name_any());
    }
    forget_deleted(&ctx);
    result.map(|(action, _)| action)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Unchanged,
    Invalid,
    Active,
}

async fn reconcile_inner(
    policy: &ClusterGatePolicy,
    ctx: &Context,
) -> Result<(Action, Outcome), Error> {
    let name = policy.name_any();
    let generation = policy.meta().generation;
    let previous = policy.status.clone().unwrap_or_default();

    // Writing status triggers another reconcile; skip work that is already up to date.
    if previous.observed_generation == generation && generation.is_some() {
        let invalid = condition_status(&previous, CONDITION_READY) == Some("False");
        if invalid {
            return Ok((Action::await_change(), Outcome::Unchanged));
        }
        if let Some(remaining) = scan_due_in(&previous, &ctx.config) {
            return Ok((Action::requeue(remaining), Outcome::Unchanged));
        }
    }

    let labels = metrics::PolicyLabels {
        policy: name.clone(),
    };
    ctx.metrics.track(&name);
    let mut status = ClusterGatePolicyStatus {
        observed_generation: generation,
        ..ClusterGatePolicyStatus::default()
    };

    let compiled = match policy.compile() {
        Ok(compiled) => compiled,
        Err(error) => {
            let message = error.to_string();
            tracing::warn!(policy = %name, %message, "invalid policy");
            status.conditions = vec![condition(
                CONDITION_READY,
                false,
                "InvalidPolicy",
                &message,
                generation,
                &previous,
            )];
            write_status(ctx, &name, &status).await?;
            ctx.metrics.ready.get_or_create(&labels).set(0);
            if condition_status(&previous, CONDITION_READY) != Some("False") {
                publish(ctx, policy, EventType::Warning, "InvalidPolicy", &message).await;
            }
            return Ok((Action::await_change(), Outcome::Invalid));
        }
    };

    let summary = format!(
        "{} rule(s), mode {}, severity {}",
        compiled.rules.len(),
        compiled.mode.as_str(),
        compiled.severity.as_str()
    );
    status.conditions.push(condition(
        CONDITION_READY,
        true,
        "Active",
        &summary,
        generation,
        &previous,
    ));
    ctx.metrics.ready.get_or_create(&labels).set(1);
    if condition_status(&previous, CONDITION_READY) != Some("True") {
        publish(ctx, policy, EventType::Normal, "Active", &summary).await;
    }

    if ctx.config.scan {
        scan_into_status(ctx, policy, &compiled, &previous, &mut status).await?;
    }

    write_status(ctx, &name, &status).await?;
    Ok((Action::requeue(ctx.config.scan_interval), Outcome::Active))
}

/// Runs a background scan and records the result in `status`, metrics and events.
async fn scan_into_status(
    ctx: &Context,
    policy: &ClusterGatePolicy,
    compiled: &ferrogate_core::Policy,
    previous: &ClusterGatePolicyStatus,
    status: &mut ClusterGatePolicyStatus,
) -> Result<(), Error> {
    let name = policy.name_any();
    let generation = status.observed_generation;
    let labels = metrics::PolicyLabels {
        policy: name.clone(),
    };
    let outcome = run_scan(ctx, compiled).await?;
    let (ok, reason, message) = scan_condition(&outcome);
    status.conditions.push(condition(
        CONDITION_SCANNED,
        ok,
        reason,
        &message,
        generation,
        previous,
    ));
    status.scanned = Some(i64::from(outcome.scanned));
    status.violations = Some(i64::from(outcome.failing));
    status.last_scan_time = Some(Timestamp::now().to_string());
    status.results = outcome.results;
    ctx.metrics
        .violations
        .get_or_create(&labels)
        .set(i64::from(outcome.failing));
    ctx.metrics
        .scanned
        .get_or_create(&labels)
        .set(i64::from(outcome.scanned));
    tracing::info!(
        policy = %name,
        scanned = outcome.scanned,
        failing = outcome.failing,
        problems = outcome.problems.len(),
        "background scan finished"
    );
    if previous.violations != status.violations && outcome.failing > 0 {
        let note = format!(
            "{} of {} resource(s) fail this policy",
            outcome.failing, outcome.scanned
        );
        publish(ctx, policy, EventType::Warning, "ViolationsFound", &note).await;
    }
    Ok(())
}

async fn run_scan(
    ctx: &Context,
    policy: &ferrogate_core::Policy,
) -> Result<scan::ScanOutcome, Error> {
    let available = ctx.discovery.get(&ctx.client).await?;
    let (targets, wildcard) = scan::targets(policy, &available);
    let namespaces = scan::namespace_labels(&ctx.client).await?;
    let mut outcome = scan::scan(
        &ctx.client,
        policy,
        &targets,
        &namespaces,
        ctx.config.max_scan_objects,
    )
    .await;
    outcome.wildcard = wildcard;
    Ok(outcome)
}

fn scan_condition(outcome: &scan::ScanOutcome) -> (bool, &'static str, String) {
    if outcome.wildcard {
        return (
            false,
            "WildcardKinds",
            "kinds \"*\" are checked at admission only and not scanned".into(),
        );
    }
    if !outcome.problems.is_empty() {
        return (
            false,
            "Incomplete",
            format!("could not list: {}", outcome.problems.join("; ")),
        );
    }
    if outcome.truncated {
        return (
            false,
            "Truncated",
            format!("stopped after {} resources", outcome.scanned),
        );
    }
    (
        true,
        "Complete",
        format!(
            "{} of {} resource(s) fail",
            outcome.failing, outcome.scanned
        ),
    )
}

/// Time until the next scan is due, or `None` if it is due now.
fn scan_due_in(status: &ClusterGatePolicyStatus, config: &Config) -> Option<Duration> {
    if !config.scan {
        // Nothing periodic to do; status is up to date.
        return Some(config.scan_interval);
    }
    let last = Timestamp::from_str(status.last_scan_time.as_deref()?).ok()?;
    let elapsed = Timestamp::now().duration_since(last);
    let elapsed = Duration::try_from(elapsed).ok()?;
    config
        .scan_interval
        .checked_sub(elapsed)
        .filter(|left| *left > Duration::from_secs(1))
}

fn condition_status<'a>(status: &'a ClusterGatePolicyStatus, type_: &str) -> Option<&'a str> {
    status
        .conditions
        .iter()
        .find(|c| c.type_ == type_)
        .map(|c| c.status.as_str())
}

/// Builds a condition, keeping the previous transition time if the status did not change.
fn condition(
    type_: &str,
    ok: bool,
    reason: &str,
    message: &str,
    generation: Option<i64>,
    previous: &ClusterGatePolicyStatus,
) -> Condition {
    let status = if ok { "True" } else { "False" };
    let last_transition_time = previous
        .conditions
        .iter()
        .find(|c| c.type_ == type_ && c.status == status)
        .map_or_else(
            || Time(Timestamp::now()),
            |c| c.last_transition_time.clone(),
        );
    Condition {
        type_: type_.into(),
        status: status.into(),
        reason: reason.into(),
        message: message.chars().take(32 * 1024).collect(),
        observed_generation: generation,
        last_transition_time,
    }
}

/// Patches the status subresource. Fields that are unset are removed explicitly.
async fn write_status(
    ctx: &Context,
    name: &str,
    status: &ClusterGatePolicyStatus,
) -> Result<(), Error> {
    let mut value = serde_json::to_value(status)?;
    if let Value::Object(map) = &mut value {
        for field in [
            "conditions",
            "violations",
            "scanned",
            "lastScanTime",
            "results",
        ] {
            map.entry(field).or_insert(Value::Null);
        }
    }
    let api: Api<ClusterGatePolicy> = Api::all(ctx.client.clone());
    api.patch_status(
        name,
        &PatchParams::default(),
        &Patch::Merge(json!({ "status": value })),
    )
    .await?;
    Ok(())
}

async fn publish(
    ctx: &Context,
    policy: &ClusterGatePolicy,
    type_: EventType,
    reason: &str,
    note: &str,
) {
    let event = Event {
        type_,
        reason: reason.into(),
        note: Some(note.chars().take(1024).collect()),
        action: "Reconcile".into(),
        secondary: None,
    };
    if let Err(error) = ctx.recorder.publish(&event, &policy.object_ref(&())).await {
        tracing::debug!(%error, "could not publish event");
    }
}

fn forget_deleted(ctx: &Context) {
    let mut failures = ctx.failures.lock().unwrap_or_else(PoisonError::into_inner);
    let live: Vec<String> = ctx.store.state().iter().map(|p| p.name_any()).collect();
    failures.retain(|name, _| live.contains(name));
    drop(failures);
    // Gauges of deleted policies would otherwise be reported forever.
    for name in ctx.metrics.known_policies() {
        if !live.contains(&name) {
            ctx.metrics.forget(&name);
        }
    }
}

/// Exponential back-off per policy: 5 s, 10 s, 20 s, ... up to 5 minutes.
fn backoff(failures: u32) -> Duration {
    let seconds = 5u64.saturating_mul(1u64 << failures.min(6));
    Duration::from_secs(seconds.min(300))
}

// The signature is fixed by `Controller::run`.
#[allow(clippy::needless_pass_by_value)]
fn error_policy(policy: Arc<ClusterGatePolicy>, error: &Error, ctx: Arc<Context>) -> Action {
    let name = policy.name_any();
    let mut failures = ctx.failures.lock().unwrap_or_else(PoisonError::into_inner);
    let count = failures.entry(name.clone()).or_insert(0);
    let delay = backoff(*count);
    *count = count.saturating_add(1);
    tracing::warn!(policy = %name, %error, retry_in = ?delay, "will retry");
    Action::requeue(delay)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_grows_and_caps() {
        assert_eq!(backoff(0), Duration::from_secs(5));
        assert_eq!(backoff(1), Duration::from_secs(10));
        assert_eq!(backoff(3), Duration::from_secs(40));
        assert_eq!(backoff(10), Duration::from_secs(300));
        assert_eq!(backoff(u32::MAX), Duration::from_secs(300));
    }

    #[test]
    fn condition_keeps_transition_time_when_unchanged() {
        let then = Time(Timestamp::from_second(1_700_000_000).unwrap());
        let previous = ClusterGatePolicyStatus {
            conditions: vec![Condition {
                type_: CONDITION_READY.into(),
                status: "True".into(),
                reason: "Active".into(),
                message: String::new(),
                observed_generation: Some(1),
                last_transition_time: then.clone(),
            }],
            ..ClusterGatePolicyStatus::default()
        };
        let same = condition(CONDITION_READY, true, "Active", "m", Some(2), &previous);
        assert_eq!(same.last_transition_time, then);
        let flipped = condition(
            CONDITION_READY,
            false,
            "InvalidPolicy",
            "m",
            Some(2),
            &previous,
        );
        assert_ne!(flipped.last_transition_time, then);
    }

    #[test]
    fn scan_is_due_when_never_run_or_old() {
        let config = Config::default();
        assert_eq!(
            scan_due_in(&ClusterGatePolicyStatus::default(), &config),
            None
        );

        let fresh = ClusterGatePolicyStatus {
            last_scan_time: Some(Timestamp::now().to_string()),
            ..ClusterGatePolicyStatus::default()
        };
        assert!(scan_due_in(&fresh, &config).is_some_and(|d| d > Duration::from_secs(290)));

        let old = ClusterGatePolicyStatus {
            last_scan_time: Some("2020-01-01T00:00:00Z".into()),
            ..ClusterGatePolicyStatus::default()
        };
        assert_eq!(scan_due_in(&old, &config), None);
    }

    #[test]
    fn scan_conditions() {
        let complete = scan::ScanOutcome {
            scanned: 10,
            failing: 2,
            ..scan::ScanOutcome::default()
        };
        assert_eq!(scan_condition(&complete).1, "Complete");
        let partial = scan::ScanOutcome {
            problems: vec!["Pod: Forbidden (403)".into()],
            ..scan::ScanOutcome::default()
        };
        assert_eq!(scan_condition(&partial).1, "Incomplete");
        let wildcard = scan::ScanOutcome {
            wildcard: true,
            ..scan::ScanOutcome::default()
        };
        assert!(!scan_condition(&wildcard).0);
    }
}
