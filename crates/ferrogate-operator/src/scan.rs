// SPDX-License-Identifier: Apache-2.0

//! Background scan: evaluate existing resources against a policy.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ferrogate_core::{Policy, Resource};
use ferrogate_crd::{MAX_STATUS_RESULTS, ScanResult};
use k8s_openapi::api::core::v1::Namespace;
use kube::api::{ListParams, ObjectList};
use kube::core::{ApiResource, DynamicObject, PartialObjectMeta};
use kube::discovery::{Discovery, Scope, verbs};
use kube::{Api, Client, ResourceExt};
use tokio::sync::RwLock;

use crate::Error;

/// How long discovery results are reused before asking the API server again.
const DISCOVERY_TTL: Duration = Duration::from_secs(600);
/// Page size for list calls.
const PAGE_SIZE: u32 = 500;

/// A listable resource type found by discovery.
#[derive(Clone, Debug)]
pub struct Listable {
    pub resource: ApiResource,
    pub namespaced: bool,
}

/// Cached API discovery.
#[derive(Default)]
pub struct DiscoveryCache {
    inner: RwLock<Option<(Instant, Arc<Vec<Listable>>)>>,
}

impl DiscoveryCache {
    pub async fn get(&self, client: &Client) -> Result<Arc<Vec<Listable>>, Error> {
        if let Some((at, cached)) = self.inner.read().await.as_ref()
            && at.elapsed() < DISCOVERY_TTL
        {
            return Ok(cached.clone());
        }
        let discovery = Discovery::new(client.clone()).run().await?;
        let mut found = Vec::new();
        for group in discovery.groups() {
            for (resource, caps) in group.recommended_resources() {
                if caps.supports_operation(verbs::LIST) && !resource.plural.contains('/') {
                    found.push(Listable {
                        resource,
                        namespaced: caps.scope == Scope::Namespaced,
                    });
                }
            }
        }
        let found = Arc::new(found);
        *self.inner.write().await = Some((Instant::now(), found.clone()));
        Ok(found)
    }
}

/// What a scan found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScanOutcome {
    /// Resources evaluated.
    pub scanned: u32,
    /// Resources that failed at least one rule.
    pub failing: u32,
    /// Sample of failures for the status (at most [`MAX_STATUS_RESULTS`]).
    pub results: Vec<ScanResult>,
    /// Problems that made the scan incomplete (for example missing RBAC), one per kind.
    pub problems: Vec<String>,
    /// True if the object limit stopped the scan early.
    pub truncated: bool,
    /// True if the policy matches `*` kinds, which are not scanned.
    pub wildcard: bool,
}

/// The resource types a policy's `match.resources` refers to.
pub fn targets(policy: &Policy, available: &[Listable]) -> (Vec<Listable>, bool) {
    let wildcard = policy
        .match_
        .resources
        .iter()
        .any(|r| r.kinds.iter().any(|k| k == "*"));
    let found = available
        .iter()
        .filter(|l| {
            policy.match_.resources.iter().any(|filter| {
                filter.kinds.iter().any(|k| k == &l.resource.kind)
                    && (filter.api_groups.is_empty()
                        || filter
                            .api_groups
                            .iter()
                            .any(|g| g == "*" || g == &l.resource.group))
            })
        })
        .cloned()
        .collect();
    (found, wildcard)
}

/// Labels of every namespace, by name.
pub type NamespaceLabels = BTreeMap<String, BTreeMap<String, String>>;

/// Labels of every namespace, for namespace selectors.
pub async fn namespace_labels(client: &Client) -> Result<NamespaceLabels, Error> {
    let api: Api<Namespace> = Api::all(client.clone());
    let list: ObjectList<PartialObjectMeta<Namespace>> =
        api.list_metadata(&ListParams::default()).await?;
    Ok(list
        .items
        .into_iter()
        .map(|ns| (ns.name_any(), ns.labels().clone()))
        .collect())
}

/// Lists and evaluates every resource the policy targets, up to `max_objects`.
pub async fn scan(
    client: &Client,
    policy: &Policy,
    targets: &[Listable],
    namespaces: &NamespaceLabels,
    max_objects: usize,
) -> ScanOutcome {
    let mut outcome = ScanOutcome::default();
    let mut seen = 0usize;
    'kinds: for target in targets {
        let api: Api<DynamicObject> = Api::all_with(client.clone(), &target.resource);
        let mut params = ListParams::default().limit(PAGE_SIZE);
        loop {
            let page = match api.list(&params).await {
                Ok(page) => page,
                Err(error) => {
                    outcome.problems.push(format!(
                        "{}: {}",
                        target.resource.kind,
                        short_error(&error)
                    ));
                    continue 'kinds;
                }
            };
            for object in &page.items {
                if seen >= max_objects {
                    outcome.truncated = true;
                    break 'kinds;
                }
                seen += 1;
                evaluate_one(policy, target, object, namespaces, &mut outcome);
            }
            match page.metadata.continue_.filter(|c| !c.is_empty()) {
                Some(token) => params = params.continue_token(&token),
                None => break,
            }
        }
    }
    outcome
}

