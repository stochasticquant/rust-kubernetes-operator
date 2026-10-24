// SPDX-License-Identifier: Apache-2.0

//! Property tests: the admission endpoint answers every request without panicking (threat T-04).

// Test code: a failed setup should panic and fail the test.
#![allow(clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use ferrogate_core::{Policy, PolicySpec};
use ferrogate_webhook::{Limits, PolicySource, WebhookMetrics, admission_router};
use prometheus_client::registry::Registry;
use proptest::prelude::*;
use serde_json::json;
use tower::ServiceExt;

struct Fixed(Arc<Vec<Policy>>);

impl PolicySource for Fixed {
    fn policies(&self) -> Arc<Vec<Policy>> {
        self.0.clone()
    }
    fn namespace_labels(&self, _: &str) -> Option<BTreeMap<String, String>> {
        None
    }
    fn is_ready(&self) -> bool {
        true
    }
}

fn router() -> axum::Router {
    let spec: PolicySpec = serde_json::from_value(json!({
        "mode": "Enforce",
        "match": {"resources": [{"kinds": ["*"]}]},
        "rules": [{"name": "tag", "disallowLatestTag": true}, {"name": "priv", "disallowPrivileged": true}]
    }))
    .unwrap();
    let policies = Arc::new(vec![Policy::compile("p", &spec).unwrap()]);
    let mut registry = Registry::default();
    admission_router(
        Arc::new(Fixed(policies)),
        WebhookMetrics::register(&mut registry),
        Limits::default(),
    )
}

fn status_for(body: Vec<u8>) -> StatusCode {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let request = Request::post("/validate")
            .header("content-type", "application/json")
            .body(Body::from(body))
            .unwrap();
        router().oneshot(request).await.unwrap().status()
    })
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 500, ..ProptestConfig::default() })]

    #[test]
    fn random_bytes_get_an_answer(body in prop::collection::vec(any::<u8>(), 0..2048)) {
        let status = status_for(body);
        prop_assert!(status == StatusCode::OK || status == StatusCode::BAD_REQUEST);
    }

    #[test]
    fn reviews_with_random_objects_get_an_answer(
        operation in prop_oneof![Just("CREATE".to_owned()), Just("UPDATE".to_owned()), Just("DELETE".to_owned()), ".{0,8}"],
        kind in ".{0,12}",
        image in ".{0,40}",
        privileged in any::<Option<bool>>(),
        extra in ".{0,64}",
    ) {
        let review = json!({
            "apiVersion": "admission.k8s.io/v1",
            "kind": "AdmissionReview",
            "request": {
                "uid": "u",
                "kind": {"group": "", "version": "v1", "kind": kind},
                "operation": operation,
                "object": {
                    "metadata": {"annotations": {"x": extra}},
                    "spec": {"containers": [{"name": "c", "image": image, "securityContext": {"privileged": privileged}}]}
                }
            }
        });
        let status = status_for(review.to_string().into_bytes());
        prop_assert_eq!(status, StatusCode::OK);
    }
}
