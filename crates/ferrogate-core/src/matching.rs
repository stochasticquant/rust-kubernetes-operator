// SPDX-License-Identifier: Apache-2.0

//! Which resources a policy applies to.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::CompileError;
use crate::selector::{LabelSelector, SelectorOperator};

/// The label Kubernetes sets on every namespace with its own name.
pub const NAMESPACE_NAME_LABEL: &str = "kubernetes.io/metadata.name";

/// Which resources a policy applies to. All set conditions must hold.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MatchSpec {
    /// Resource kinds to check. At least one entry is required, so a policy never applies to
    /// every kind in the cluster by accident.
    #[schemars(length(min = 1, max = 32))]
    pub resources: Vec<ResourceFilter>,

    /// Only these namespaces. Empty means all. Cluster-scoped resources never match a non-empty list.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(length(max = 256))]
    pub namespaces: Vec<String>,

    /// Only namespaces whose labels match. Cluster-scoped resources (other than Namespaces
    /// themselves) never match when this is set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace_selector: Option<LabelSelector>,

    /// Only resources whose own labels match.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object_selector: Option<LabelSelector>,
}

/// A set of kinds, optionally limited to API groups.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResourceFilter {
    /// API groups, for example `""` (core) or `"apps"`. Empty or `"*"` means any group.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(length(max = 32))]
    pub api_groups: Vec<String>,

    /// Kinds, for example `Pod` or `Deployment`. `"*"` means any kind.
    #[schemars(length(min = 1, max = 64))]
    pub kinds: Vec<String>,
}

/// Resources and requesters a policy skips, even if they match.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExcludeSpec {
    /// Namespaces to skip.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(length(max = 256))]
    pub namespaces: Vec<String>,

    /// Skip resources whose labels match this selector.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object_selector: Option<LabelSelector>,

    /// Skip requests from these users (admission only; background scans have no requester).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(length(max = 256))]
    pub usernames: Vec<String>,

    /// Skip requests from members of these groups (admission only).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(length(max = 256))]
    pub groups: Vec<String>,
}

/// Who made an admission request.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Requester {
    pub username: String,
    pub groups: Vec<String>,
}

/// A resource to evaluate, with the context needed for matching.
#[derive(Clone, Copy, Debug)]
pub struct Resource<'a> {
    /// API group, `""` for the core group.
    pub group: &'a str,
    pub kind: &'a str,
    /// `None` for cluster-scoped resources.
    pub namespace: Option<&'a str>,
    /// Labels of the resource's namespace, when known.
    pub namespace_labels: Option<&'a BTreeMap<String, String>>,
    /// Who made the request; `None` in background scans.
    pub requester: Option<&'a Requester>,
    pub object: &'a Value,
}

impl Resource<'_> {
    fn labels(&self) -> BTreeMap<String, String> {
        self.object
            .get("metadata")
            .and_then(|m| m.get("labels"))
            .and_then(Value::as_object)
            .map(|labels| {
                labels
                    .iter()
                    .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_owned())))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Namespace labels for selector matching. A Namespace object is matched on its own labels.
    /// If the labels are unknown, Kubernetes' automatic name label is still known.
    fn effective_namespace_labels(&self) -> Option<BTreeMap<String, String>> {
        if self.group.is_empty() && self.kind == "Namespace" {
            return Some(self.labels());
        }
        let namespace = self.namespace?;
        Some(self.namespace_labels.cloned().unwrap_or_else(|| {
            BTreeMap::from([(NAMESPACE_NAME_LABEL.to_owned(), namespace.to_owned())])
        }))
    }
}

fn any_or(list: &[String], value: &str) -> bool {
    list.is_empty() || list.iter().any(|item| item == "*" || item == value)
}

impl ResourceFilter {
    fn matches(&self, group: &str, kind: &str) -> bool {
        any_or(&self.api_groups, group) && self.kinds.iter().any(|k| k == "*" || k == kind)
    }
}

impl MatchSpec {
    pub(crate) fn validate(&self) -> Result<(), CompileError> {
        if self.resources.is_empty() {
            return Err(CompileError::NoResources);
        }
        if let Some(index) = self.resources.iter().position(|r| r.kinds.is_empty()) {
            return Err(CompileError::NoKinds(index));
        }
        validate_selector("match.namespaceSelector", self.namespace_selector.as_ref())?;
        validate_selector("match.objectSelector", self.object_selector.as_ref())
    }