fn evaluate_one(
    policy: &Policy,
    target: &Listable,
    object: &DynamicObject,
    namespaces: &NamespaceLabels,
    outcome: &mut ScanOutcome,
) {
    // Objects created by a controller whose kind the policy also checks (for example Pods of a
    // Deployment) are reported through their owner, not twice.
    let owned_by_checked_kind = object.owner_references().iter().any(|owner| {
        owner.controller == Some(true) && {
            let group = owner.api_version.rsplit_once('/').map_or("", |(g, _)| g);
            policy.match_.matches_kind(group, &owner.kind)
        }
    });
    if owned_by_checked_kind {
        return;
    }
    let Ok(value) = serde_json::to_value(object) else {
        return;
    };
    let namespace = object.namespace();
    let resource = Resource {
        group: &target.resource.group,
        kind: &target.resource.kind,
        namespace: namespace.as_deref().filter(|_| target.namespaced),
        namespace_labels: namespace.as_deref().and_then(|ns| namespaces.get(ns)),
        requester: None,
        object: &value,
    };
    let Some(result) = policy.evaluate(&resource) else {
        return;
    };
    outcome.scanned += 1;
    if result.passed() {
        return;
    }
    outcome.failing += 1;
    for violation in result.violations {
        if outcome.results.len() >= MAX_STATUS_RESULTS {
            break;
        }
        outcome.results.push(ScanResult {
            api_version_kind: format!("{}/{}", target.resource.api_version, target.resource.kind),
            namespace: namespace.clone(),
            name: object.name_any(),
            rule: violation.rule,
            message: violation.message,
            path: violation.path,
        });
    }
}

fn short_error(error: &kube::Error) -> String {
    match error {
        kube::Error::Api(status) => format!("{} ({})", status.reason, status.code),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferrogate_core::PolicySpec;
    use serde_json::json;

    fn listable(group: &str, version: &str, kind: &str, plural: &str) -> Listable {
        let api_version = if group.is_empty() {
            version.to_owned()
        } else {
            format!("{group}/{version}")
        };
        Listable {
            resource: ApiResource {
                group: group.into(),
                version: version.into(),
                api_version,
                kind: kind.into(),
                plural: plural.into(),
            },
            namespaced: true,
        }
    }

    #[allow(clippy::needless_pass_by_value)]
    fn policy(resources: serde_json::Value) -> Policy {
        let spec: PolicySpec = serde_json::from_value(json!({
            "match": {"resources": resources},
            "rules": [{"name": "tag", "disallowLatestTag": true}]
        }))
        .unwrap();
        Policy::compile("p", &spec).unwrap()
    }

    #[test]
    fn targets_follow_kinds_and_groups() {
        let available = vec![
            listable("", "v1", "Pod", "pods"),
            listable("apps", "v1", "Deployment", "deployments"),
            listable("extensions", "v1beta1", "Deployment", "deployments"),
            listable("", "v1", "ConfigMap", "configmaps"),
        ];
        let (found, wildcard) = targets(
            &policy(json!([{"apiGroups": ["apps"], "kinds": ["Deployment"]}])),
            &available,
        );
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].resource.group, "apps");
        assert!(!wildcard);

        let (found, _) = targets(
            &policy(json!([{"kinds": ["Deployment", "Pod"]}])),
            &available,
        );
        assert_eq!(found.len(), 3);

        let (found, wildcard) = targets(&policy(json!([{"kinds": ["*"]}])), &available);
        assert!(found.is_empty());
        assert!(wildcard);
    }

    #[test]
    fn owned_objects_are_reported_through_their_owner() {
        let p = policy(json!([{"apiGroups": ["", "apps"], "kinds": ["Pod", "ReplicaSet"]}]));
        let target = listable("", "v1", "Pod", "pods");
        let pod: DynamicObject = serde_json::from_value(json!({
            "apiVersion": "v1", "kind": "Pod",
            "metadata": {"name": "web-abc", "namespace": "default", "ownerReferences": [
                {"apiVersion": "apps/v1", "kind": "ReplicaSet", "name": "web", "uid": "1", "controller": true}
            ]},
            "spec": {"containers": [{"name": "a", "image": "nginx"}]}
        }))
        .unwrap();
        let mut outcome = ScanOutcome::default();
        evaluate_one(&p, &target, &pod, &NamespaceLabels::new(), &mut outcome);
        assert_eq!(outcome.scanned, 0);

        let mut bare = pod.clone();
        bare.metadata.owner_references = None;
        evaluate_one(&p, &target, &bare, &NamespaceLabels::new(), &mut outcome);
        assert_eq!((outcome.scanned, outcome.failing), (1, 1));
        assert_eq!(outcome.results[0].api_version_kind, "v1/Pod");
        assert_eq!(outcome.results[0].namespace.as_deref(), Some("default"));
    }
}
