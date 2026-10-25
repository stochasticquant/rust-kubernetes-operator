// SPDX-License-Identifier: Apache-2.0

//! Policies: definition, validation and evaluation.

use std::collections::BTreeSet;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::CompileError;
use crate::matching::{ExcludeSpec, MatchSpec, Resource};
use crate::rules::{Rule, RuleSpec};
use crate::{Enforcement, Mode, PolicyResult, Severity};

/// Maximum number of rules in one policy.
pub const MAX_RULES: usize = 64;

/// A policy as written by the user. Shared by every Ferrogate policy kind.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PolicySpec {
    /// `Audit` (default) only reports, `Warn` allows with a warning, `Enforce` denies.
    #[serde(default)]
    pub mode: Mode,

    /// How serious a violation of this policy is.
    #[serde(default)]
    pub severity: Severity,

    /// `Webhook` (default) or `Native` (compiled to a `ValidatingAdmissionPolicy`).
    #[serde(default)]
    pub enforcement: Enforcement,

    /// Which resources the policy applies to.
    #[serde(rename = "match")]
    pub match_: MatchSpec,

    /// Resources and requesters to skip.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclude: Option<ExcludeSpec>,

    /// The checks. A resource passes only if it passes every rule.
    #[schemars(length(min = 1, max = 64))]
    pub rules: Vec<RuleSpec>,
}

/// A validated policy, ready to evaluate. Cheap to clone and share between threads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Policy {
    pub name: String,
    pub mode: Mode,
    pub severity: Severity,
    pub enforcement: Enforcement,
    pub match_: MatchSpec,
    pub exclude: ExcludeSpec,
    pub rules: Vec<Rule>,
}

impl Policy {
    /// Validates `spec`. The error is meant for the policy's status condition.
    pub fn compile(name: &str, spec: &PolicySpec) -> Result<Self, CompileError> {
        spec.match_.validate()?;
        let exclude = spec.exclude.clone().unwrap_or_default();
        exclude.validate()?;
        if spec.rules.is_empty() {
            return Err(CompileError::NoRules);
        }
        if spec.rules.len() > MAX_RULES {
            return Err(CompileError::TooManyRules(spec.rules.len()));
        }
        let mut seen = BTreeSet::new();
        let mut rules = Vec::with_capacity(spec.rules.len());
        for rule_spec in &spec.rules {
            let rule = Rule::compile(rule_spec)?;
            if !seen.insert(rule.name.clone()) {
                return Err(CompileError::DuplicateRule(rule.name));
            }
            rules.push(rule);
        }
        Ok(Self {
            name: name.to_owned(),
            mode: spec.mode,
            severity: spec.severity,
            enforcement: spec.enforcement,
            match_: spec.match_.clone(),
            exclude,
            rules,
        })
    }

    /// True if the policy applies to `resource`.
    pub fn applies_to(&self, resource: &Resource<'_>) -> bool {
        self.match_.matches(resource) && !self.exclude.excludes(resource)
    }

    /// Evaluates every rule. Returns `None` if the policy does not apply to `resource`.
    pub fn evaluate(&self, resource: &Resource<'_>) -> Option<PolicyResult> {
        if !self.applies_to(resource) {
            return None;
        }
        let violations = self
            .rules
            .iter()
            .flat_map(|rule| rule.check(resource.kind, resource.object))
            .collect();
        Some(PolicyResult {
            policy: self.name.clone(),
            mode: self.mode,
            severity: self.severity,
            violations,
        })
    }
}

/// The combined admission outcome of several policies for one request.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Decision {
    /// Failed `Enforce` policies: the request must be denied.
    pub denied: Vec<PolicyResult>,
    /// Failed `Warn` policies: allowed, with a warning to the client.
    pub warned: Vec<PolicyResult>,
    /// Failed `Audit` policies: allowed silently, recorded for reporting.
    pub audited: Vec<PolicyResult>,
    /// Number of policies that applied and passed.
    pub passed: usize,
}

impl Decision {
    pub fn allowed(&self) -> bool {
        self.denied.is_empty()
    }

    /// Every failed result, whatever the mode.
    pub fn failures(&self) -> impl Iterator<Item = &PolicyResult> {
        self.denied.iter().chain(&self.warned).chain(&self.audited)
    }
}

