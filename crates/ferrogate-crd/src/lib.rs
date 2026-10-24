// SPDX-License-Identifier: Apache-2.0

//! Ferrogate custom resource definitions.
//!
//! API group: `ferrogate.stochasticquant.io`. Version `v1alpha1`: fields may still change between
//! releases. See `docs/decisions/0003-api-group-and-versioning.md`.

use ferrogate_core::{Mode, Severity};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::Condition;
use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// API group shared by all Ferrogate resources.
pub const GROUP: &str = "ferrogate.stochasticquant.io";

/// A cluster-wide policy that Ferrogate audits or enforces.
#[derive(CustomResource, Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[kube(
    group = "ferrogate.stochasticquant.io",
    version = "v1alpha1",
    kind = "ClusterGatePolicy",
    shortname = "cgp",
    status = "ClusterGatePolicyStatus",
    printcolumn = r#"{"name":"Mode","type":"string","jsonPath":".spec.mode"}"#,
    printcolumn = r#"{"name":"Severity","type":"string","jsonPath":".spec.severity"}"#,
    printcolumn = r#"{"name":"Age","type":"date","jsonPath":".metadata.creationTimestamp"}"#
)]
#[serde(rename_all = "camelCase")]
pub struct ClusterGatePolicySpec {
    /// `Audit` (default) only reports, `Warn` allows with a warning, `Enforce` denies.
    #[serde(default)]
    pub mode: Mode,

    /// How serious a violation of this policy is.
    #[serde(default)]
    pub severity: Severity,

    /// Labels every matched resource must carry.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub require_labels: Vec<String>,
}

/// Observed state of a `ClusterGatePolicy`, written by the operator.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ClusterGatePolicyStatus {
    /// The `metadata.generation` this status describes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_generation: Option<i64>,

    /// Standard Kubernetes conditions, for example `Ready`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conditions: Vec<Condition>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use kube::CustomResourceExt;

    #[test]
    fn crd_identity() {
        let crd = ClusterGatePolicy::crd();
        assert_eq!(
            crd.metadata.name.as_deref(),
            Some("clustergatepolicies.ferrogate.stochasticquant.io")
        );
        assert_eq!(crd.spec.group, GROUP);
        assert_eq!(crd.spec.scope, "Cluster");
        assert_eq!(crd.spec.versions[0].name, "v1alpha1");
    }

    #[test]
    fn mode_defaults_to_audit() {
        let spec: ClusterGatePolicySpec = serde_json::from_str("{}").unwrap();
        assert_eq!(spec.mode, Mode::Audit);
    }
}