    /// True if the policy's kinds include this group and kind (ignoring namespaces and labels).
    pub fn matches_kind(&self, group: &str, kind: &str) -> bool {
        self.resources.iter().any(|r| r.matches(group, kind))
    }

    pub fn matches(&self, resource: &Resource<'_>) -> bool {
        if !self.matches_kind(resource.group, resource.kind) {
            return false;
        }
        if !self.namespaces.is_empty()
            && !resource
                .namespace
                .is_some_and(|ns| self.namespaces.iter().any(|n| n == ns))
        {
            return false;
        }
        if let Some(selector) = &self.namespace_selector {
            match resource.effective_namespace_labels() {
                Some(labels) if selector.matches(&labels) => {}
                _ => return false,
            }
        }
        if let Some(selector) = &self.object_selector
            && !selector.matches(&resource.labels())
        {
            return false;
        }
        true
    }
}

impl ExcludeSpec {
    pub(crate) fn validate(&self) -> Result<(), CompileError> {
        validate_selector("exclude.objectSelector", self.object_selector.as_ref())
    }

    pub fn excludes(&self, resource: &Resource<'_>) -> bool {
        if resource
            .namespace
            .is_some_and(|ns| self.namespaces.iter().any(|n| n == ns))
        {
            return true;
        }
        if let Some(selector) = &self.object_selector
            && !selector.is_empty()
            && selector.matches(&resource.labels())
        {
            return true;
        }
        if let Some(requester) = resource.requester
            && (self.usernames.contains(&requester.username)
                || requester.groups.iter().any(|g| self.groups.contains(g)))
        {
            return true;
        }
        false
    }
}

