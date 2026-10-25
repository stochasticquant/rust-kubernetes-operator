// SPDX-License-Identifier: Apache-2.0

//! Ferrogate custom resource definitions.
//!
//! API group: `ferrogate.stochasticquant.io`. Version `v1alpha1`: fields may still change between
//! releases. See `docs/decisions/0003-api-group-and-versioning.md`.

use ferrogate_core::{CompileError, Enforcement, Policy, PolicySpec, Severity};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::Condition;
use kube::{CustomResource, ResourceExt};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// API group shared by all Ferrogate resources.
pub const GROUP: &str = "ferrogate.stochasticquant.io";
/// API version of the current resources.
pub const VERSION: &str = "v1alpha1";
/// Maximum number of sample results kept in a policy's status.
pub const MAX_STATUS_RESULTS: usize = 20;

/// A cluster-wide policy that Ferrogate audits or enforces.
#[derive(CustomResource, Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[kube(
    group = "ferrogate.stochasticquant.io",
    version = "v1alpha1",
    kind = "ClusterGatePolicy",
    shortname = "cgp",
    category = "ferrogate",
    status = "ClusterGatePolicyStatus",
    printcolumn = r#"{"name":"Mode","type":"string","jsonPath":".spec.mode"}"#,
    printcolumn = r#"{"name":"Severity","type":"string","jsonPath":".spec.severity"}"#,
    printcolumn = r#"{"name":"Enforcement","type":"string","jsonPath":".spec.enforcement","priority":1}"#,
    printcolumn = r#"{"name":"Ready","type":"string","jsonPath":".status.conditions[?(@.type==\"Ready\")].status"}"#,
    printcolumn = r#"{"name":"Violations","type":"integer","jsonPath":".status.violations"}"#,
    printcolumn = r#"{"name":"Age","type":"date","jsonPath":".metadata.creationTimestamp"}"#
)]
pub struct ClusterGatePolicySpec {
    #[serde(flatten)]
    pub policy: PolicySpec,
}

/// Observed state of a policy, written by the operator.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ClusterGatePolicyStatus {
    /// The `metadata.generation` this status describes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_generation: Option<i64>,

    /// Standard conditions. `Ready` is `True` when the policy is valid and active.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conditions: Vec<Condition>,

    /// Who enforces the policy at admission: `Webhook`, or `Native` through the generated
    /// `ValidatingAdmissionPolicy` named in the `Ready` condition.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enforcement: Option<Enforcement>,

    /// Number of existing resources that fail the policy, from the last background scan.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub violations: Option<i64>,

    /// Number of existing resources the last scan evaluated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scanned: Option<i64>,

    /// When the last background scan finished (RFC 3339).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_scan_time: Option<String>,

    /// Up to 20 failing resources from the last scan, for quick inspection.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub results: Vec<ScanResult>,
}

/// One failing resource found by a background scan.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ScanResult {
    /// `group/version/Kind`, for example `apps/v1/Deployment`.
    pub api_version_kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    pub name: String,
    pub rule: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

impl ClusterGatePolicy {
    /// Validates the policy and turns it into an evaluable [`Policy`].
    pub fn compile(&self) -> Result<Policy, CompileError> {
        Policy::compile(&self.name_any(), &self.spec.policy)
    }

    pub fn severity(&self) -> Severity {
        self.spec.policy.severity
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferrogate_core::Mode;
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
        assert_eq!(crd.spec.versions[0].name, VERSION);
    }

    #[test]
    fn spec_fields_are_top_level_and_mode_defaults_to_audit() {
        let policy: ClusterGatePolicy = serde_json::from_value(serde_json::json!({
            "apiVersion": "ferrogate.stochasticquant.io/v1alpha1",
            "kind": "ClusterGatePolicy",
            "metadata": {"name": "p"},
            "spec": {
                "match": {"resources": [{"kinds": ["Pod"]}]},
                "rules": [{"name": "no-latest", "disallowLatestTag": true}]
            }
        }))
        .unwrap();
        assert_eq!(policy.spec.policy.mode, Mode::Audit);
        let compiled = policy.compile().unwrap();
        assert_eq!(compiled.name, "p");
        assert_eq!(compiled.rules.len(), 1);
    }

    #[test]
    fn schema_is_structural_and_validates_rules() {
        let crd = serde_json::to_value(ClusterGatePolicy::crd()).unwrap();
        let spec = &crd["spec"]["versions"][0]["schema"]["openAPIV3Schema"]["properties"]["spec"];
        let required: Vec<&str> = spec["required"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        assert!(required.contains(&"match") && required.contains(&"rules"));

        let rule = &spec["properties"]["rules"]["items"];
        let validation = rule["x-kubernetes-validations"][0]["rule"]
            .as_str()
            .unwrap();
        assert!(validation.contains("has(self.disallowPrivileged)"));
        assert_eq!(spec["properties"]["rules"]["maxItems"], 64);
        assert_eq!(spec["properties"]["mode"]["default"], "Audit");
    }
}
