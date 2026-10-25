// SPDX-License-Identifier: Apache-2.0

//! Label selectors with Kubernetes semantics, without depending on Kubernetes crates.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A Kubernetes-style label selector. An empty selector matches everything.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LabelSelector {
    /// Every key/value pair must be present.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub match_labels: BTreeMap<String, String>,

    /// Every expression must be true.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(length(max = 32))]
    pub match_expressions: Vec<LabelSelectorRequirement>,
}

/// One expression of a label selector.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LabelSelectorRequirement {
    pub key: String,
    pub operator: SelectorOperator,
    /// Values for `In` and `NotIn`. Must be empty for `Exists` and `DoesNotExist`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(length(max = 64))]
    pub values: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum SelectorOperator {
    In,
    NotIn,
    Exists,
    DoesNotExist,
}

impl LabelSelector {
    pub fn is_empty(&self) -> bool {
        self.match_labels.is_empty() && self.match_expressions.is_empty()
    }

    pub fn matches(&self, labels: &BTreeMap<String, String>) -> bool {
        self.match_labels
            .iter()
            .all(|(key, value)| labels.get(key) == Some(value))
            && self
                .match_expressions
                .iter()
                .all(|expr| expr.matches(labels))
    }
}

impl LabelSelectorRequirement {
    fn matches(&self, labels: &BTreeMap<String, String>) -> bool {
        let value = labels.get(&self.key);
        match self.operator {
            SelectorOperator::In => value.is_some_and(|v| self.values.contains(v)),
            SelectorOperator::NotIn => value.is_none_or(|v| !self.values.contains(v)),
            SelectorOperator::Exists => value.is_some(),
            SelectorOperator::DoesNotExist => value.is_none(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    fn expr(key: &str, operator: SelectorOperator, values: &[&str]) -> LabelSelector {
        LabelSelector {
            match_labels: BTreeMap::new(),
            match_expressions: vec![LabelSelectorRequirement {
                key: key.into(),
                operator,
                values: values.iter().map(ToString::to_string).collect(),
            }],
        }
    }

    #[test]
    fn empty_selector_matches_everything() {
        assert!(LabelSelector::default().matches(&labels(&[])));
    }

    #[test]
    fn match_labels_needs_exact_values() {
        let selector = LabelSelector {
            match_labels: labels(&[("env", "prod")]),
            match_expressions: vec![],
        };
        assert!(selector.matches(&labels(&[("env", "prod"), ("x", "y")])));
        assert!(!selector.matches(&labels(&[("env", "dev")])));
        assert!(!selector.matches(&labels(&[])));
    }

    #[test]
    fn operators_follow_kubernetes_semantics() {
        let prod = labels(&[("env", "prod")]);
        let none = labels(&[]);
        let r#in = expr("env", SelectorOperator::In, &["prod", "stage"]);
        assert!(r#in.matches(&prod));
        assert!(!r#in.matches(&none));

        let not_in = expr("env", SelectorOperator::NotIn, &["prod"]);
        assert!(!not_in.matches(&prod));
        assert!(
            not_in.matches(&none),
            "NotIn matches when the key is absent"
        );

        assert!(expr("env", SelectorOperator::Exists, &[]).matches(&prod));
        assert!(!expr("env", SelectorOperator::Exists, &[]).matches(&none));
        assert!(expr("env", SelectorOperator::DoesNotExist, &[]).matches(&none));
        assert!(!expr("env", SelectorOperator::DoesNotExist, &[]).matches(&prod));
    }
}