/// Evaluates every policy against `resource` and sorts the results by mode.
pub fn decide<'p>(
    policies: impl IntoIterator<Item = &'p Policy>,
    resource: &Resource<'_>,
) -> Decision {
    let mut decision = Decision::default();
    for policy in policies {
        let Some(result) = policy.evaluate(resource) else {
            continue;
        };
        if result.passed() {
            decision.passed += 1;
            continue;
        }
        match result.mode {
            Mode::Enforce => decision.denied.push(result),
            Mode::Warn => decision.warned.push(result),
            Mode::Audit => decision.audited.push(result),
        }
    }
    decision
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn policy(name: &str, spec: Value) -> Policy {
        Policy::compile(name, &serde_json::from_value(spec).unwrap()).unwrap()
    }

    fn pod(object: &Value) -> Resource<'_> {
        Resource {
            group: "",
            kind: "Pod",
            namespace: Some("default"),
            namespace_labels: None,
            requester: None,
            object,
        }
    }

    #[test]
    fn spec_defaults_to_audit_and_medium() {
        let spec: PolicySpec = serde_json::from_value(json!({
            "match": {"resources": [{"kinds": ["Pod"]}]},
            "rules": [{"name": "r", "disallowPrivileged": true}]
        }))
        .unwrap();
        assert_eq!(spec.mode, Mode::Audit);
        assert_eq!(spec.severity, Severity::Medium);
    }

    #[test]
    fn compile_rejects_duplicates_and_empty_rules() {
        let dup: PolicySpec = serde_json::from_value(json!({
            "match": {"resources": [{"kinds": ["Pod"]}]},
            "rules": [{"name": "r", "disallowPrivileged": true}, {"name": "r", "disallowHostPath": true}]
        }))
        .unwrap();
        assert_eq!(
            Policy::compile("p", &dup),
            Err(CompileError::DuplicateRule("r".into()))
        );

        let none: PolicySpec = serde_json::from_value(json!({
            "match": {"resources": [{"kinds": ["Pod"]}]}, "rules": []
        }))
        .unwrap();
        assert_eq!(Policy::compile("p", &none), Err(CompileError::NoRules));
    }

    #[test]
    fn decision_sorts_by_mode() {
        let rules = json!([{"name": "tag", "disallowLatestTag": true}]);
        let enforce = policy(
            "enforce",
            json!({"mode": "Enforce", "match": {"resources": [{"kinds": ["Pod"]}]}, "rules": rules}),
        );
        let warn = policy(
            "warn",
            json!({"mode": "Warn", "match": {"resources": [{"kinds": ["Pod"]}]}, "rules": rules}),
        );
        let audit = policy(
            "audit",
            json!({"match": {"resources": [{"kinds": ["Pod"]}]}, "rules": rules}),
        );
        let other = policy(
            "deploy-only",
            json!({"mode": "Enforce", "match": {"resources": [{"kinds": ["Deployment"]}]}, "rules": rules}),
        );
        let policies = [enforce, warn, audit, other];

        let bad = json!({"spec": {"containers": [{"name": "a", "image": "nginx"}]}});
        let decision = decide(&policies, &pod(&bad));
        assert!(!decision.allowed());
        assert_eq!(decision.denied[0].policy, "enforce");
        assert_eq!(decision.warned[0].policy, "warn");
        assert_eq!(decision.audited[0].policy, "audit");
        assert_eq!(decision.failures().count(), 3);

        let good = json!({"spec": {"containers": [{"name": "a", "image": "nginx:1.27"}]}});
        let decision = decide(&policies, &pod(&good));
        assert!(decision.allowed());
        assert_eq!(decision.passed, 3);
    }

    #[test]
    fn excluded_resources_are_not_evaluated() {
        let p = policy(
            "p",
            json!({
                "mode": "Enforce",
                "match": {"resources": [{"kinds": ["Pod"]}]},
                "exclude": {"namespaces": ["default"]},
                "rules": [{"name": "tag", "disallowLatestTag": true}]
            }),
        );
        let bad = json!({"spec": {"containers": [{"name": "a", "image": "nginx"}]}});
        assert_eq!(p.evaluate(&pod(&bad)), None);
    }

    #[test]
    fn summary_lists_messages_and_paths() {
        let p = policy(
            "tags",
            json!({
                "match": {"resources": [{"kinds": ["Pod"]}]},
                "rules": [{"name": "tag", "disallowLatestTag": true}]
            }),
        );
        let bad = json!({"spec": {"containers": [{"name": "a", "image": "nginx"}]}});
        let result = p.evaluate(&pod(&bad)).unwrap();
        assert_eq!(
            result.summary(),
            "[tags] image \"nginx\" of container \"a\" must use a fixed tag or digest, not latest (spec.containers[0].image)"
        );
    }
}
