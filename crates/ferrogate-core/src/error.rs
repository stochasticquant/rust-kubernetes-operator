// SPDX-License-Identifier: Apache-2.0

/// Why a policy definition is invalid. Shown in the policy's `Ready` condition.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CompileError {
    #[error("a rule has an empty name")]
    EmptyRuleName,
    #[error("rule {rule:?}: {reason}")]
    InvalidRule { rule: String, reason: String },
    #[error("rule name {0:?} is used more than once")]
    DuplicateRule(String),
    #[error("the policy has no rules")]
    NoRules,
    #[error("the policy has too many rules ({0}, maximum {max})", max = crate::MAX_RULES)]
    TooManyRules(usize),
    #[error("match.resources must list at least one entry")]
    NoResources,
    #[error("match.resources[{0}].kinds must list at least one kind (use \"*\" for all)")]
    NoKinds(usize),
    #[error("{field}: operator {operator} {reason}")]
    InvalidSelector {
        field: String,
        operator: String,
        reason: String,
    },
}
