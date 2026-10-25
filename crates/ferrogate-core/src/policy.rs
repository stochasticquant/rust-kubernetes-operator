// SPDX-License-Identifier: Apache-2.0

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// What happens when a resource violates a policy.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum Mode {
    /// Record violations in reports only. Nothing is blocked. This is the safe default.
    #[default]
    Audit,
    /// Allow the request but return a warning to the client.
    Warn,
    /// Deny the request.
    Enforce,
}

/// How serious a violation is.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
pub enum Severity {
    Low,
    #[default]
    Medium,
    High,
    Critical,
}

/// One rule that a resource failed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Violation {
    /// Name of the rule that failed.
    pub rule: String,
    /// Human-readable explanation, safe to show to the user who made the request.
    pub message: String,
}
