// SPDX-License-Identifier: Apache-2.0

//! Prometheus metrics for admission requests.

use prometheus_client::encoding::EncodeLabelSet;
use prometheus_client::metrics::counter::Counter;
use prometheus_client::metrics::family::Family;
use prometheus_client::metrics::histogram::{Histogram, exponential_buckets};
use prometheus_client::registry::Registry;

use crate::review::Outcome;
use ferrogate_core::Decision;

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
struct OutcomeLabels {
    outcome: &'static str,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
struct ViolationLabels {
    policy: String,
    mode: &'static str,
    severity: &'static str,
}

fn latency_histogram() -> Histogram {
    // 0.25 ms .. ~2 s
    Histogram::new(exponential_buckets(0.000_25, 2.0, 14))
}

/// Admission metrics. Cheap to clone; clones share the same series.
#[derive(Clone)]
pub struct WebhookMetrics {
    requests: Family<OutcomeLabels, Counter>,
    duration: Family<OutcomeLabels, Histogram, fn() -> Histogram>,
    violations: Family<ViolationLabels, Counter>,
    errors: Counter,
}

impl WebhookMetrics {
    pub fn register(registry: &mut Registry) -> Self {
        let metrics = Self {
            requests: Family::default(),
            duration: Family::new_with_constructor(latency_histogram),
            violations: Family::default(),
            errors: Counter::default(),
        };
        registry.register(
            "ferrogate_admission_requests",
            "Admission requests by outcome (skipped, allowed, warned, denied)",
            metrics.requests.clone(),
        );
        registry.register(
            "ferrogate_admission_duration_seconds",
            "Time to decide an admission request",
            metrics.duration.clone(),
        );
        registry.register(
            "ferrogate_admission_violations",
            "Policy failures seen at admission, by policy, mode and severity",
            metrics.violations.clone(),
        );
        registry.register(
            "ferrogate_admission_errors",
            "Admission requests that could not be decided (bad request or not ready)",
            metrics.errors.clone(),
        );
        metrics
    }

    pub(crate) fn record(&self, outcome: Outcome, seconds: f64, decision: &Decision) {
        let labels = OutcomeLabels {
            outcome: outcome.as_str(),
        };
        self.requests.get_or_create(&labels).inc();
        self.duration.get_or_create(&labels).observe(seconds);
        for result in decision.failures() {
            self.violations
                .get_or_create(&ViolationLabels {
                    policy: result.policy.clone(),
                    mode: result.mode.as_str(),
                    severity: result.severity.as_str(),
                })
                .inc();
        }
    }

    pub(crate) fn error(&self) {
        self.errors.inc();
    }
}
