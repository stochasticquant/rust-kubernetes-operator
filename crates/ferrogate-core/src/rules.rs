// SPDX-License-Identifier: Apache-2.0

use serde_json::Value;

use crate::Violation;

/// A check that a Kubernetes object either passes or fails.
///
/// Implementations must be pure: no I/O, no panics, and the same answer for the same input.
pub trait Rule {
    /// Stable name used in reports and messages.
    fn name(&self) -> &str;

    /// Returns `Some(violation)` if `object` fails the rule.
    fn check(&self, object: &Value) -> Option<Violation>;
}

/// Requires the object to carry every listed label.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequireLabels {
    pub labels: Vec<String>,
}

impl Rule for RequireLabels {
    fn name(&self) -> &'static str {
        "require-labels"
    }

    fn check(&self, object: &Value) -> Option<Violation> {
        let present = object
            .pointer("/metadata/labels")
            .and_then(Value::as_object);
        let missing: Vec<&str> = self
            .labels
            .iter()
            .filter(|label| present.is_none_or(|labels| !labels.contains_key(label.as_str())))
            .map(String::as_str)
            .collect();

        if missing.is_empty() {
            None
        } else {
            Some(Violation {
                rule: self.name().to_owned(),
                message: format!("missing required label(s): {}", missing.join(", ")),
            })
        }
    }
}

/// Runs every rule against `object` and returns all violations.
pub fn evaluate(rules: &[&dyn Rule], object: &Value) -> Vec<Violation> {
    rules.iter().filter_map(|rule| rule.check(object)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn require(labels: &[&str]) -> RequireLabels {
        RequireLabels {
            labels: labels.iter().map(ToString::to_string).collect(),
        }
    }

    #[test]
    fn passes_when_all_labels_present() {
        let object = json!({"metadata": {"labels": {"team": "a", "env": "prod"}}});
        assert_eq!(require(&["team", "env"]).check(&object), None);
    }

    #[test]
    fn reports_every_missing_label() {
        let object = json!({"metadata": {"labels": {"team": "a"}}});
        let violation = require(&["team", "env", "owner"]).check(&object).unwrap();
        assert_eq!(violation.rule, "require-labels");
        assert_eq!(violation.message, "missing required label(s): env, owner");
    }

    #[test]
    fn object_without_labels_fails() {
        let object = json!({"metadata": {"name": "x"}});
        assert!(require(&["team"]).check(&object).is_some());
    }

    #[test]
    fn malformed_object_does_not_panic() {
        for object in [json!(null), json!(42), json!({"metadata": "oops"})] {
            assert!(require(&["team"]).check(&object).is_some());
        }
    }

    #[test]
    fn empty_rule_always_passes() {
        assert_eq!(require(&[]).check(&json!({})), None);
    }

    #[test]
    fn evaluate_collects_all_violations() {
        let a = require(&["a"]);
        let b = require(&["b"]);
        let violations = evaluate(&[&a, &b], &json!({"metadata": {"labels": {"a": "1"}}}));
        assert_eq!(violations.len(), 1);
        assert!(violations[0].message.contains('b'));
    }
}