fn validate_selector(field: &str, selector: Option<&LabelSelector>) -> Result<(), CompileError> {
    for expr in selector.map_or(&[][..], |s| s.match_expressions.as_slice()) {
        let needs_values = matches!(
            expr.operator,
            SelectorOperator::In | SelectorOperator::NotIn
        );
        let reason = match (needs_values, expr.values.is_empty()) {
            (true, true) => "needs at least one value",
            (false, false) => "must not have values",
            _ => continue,
        };
        return Err(CompileError::InvalidSelector {
            field: field.to_owned(),
            operator: format!("{:?}", expr.operator),
            reason: reason.to_owned(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selector::LabelSelectorRequirement;
    use serde_json::json;

    fn kinds(groups: &[&str], kinds: &[&str]) -> MatchSpec {
        MatchSpec {
            resources: vec![ResourceFilter {
                api_groups: groups.iter().map(ToString::to_string).collect(),
                kinds: kinds.iter().map(ToString::to_string).collect(),
            }],
            ..MatchSpec::default()
        }
    }

    fn resource<'a>(
        group: &'a str,
        kind: &'a str,
        ns: Option<&'a str>,
        object: &'a Value,
    ) -> Resource<'a> {
        Resource {
            group,
            kind,
            namespace: ns,
            namespace_labels: None,
            requester: None,
            object,
        }
    }

    #[test]
    fn kinds_and_groups() {
        let object = json!({});
        let spec = kinds(&["apps"], &["Deployment"]);
        assert!(spec.matches(&resource("apps", "Deployment", Some("a"), &object)));
        assert!(!spec.matches(&resource("", "Pod", Some("a"), &object)));
        assert!(!spec.matches(&resource("extensions", "Deployment", Some("a"), &object)));

        let any_group = kinds(&[], &["Deployment"]);
        assert!(any_group.matches(&resource("extensions", "Deployment", None, &object)));
        assert!(kinds(&["*"], &["*"]).matches(&resource("x", "Y", None, &object)));
    }

    #[test]
    fn namespace_list_excludes_cluster_scoped() {
        let object = json!({});
        let mut spec = kinds(&[], &["*"]);
        spec.namespaces = vec!["prod".into()];
        assert!(spec.matches(&resource("", "Pod", Some("prod"), &object)));
        assert!(!spec.matches(&resource("", "Pod", Some("dev"), &object)));
        assert!(!spec.matches(&resource("", "Node", None, &object)));
    }

    #[test]
    fn namespace_selector_uses_name_label_when_labels_unknown() {
        let object = json!({});
        let mut spec = kinds(&[], &["Pod"]);
        spec.namespace_selector = Some(LabelSelector {
            match_labels: BTreeMap::new(),
            match_expressions: vec![LabelSelectorRequirement {
                key: NAMESPACE_NAME_LABEL.into(),
                operator: SelectorOperator::NotIn,
                values: vec!["kube-system".into()],
            }],
        });
        assert!(spec.matches(&resource("", "Pod", Some("default"), &object)));
        assert!(!spec.matches(&resource("", "Pod", Some("kube-system"), &object)));
    }

    #[test]
    fn namespace_selector_with_known_labels_and_namespace_objects() {
        let mut spec = kinds(&[], &["Pod", "Namespace"]);
        spec.namespace_selector = Some(LabelSelector {
            match_labels: BTreeMap::from([("env".into(), "prod".into())]),
            match_expressions: vec![],
        });
        let labels = BTreeMap::from([("env".to_owned(), "prod".to_owned())]);
        let object = json!({});
        let mut pod = resource("", "Pod", Some("x"), &object);
        pod.namespace_labels = Some(&labels);
        assert!(spec.matches(&pod));

        let ns_object = json!({"metadata": {"labels": {"env": "prod"}}});
        assert!(spec.matches(&resource("", "Namespace", None, &ns_object)));
        assert!(!spec.matches(&resource("", "Namespace", None, &object)));
    }

    #[test]
    fn object_selector() {
        let mut spec = kinds(&[], &["Pod"]);
        spec.object_selector = Some(LabelSelector {
            match_labels: BTreeMap::from([("app".into(), "web".into())]),
            match_expressions: vec![],
        });
        let web = json!({"metadata": {"labels": {"app": "web"}}});
        let db = json!({"metadata": {"labels": {"app": "db", "n": 1}}});
        assert!(spec.matches(&resource("", "Pod", Some("a"), &web)));
        assert!(!spec.matches(&resource("", "Pod", Some("a"), &db)));
    }

    #[test]
    fn exclusions() {
        let exclude = ExcludeSpec {
            namespaces: vec!["kube-system".into()],
            usernames: vec!["system:serviceaccount:ci:deployer".into()],
            groups: vec!["system:masters".into()],
            object_selector: Some(LabelSelector {
                match_labels: BTreeMap::from([("ferrogate.io/skip".into(), "true".into())]),
                match_expressions: vec![],
            }),
        };
        let object = json!({});
        assert!(exclude.excludes(&resource("", "Pod", Some("kube-system"), &object)));
        assert!(!exclude.excludes(&resource("", "Pod", Some("default"), &object)));

        let skipped = json!({"metadata": {"labels": {"ferrogate.io/skip": "true"}}});
        assert!(exclude.excludes(&resource("", "Pod", Some("default"), &skipped)));

        let admin = Requester {
            username: "alice".into(),
            groups: vec!["system:masters".into()],
        };
        let mut request = resource("", "Pod", Some("default"), &object);
        request.requester = Some(&admin);
        assert!(exclude.excludes(&request));
    }

    #[test]
    fn empty_exclude_selector_excludes_nothing() {
        let exclude = ExcludeSpec {
            object_selector: Some(LabelSelector::default()),
            ..ExcludeSpec::default()
        };
        assert!(!exclude.excludes(&resource("", "Pod", Some("a"), &json!({}))));
    }

    #[test]
    fn validation() {
        assert_eq!(
            MatchSpec::default().validate(),
            Err(CompileError::NoResources)
        );
        assert_eq!(kinds(&[], &[]).validate(), Err(CompileError::NoKinds(0)));
        let mut spec = kinds(&[], &["Pod"]);
        spec.object_selector = Some(LabelSelector {
            match_labels: BTreeMap::new(),
            match_expressions: vec![LabelSelectorRequirement {
                key: "a".into(),
                operator: SelectorOperator::In,
                values: vec![],
            }],
        });
        assert!(spec.validate().is_err());
    }
}
