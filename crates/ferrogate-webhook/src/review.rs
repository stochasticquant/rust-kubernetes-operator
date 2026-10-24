// SPDX-License-Identifier: Apache-2.0

//! `admission.k8s.io/v1` `AdmissionReview` types and the admission decision.
//!
//! Only the fields Ferrogate needs are modelled; unknown fields are ignored.

use std::collections::BTreeMap;

use ferrogate_core::{Decision, Policy, PolicyResult, Requester, Resource, decide};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const API_VERSION: &str = "admission.k8s.io/v1";
pub const KIND: &str = "AdmissionReview";

/// Kubernetes truncates longer warnings; keep each one readable.
const MAX_WARNING_CHARS: usize = 256;
/// Upper bound for the denial message returned to the client.
const MAX_MESSAGE_CHARS: usize = 4096;
/// Upper bound for one audit annotation value.
const MAX_AUDIT_CHARS: usize = 1024;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdmissionReview {
    pub api_version: String,
    pub kind: String,
    pub request: Option<AdmissionRequest>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdmissionRequest {
    pub uid: String,
    pub kind: GroupVersionKind,
    #[serde(default)]
    pub namespace: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    pub operation: String,
    #[serde(default)]
    pub sub_resource: Option<String>,
    #[serde(default)]
    pub user_info: UserInfo,
    #[serde(default)]
    pub object: Option<Value>,
    #[serde(default)]
    pub dry_run: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
pub struct GroupVersionKind {
    #[serde(default)]
    pub group: String,
    #[serde(default)]
    pub version: String,
    pub kind: String,
}

#[derive(Debug, Default, Deserialize)]
pub struct UserInfo {
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub groups: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdmissionReviewResponse {
    pub api_version: &'static str,
    pub kind: &'static str,
    pub response: AdmissionResponse,
}

#[derive(Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdmissionResponse {
    pub uid: String,
    pub allowed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<Status>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub audit_annotations: BTreeMap<String, String>,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct Status {
    pub code: u16,
    pub reason: &'static str,
    pub message: String,
}

impl AdmissionResponse {
    pub fn wrap(self) -> AdmissionReviewResponse {
        AdmissionReviewResponse {
            api_version: API_VERSION,
            kind: KIND,
            response: self,
        }
    }
}

/// What happened to a request, for logs and metrics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Nothing to evaluate (delete, subresource, no matching policy).
    Skipped,
    Allowed,
    Warned,
    Denied,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Skipped => "skipped",
            Self::Allowed => "allowed",
            Self::Warned => "warned",
            Self::Denied => "denied",
        }
    }
}

/// Decides one admission request. Pure: no I/O, so it is easy to test.
pub fn admit(
    request: &AdmissionRequest,
    policies: &[Policy],
    namespace_labels: Option<&BTreeMap<String, String>>,
) -> (AdmissionResponse, Outcome, Decision) {
    let allow = AdmissionResponse {
        uid: request.uid.clone(),
        allowed: true,
        ..AdmissionResponse::default()
    };

    // Deletes carry no new object, and subresources (status, scale, exec, ...) are partial
    // objects that policies are not written against.
    let object = match (&request.object, request.operation.as_str()) {
        (Some(object), "CREATE" | "UPDATE") if request.sub_resource.is_none() => object,
        _ => return (allow, Outcome::Skipped, Decision::default()),
    };

    let requester = Requester {
        username: request.user_info.username.clone(),
        groups: request.user_info.groups.clone(),
    };
    let namespace = request.namespace.as_deref().filter(|ns| !ns.is_empty());
    let resource = Resource {
        group: &request.kind.group,
        kind: &request.kind.kind,
        namespace,
        namespace_labels,
        requester: Some(&requester),
        object,
    };
    let decision = decide(policies, &resource);
    if decision.passed == 0 && decision.failures().next().is_none() {
        return (allow, Outcome::Skipped, decision);
    }

    let mut response = allow;
    response.warnings = decision
        .warned
        .iter()
        .map(|r| truncate(&r.summary(), MAX_WARNING_CHARS))
        .collect();
    // Audit-mode findings go to the API server audit log, never to the client.
    response.audit_annotations = decision
        .audited
        .iter()
        .map(|r| (r.policy.clone(), truncate(&r.summary(), MAX_AUDIT_CHARS)))
        .collect();

    let outcome = if !decision.allowed() {
        response.allowed = false;
        response.status = Some(Status {
            code: 403,
            reason: "Forbidden",
            message: truncate(&denial_message(&decision.denied), MAX_MESSAGE_CHARS),
        });
        Outcome::Denied
    } else if response.warnings.is_empty() {
        Outcome::Allowed
    } else {
        Outcome::Warned
    };
    (response, outcome, decision)
}

