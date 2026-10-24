// SPDX-License-Identifier: Apache-2.0

//! Ferrogate policy model and evaluation engine.
//!
//! This crate has no Kubernetes client and does no I/O. It takes a Kubernetes object as JSON and
//! returns violations, so the operator, the webhook and the CLI all reach the same verdict, and it
//! is fast to test.
//!
//! ```
//! use ferrogate_core::{Policy, PolicySpec, Resource, decide};
//! use serde_json::json;
//!
//! let spec: PolicySpec = serde_json::from_value(json!({
//!     "mode": "Enforce",
//!     "match": {"resources": [{"kinds": ["Pod"]}]},
//!     "rules": [{"name": "no-latest", "disallowLatestTag": true}]
//! })).unwrap();
//! let policy = Policy::compile("no-latest", &spec).unwrap();
//!
//! let pod = json!({"spec": {"containers": [{"name": "web", "image": "nginx"}]}});
//! let resource = Resource {
//!     group: "", kind: "Pod", namespace: Some("default"),
//!     namespace_labels: None, requester: None, object: &pod,
//! };
//! assert!(!decide([&policy], &resource).allowed());
//! ```

mod engine;
mod error;
mod matching;
mod policy;
mod rules;
mod selector;
pub mod workload;

pub use engine::{Decision, MAX_RULES, Policy, PolicySpec, decide};
pub use error::CompileError;
pub use matching::{
    ExcludeSpec, MatchSpec, NAMESPACE_NAME_LABEL, Requester, Resource, ResourceFilter,
};
pub use policy::{Mode, PolicyResult, Severity, Violation};
pub use rules::{Check, Rule, RuleSpec};
pub use selector::{LabelSelector, LabelSelectorRequirement, SelectorOperator};
