// SPDX-License-Identifier: Apache-2.0

//! Leader election with a `coordination.k8s.io/v1` Lease, so several operator replicas can run and
//! exactly one reconciles.
//!
//! The leader renews the Lease every `retry_period`. A standby takes over when the Lease has not
//! been renewed for `lease_duration`. A leader that cannot renew within `renew_deadline` steps
//! down; the caller then stops the process so it can never act alongside a new leader.

use std::time::{Duration, Instant};

use k8s_openapi::api::coordination::v1::{Lease, LeaseSpec};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::MicroTime;
use k8s_openapi::jiff::{SignedDuration, Timestamp};
use kube::api::{ObjectMeta, PostParams};
use kube::{Api, Client};
use tokio::sync::watch;

/// Leader election settings. The defaults match common Kubernetes controllers.
#[derive(Clone, Debug)]
pub struct LeaseConfig {
    pub name: String,
    pub namespace: String,
    /// Unique identity of this replica, usually the pod name.
    pub identity: String,
    pub lease_duration: Duration,
    pub renew_deadline: Duration,
    pub retry_period: Duration,
}

impl LeaseConfig {
    pub fn new(namespace: impl Into<String>, identity: impl Into<String>) -> Self {
        Self {
            name: "ferrogate-operator".into(),
            namespace: namespace.into(),
            identity: identity.into(),
            lease_duration: Duration::from_secs(15),
            renew_deadline: Duration::from_secs(10),
            retry_period: Duration::from_secs(2),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LeaderError {
    #[error("lost leadership of lease {0}")]
    Lost(String),
}

/// What to do with the Lease on this attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Plan {
    /// No Lease exists yet.
    Create,
    /// We hold it: renew.
    Renew,
    /// Free or expired: take it over.
    Take,
    /// Someone else holds a valid Lease.
    Wait,
}

fn plan(
    spec: Option<&LeaseSpec>,
    identity: &str,
    now: Timestamp,
    default_duration: Duration,
) -> Plan {
    let Some(spec) = spec else {
        return Plan::Create;
    };
    match spec.holder_identity.as_deref() {
        Some(holder) if holder == identity => return Plan::Renew,
        None | Some("") => return Plan::Take,
        Some(_) => {}
    }
    let duration = spec
        .lease_duration_seconds
        .and_then(|s| u64::try_from(s).ok())
        .map_or(default_duration, Duration::from_secs);
    let expired = spec.renew_time.as_ref().is_none_or(|renewed| {
        SignedDuration::try_from(duration)
            .ok()
            .and_then(|d| renewed.0.checked_add(d).ok())
            .is_none_or(|expiry| expiry < now)
    });
    if expired { Plan::Take } else { Plan::Wait }
}

fn duration_seconds(duration: Duration) -> i32 {
    i32::try_from(duration.as_secs()).unwrap_or(i32::MAX)
}

/// Tries once to acquire or renew the Lease. Returns whether we hold it afterwards.
async fn try_acquire(api: &Api<Lease>, config: &LeaseConfig) -> Result<bool, kube::Error> {
    let now = Timestamp::now();
    let existing = api.get_opt(&config.name).await?;
    let current = existing.as_ref().and_then(|lease| lease.spec.as_ref());
    let step = plan(current, &config.identity, now, config.lease_duration);
    let mut spec = current.cloned().unwrap_or_default();
    match step {
        Plan::Wait => return Ok(false),
        Plan::Renew => {}
        Plan::Create | Plan::Take => {
            spec.acquire_time = Some(MicroTime(now));
            spec.lease_transitions = Some(
                spec.lease_transitions
                    .unwrap_or(0)
                    .saturating_add(i32::from(step == Plan::Take)),
            );
        }
    }
    spec.holder_identity = Some(config.identity.clone());
    spec.lease_duration_seconds = Some(duration_seconds(config.lease_duration));
    spec.renew_time = Some(MicroTime(now));

    let result = match existing {
        None => {
            let lease = Lease {
                metadata: ObjectMeta {
                    name: Some(config.name.clone()),
                    namespace: Some(config.namespace.clone()),
                    ..ObjectMeta::default()
                },
                spec: Some(spec),
            };
            api.create(&PostParams::default(), &lease).await.map(|_| ())
        }
        Some(mut lease) => {
            // `replace` sends the resourceVersion we read, so concurrent writers conflict (409).
            lease.spec = Some(spec);
            api.replace(&config.name, &PostParams::default(), &lease)
                .await
                .map(|_| ())
        }
    };
    match result {
        Ok(()) => Ok(true),
        Err(kube::Error::Api(status)) if status.code == 409 => Ok(false),
        Err(error) => Err(error),
    }
}

/// Runs the election until leadership is lost. Publishes `true` on `leader` once elected.
///
/// Returns only with [`LeaderError::Lost`]; while not elected it keeps retrying.
pub async fn run(client: Client, config: LeaseConfig, leader: watch::Sender<bool>) -> LeaderError {
    let api: Api<Lease> = Api::namespaced(client, &config.namespace);
    let mut leading = false;
    let mut last_renewal = Instant::now();
    loop {
        match try_acquire(&api, &config).await {
            Ok(true) => {
                last_renewal = Instant::now();
                if !leading {
                    leading = true;
                    tracing::info!(lease = %config.name, identity = %config.identity, "became leader");
                    let _ = leader.send(true);
                }
            }
            Ok(false) if leading => {
                let _ = leader.send(false);
                return LeaderError::Lost(config.name);
            }
            Ok(false) => tracing::debug!(lease = %config.name, "another replica is leader"),
            Err(error) => {
                tracing::warn!(%error, lease = %config.name, "leader election request failed");
                if leading && last_renewal.elapsed() > config.renew_deadline {
                    let _ = leader.send(false);
                    return LeaderError::Lost(config.name);
                }
            }
        }
        tokio::time::sleep(config.retry_period).await;
    }
}

/// Gives the Lease up so a standby can take over at once. Best effort.
pub async fn release(client: Client, config: &LeaseConfig) {
    let api: Api<Lease> = Api::namespaced(client, &config.namespace);
    let Ok(Some(mut lease)) = api.get_opt(&config.name).await else {
        return;
    };
    let Some(spec) = lease.spec.as_mut() else {
        return;
    };
    if spec.holder_identity.as_deref() != Some(config.identity.as_str()) {
        return;
    }
    spec.holder_identity = None;
    spec.renew_time = None;
    match api
        .replace(&config.name, &PostParams::default(), &lease)
        .await
    {
        Ok(_) => tracing::info!(lease = %config.name, "released leadership"),
        Err(error) => tracing::debug!(%error, "could not release lease"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DURATION: Duration = Duration::from_secs(15);

    fn lease(holder: Option<&str>, renewed_secs_ago: Option<i64>, now: Timestamp) -> LeaseSpec {
        LeaseSpec {
            holder_identity: holder.map(ToOwned::to_owned),
            lease_duration_seconds: Some(15),
            renew_time: renewed_secs_ago
                .map(|ago| MicroTime(now.checked_sub(SignedDuration::from_secs(ago)).unwrap())),
            ..LeaseSpec::default()
        }
    }

    #[test]
    fn plans() {
        let now = Timestamp::now();
        assert_eq!(plan(None, "me", now, DURATION), Plan::Create);
        assert_eq!(
            plan(Some(&lease(Some("me"), Some(1), now)), "me", now, DURATION),
            Plan::Renew
        );
        assert_eq!(
            plan(
                Some(&lease(Some("other"), Some(1), now)),
                "me",
                now,
                DURATION
            ),
            Plan::Wait
        );
        assert_eq!(
            plan(
                Some(&lease(Some("other"), Some(20), now)),
                "me",
                now,
                DURATION
            ),
            Plan::Take
        );
        assert_eq!(
            plan(Some(&lease(None, None, now)), "me", now, DURATION),
            Plan::Take
        );
        assert_eq!(
            plan(Some(&lease(Some(""), Some(1), now)), "me", now, DURATION),
            Plan::Take
        );
        assert_eq!(
            plan(Some(&lease(Some("other"), None, now)), "me", now, DURATION),
            Plan::Take
        );
    }

    #[test]
    fn holder_duration_wins_over_ours() {
        let now = Timestamp::now();
        let mut long = lease(Some("other"), Some(20), now);
        long.lease_duration_seconds = Some(60);
        assert_eq!(plan(Some(&long), "me", now, DURATION), Plan::Wait);
    }
}