fn denial_message(denied: &[PolicyResult]) -> String {
    let details: Vec<String> = denied.iter().map(PolicyResult::summary).collect();
    format!("denied by Ferrogate: {}", details.join(" | "))
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferrogate_core::PolicySpec;
    use serde_json::json;

    fn policy(name: &str, mode: &str) -> Policy {
        let spec: PolicySpec = serde_json::from_value(json!({
            "mode": mode,
            "match": {"resources": [{"kinds": ["Pod"]}]},
            "exclude": {"groups": ["system:masters"]},
            "rules": [{"name": "no-latest", "disallowLatestTag": true}]
        }))
        .unwrap();
        Policy::compile(name, &spec).unwrap()
    }

    fn request(operation: &str, image: &str) -> AdmissionRequest {
        serde_json::from_value(json!({
            "uid": "abc",
            "kind": {"group": "", "version": "v1", "kind": "Pod"},
            "namespace": "default",
            "name": "web",
            "operation": operation,
            "userInfo": {"username": "alice", "groups": ["dev"]},
            "object": {"spec": {"containers": [{"name": "web", "image": image}]}}
        }))
        .unwrap()
    }

    #[test]
    fn enforce_denies_with_403_and_message() {
        let (response, outcome, _) = admit(
            &request("CREATE", "nginx"),
            &[policy("tags", "Enforce")],
            None,
        );
        assert_eq!(outcome, Outcome::Denied);
        assert!(!response.allowed);
        assert_eq!(response.uid, "abc");
        let status = response.status.unwrap();
        assert_eq!(status.code, 403);
        assert!(status.message.starts_with("denied by Ferrogate: [tags]"));
    }

    #[test]
    fn warn_allows_with_warning_and_audit_records_annotation() {
        let policies = [policy("warn", "Warn"), policy("audit", "Audit")];
        let (response, outcome, _) = admit(&request("UPDATE", "nginx"), &policies, None);
        assert_eq!(outcome, Outcome::Warned);
        assert!(response.allowed);
        assert_eq!(response.warnings.len(), 1);
        assert!(response.warnings[0].starts_with("[warn]"));
        assert!(response.audit_annotations["audit"].starts_with("[audit]"));
    }

    #[test]
    fn passing_request_is_allowed_cleanly() {
        let (response, outcome, _) = admit(
            &request("CREATE", "nginx:1.27"),
            &[policy("tags", "Enforce")],
            None,
        );
        assert_eq!(outcome, Outcome::Allowed);
        assert!(response.allowed);
        assert!(response.warnings.is_empty() && response.status.is_none());
    }

    #[test]
    fn deletes_subresources_and_unmatched_kinds_are_skipped() {
        let policies = [policy("tags", "Enforce")];
        assert_eq!(
            admit(&request("DELETE", "nginx"), &policies, None).1,
            Outcome::Skipped
        );

        let mut status = request("UPDATE", "nginx");
        status.sub_resource = Some("status".into());
        assert_eq!(admit(&status, &policies, None).1, Outcome::Skipped);

        let mut config_map = request("CREATE", "nginx");
        config_map.kind.kind = "ConfigMap".into();
        assert_eq!(admit(&config_map, &policies, None).1, Outcome::Skipped);
    }

    #[test]
    fn excluded_groups_bypass_enforcement() {
        let mut admin = request("CREATE", "nginx");
        admin.user_info.groups = vec!["system:masters".into()];
        let (response, outcome, _) = admit(&admin, &[policy("tags", "Enforce")], None);
        assert!(response.allowed);
        assert_eq!(outcome, Outcome::Skipped);
    }

    #[test]
    fn response_serializes_like_kubernetes_expects() {
        let (response, _, _) = admit(
            &request("CREATE", "nginx"),
            &[policy("tags", "Enforce")],
            None,
        );
        let json = serde_json::to_value(response.wrap()).unwrap();
        assert_eq!(json["apiVersion"], "admission.k8s.io/v1");
        assert_eq!(json["kind"], "AdmissionReview");
        assert_eq!(json["response"]["allowed"], false);
        assert_eq!(json["response"]["status"]["code"], 403);
        assert!(json["response"].get("warnings").is_none());
    }

    #[test]
    fn long_texts_are_truncated() {
        let long = "x".repeat(10_000);
        assert_eq!(truncate(&long, 256).chars().count(), 256);
        assert_eq!(truncate("short", 256), "short");
    }
}
