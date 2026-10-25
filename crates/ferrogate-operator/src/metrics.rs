// SPDX-License-Identifier: Apache-2.0

//! Prometheus metrics for the controller.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, PoisonError};

use prometheus_client::encoding::EncodeLabelSet;
use prometheus_client::metrics::counter::Counter;
use prometheus_client::metrics::family::Family;
use prometheus_client::metrics::gauge::Gauge;
use prometheus_client::metrics::histogram::{Histogram, exponential_buckets};
use prometheus_client::registry::Registry;

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub(crate) struct PolicyLabels {
    pub policy: String,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub(crate) struct ResultLabels {
    pub result: &'static str,
}

/// Controller metrics. Cheap to clone; clones share the same series.
#[derive(Clone)]
pub struct OperatorMetrics {
    pub(crate) reconciles: Family<ResultLabels, Counter>,
    pub(crate) reconcile_duration: Histogram,
    pub(crate) ready: Family<PolicyLabels, Gauge>,
    pub(crate) violations: Family<PolicyLabels, Gauge>,
    pub(crate) scanned: Family<PolicyLabels, Gauge>,
    known: Arc<Mutex<BTreeSet<String>>>,
}

impl OperatorMetrics {
    pub fn register(registry: &mut Registry) -> Self {
        let metrics = Self {
            reconciles: Family::default(),
            // 10 ms .. ~80 s: reconciles include background scans.
            reconcile_duration: Histogram::new(exponential_buckets(0.01, 2.0, 14)),
            ready: Family::default(),
            violations: Family::default(),
            scanned: Family::default(),
            known: Arc::default(),
        };
        registry.register(
            "ferrogate_reconciles",
            "Policy reconciles by result (ok, invalid, error)",
            metrics.reconciles.clone(),
        );
        registry.register(
            "ferrogate_reconcile_duration_seconds",
            "Time to reconcile a policy, including its background scan",
            metrics.reconcile_duration.clone(),
        );
        registry.register(
            "ferrogate_policy_ready",
            "1 if the policy is valid and active, 0 otherwise",
            metrics.ready.clone(),
        );
        registry.register(
            "ferrogate_policy_violations",
            "Existing resources failing the policy, from the last background scan",
            metrics.violations.clone(),
        );
        registry.register(
            "ferrogate_policy_scanned_resources",
            "Existing resources evaluated by the last background scan",
            metrics.scanned.clone(),
        );
        metrics
    }

    /// Remembers that `policy` has series, so they can be removed when it is deleted.
    pub(crate) fn track(&self, policy: &str) {
        self.known
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(policy.to_owned());
    }

    pub(crate) fn known_policies(&self) -> Vec<String> {
        self.known
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .cloned()
            .collect()
    }

    pub(crate) fn forget(&self, policy: &str) {
        self.known
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(policy);
        let labels = PolicyLabels {
            policy: policy.to_owned(),
        };
        self.ready.remove(&labels);
        self.violations.remove(&labels);
        self.scanned.remove(&labels);
    }
}
