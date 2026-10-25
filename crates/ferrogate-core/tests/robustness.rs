// SPDX-License-Identifier: Apache-2.0

//! Property tests: the engine must never panic, whatever the input (threat T-04).
//!
//! Objects come from the API server, but the webhook must stay up even for objects that are
//! malformed, huge or adversarial, so every check is run against random JSON shaped like
//! Kubernetes workloads.

// Test code: a failed setup should panic and fail the test.
#![allow(clippy::unwrap_used)]

use ferrogate_core::workload::ImageRef;
use ferrogate_core::{Policy, PolicySpec, Requester, Resource, decide};
use proptest::prelude::*;
use serde_json::{Map, Value, json};

/// Random JSON, biased towards the field names the rules look at.
fn json_value() -> impl Strategy<Value = Value> {
    let key = prop_oneof![
        Just("spec".to_owned()),
        Just("template".to_owned()),
        Just("jobTemplate".to_owned()),
        Just("containers".to_owned()),
        Just("initContainers".to_owned()),
        Just("ephemeralContainers".to_owned()),
        Just("securityContext".to_owned()),
        Just("capabilities".to_owned()),
        Just("add".to_owned()),
        Just("drop".to_owned()),
        Just("image".to_owned()),
        Just("ports".to_owned()),
        Just("hostPort".to_owned()),
        Just("volumes".to_owned()),
        Just("hostPath".to_owned()),
        Just("resources".to_owned()),
        Just("limits".to_owned()),
        Just("metadata".to_owned()),
        Just("labels".to_owned()),
        Just("seccompProfile".to_owned()),
        Just("type".to_owned()),
        "[a-zA-Z]{1,8}",
    ];
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        any::<i64>().prop_map(|n| json!(n)),
        any::<f64>().prop_map(|f| json!(f)),
        ".{0,24}".prop_map(Value::String),
    ];
    leaf.prop_recursive(6, 128, 8, move |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..6).prop_map(Value::Array),
            prop::collection::vec((key.clone(), inner), 0..6)
                .prop_map(|pairs| Value::Object(pairs.into_iter().collect::<Map<_, _>>())),
        ]
    })
}

fn all_checks_policy() -> Policy {
    let spec: PolicySpec = serde_json::from_value(json!({
        "mode": "Enforce",
        "match": {
            "resources": [{"kinds": ["*"]}],
            "namespaceSelector": {"matchExpressions": [{"key": "env", "operator": "NotIn", "values": ["x"]}]},
            "objectSelector": {"matchExpressions": [{"key": "skip", "operator": "DoesNotExist"}]}
        },
        "exclude": {"usernames": ["admin"], "groups": ["system:masters"]},
        "rules": [
            {"name": "a", "requireLabels": ["team"]},
            {"name": "b", "requireAnnotations": ["owner"]},
            {"name": "c", "allowedRegistries": ["ghcr.io/acme", "registry.k8s.io"]},
            {"name": "d", "disallowLatestTag": true},
            {"name": "e", "requireResourceLimits": ["cpu", "memory"]},
            {"name": "f", "requireResourceRequests": ["memory"]},
            {"name": "g", "disallowPrivileged": true},
            {"name": "h", "disallowHostNamespaces": true},
            {"name": "i", "disallowHostPath": true},
            {"name": "j", "disallowHostPorts": true},
            {"name": "k", "allowedCapabilities": ["NET_BIND_SERVICE"]},
            {"name": "l", "disallowPrivilegeEscalation": true},
            {"name": "m", "requireRunAsNonRoot": true},
            {"name": "n", "requireDropAllCapabilities": true},
            {"name": "o", "requireSeccompProfile": true}
        ]
    }))
    .unwrap();
    Policy::compile("all", &spec).unwrap()
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 2000, ..ProptestConfig::default() })]

    #[test]
    fn evaluation_never_panics(
        object in json_value(),
        kind in prop_oneof![Just("Pod"), Just("Deployment"), Just("CronJob"), Just("Namespace"), Just("Rollout")],
        namespace in prop::option::of("[a-z]{1,8}"),
    ) {
        let policy = all_checks_policy();
        let requester = Requester { username: "u".into(), groups: vec!["g".into()] };
        let resource = Resource {
            group: "",
            kind,
            namespace: namespace.as_deref(),
            namespace_labels: None,
            requester: Some(&requester),
            object: &object,
        };
        let decision = decide([&policy], &resource);
        for result in decision.failures() {
            for violation in &result.violations {
                prop_assert!(!violation.message.is_empty());
                prop_assert!(!violation.rule.is_empty());
            }
            let _ = result.summary();
        }
    }

    #[test]
    fn image_parsing_never_panics(image in ".{0,64}", prefix in ".{0,32}") {
        let parsed = ImageRef::parse(&image);
        let _ = parsed.is_latest();
        let _ = parsed.is_from(&prefix);
        let _ = parsed.full_name();
    }

    #[test]
    fn image_with_digest_is_never_latest(name in "[a-z]{1,10}(/[a-z]{1,10}){0,2}", digest in "[a-f0-9]{8,64}") {
        let image = format!("{name}@sha256:{digest}");
        prop_assert!(!ImageRef::parse(&image).is_latest());
    }

    #[test]
    fn policy_specs_from_random_json_never_panic(spec in json_value()) {
        if let Ok(spec) = serde_json::from_value::<PolicySpec>(spec) {
            let _ = Policy::compile("random", &spec);
        }
    }
}
