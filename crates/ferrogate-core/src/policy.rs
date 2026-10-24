// SPDX-License-Identifier: Apache-2.0

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// What happens when a resource violates a policy.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub enum Mode {
    /// Record violations only. Nothing is blocked. This is the safe default.
    #[default]
    Audit,
    /// Allow the request but return a warning to the client.
    Warn,
    /// Deny the request.
    Enforce,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Audit => "Audit",
            Self::Warn => "Warn",
            Self::Enforce => "Enforce",
        }
    }
}

/// How serious a violation is.
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    Serialize,
    Deserialize,
    JsonSchema,
)]
pub enum Severity {
    Low,
    #[default]
    Medium,
    High,
    Critical,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "Low",
            Self::Medium => "Medium",
            Self::High => "High",
            Self::Critical => "Critical",
        }
    }
}

/// One way in which a resource failed a rule.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Violation {
    /// Name of the rule that failed.
    pub rule: String,
    /// Human-readable explanation, safe to show to the user who made the request.
    /// It never contains field values other than names and images.
    pub message: String,
    /// Field path of the offending value, for example `spec.containers[0].image`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// The outcome of evaluating one policy against one resource.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PolicyResult {
    pub policy: String,
    pub mode: Mode,
    pub severity: Severity,
    pub violations: Vec<Violation>,
}

impl PolicyResult {
    pub fn passed(&self) -> bool {
        self.violations.is_empty()
    }

    /// One-line summary used in admission messages and warnings.
    pub fn summary(&self) -> String {
        let details: Vec<String> = self
            .violations
            .iter()
            .map(|v| match &v.path {
                Some(path) => format!("{} ({path})", v.message),
                None => v.message.clone(),
            })
            .collect();
        format!("[{}] {}", self.policy, details.join("; "))
    }
}
