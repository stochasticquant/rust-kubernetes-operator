// SPDX-License-Identifier: Apache-2.0

//! Ferrogate policy model and evaluation engine.
//!
//! This crate has no Kubernetes client and does no I/O. It takes a Kubernetes object as JSON and
//! returns violations, so it can be used from the operator, the webhook and the CLI alike, and is
//! fast to test.

mod policy;
mod rules;

pub use policy::{Mode, Severity, Violation};
pub use rules::{RequireLabels, Rule, evaluate};
