// SPDX-License-Identifier: Apache-2.0

//! Controller tests against a fake API server.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use ferrogate_crd::ClusterGatePolicy;
use kube::runtime::controller::Action;
use kube::runtime::events::{Recorder, Reporter};
use kube::runtime::reflector;
use prometheus_client::registry::Registry;
use serde_json::{Value, json};

use crate::mock::{self, Recorded};
use crate::{Config, Context, OperatorMetrics, Outcome, reconcile_inner, scan};

fn context(client: kube::Client, config: Config) -> Context {
    let mut registry = Registry::default();
    Context {
        recorder: Recorder::new(
            client.clone(),
            Reporter {
                controller: "ferrogate".into(),
                instance: None,
            },
        ),
        client,
        config,
        metrics: OperatorMetrics::register(&mut registry),
        discovery: scan::DiscoveryCache::default(),
        store: reflector::store::<ClusterGatePolicy>().0,
        failures: Mutex::new(HashMap::new()),
    }
}

fn policy(spec: &Value, status: Option<Value>) -> ClusterGatePolicy {
    let mut object = json!({
        "apiVersion": "ferrogate.stochasticquant.io/v1alpha1",
        "kind": "ClusterGatePolicy",
        "metadata": {"name": "p", "generation": 3, "uid": "uid-1", "resourceVersion": "7"},
        "spec": spec
    });
    if let Some(status) = status {
        object["status"] = status;
    }
    serde_json::from_value(object).unwrap()
}

/// Answers status patches with the policy itself, accepts events, and rejects anything else.
fn api(policy_json: Value) -> mock::Responder {
    Box::new(move |method, path, body| match (method, path) {
        ("PATCH", p) if p.ends_with("/clustergatepolicies/p/status") => (200, policy_json.clone()),
        ("POST", p) if p.contains("/events") => (201, body.clone()),
        _ => mock::status(404, "NotFound"),
    })
}

async fn run(policy: &ClusterGatePolicy, config: Config) -> (Action, Outcome, Vec<Recorded>) {
    let (client, log) = mock::client(api(serde_json::to_value(policy).unwrap()));
    let ctx = context(client, config);
    let (action, outcome) = reconcile_inner(policy, &ctx).await.unwrap();
    let requests = log.lock().unwrap().clone();
    (action, outcome, requests)
}

fn no_scan() -> Config {
    Config {
        scan: false,
        ..Config::default()
    }
}

fn status_patch(requests: &[Recorded]) -> &Value {
    let patch = requests
        .iter()
        .find(|r| r.method == "PATCH" && r.path.ends_with("/status"))
        .expect("a status patch");
    assert_eq!(
        patch.path,
        "/apis/ferrogate.stochasticquant.io/v1alpha1/clustergatepolicies/p/status"
    );
    &patch.body["status"]
}

#[tokio::test]
async fn invalid_policy_gets_ready_false_and_waits_for_a_change() {
    let spec = json!({
        "match": {"resources": [{"kinds": ["Pod"]}]},
        "rules": [{"name": "off", "disallowPrivileged": false}]
    });
    let (action, outcome, requests) = run(&policy(&spec, None), no_scan()).await;

    assert_eq!(outcome, Outcome::Invalid);
    assert_eq!(action, Action::await_change());
    let status = status_patch(&requests);
    assert_eq!(status["observedGeneration"], 3);
    assert_eq!(status["conditions"][0]["type"], "Ready");
    assert_eq!(status["conditions"][0]["status"], "False");
    assert_eq!(status["conditions"][0]["reason"], "InvalidPolicy");
    assert!(
        status["conditions"][0]["message"]
            .as_str()
            .unwrap()
            .contains("must be true")
    );
    // Stale scan fields are cleared explicitly.
    assert_eq!(status["violations"], Value::Null);
    assert!(
        requests
            .iter()
            .any(|r| r.method == "POST" && r.body["reason"] == "InvalidPolicy")
    );
}

#[tokio::test]
async fn valid_policy_gets_ready_true() {
    let spec = json!({
        "mode": "Enforce",
        "match": {"resources": [{"kinds": ["Pod"]}]},
        "rules": [{"name": "tag", "disallowLatestTag": true}]
    });
    let (action, outcome, requests) = run(&policy(&spec, None), no_scan()).await;

    assert_eq!(outcome, Outcome::Active);
    assert_eq!(action, Action::requeue(Config::default().scan_interval));
    let status = status_patch(&requests);
    assert_eq!(status["conditions"][0]["status"], "True");
    assert_eq!(status["conditions"][0]["reason"], "Active");
    assert_eq!(
        status["conditions"][0]["message"],
        "1 rule(s), mode Enforce, severity Medium"
    );
    assert!(
        requests
            .iter()
            .any(|r| r.method == "POST" && r.body["reason"] == "Active")
    );
}

#[tokio::test]
async fn up_to_date_invalid_policy_makes_no_requests() {
    let spec = json!({
        "match": {"resources": [{"kinds": ["Pod"]}]},
        "rules": [{"name": "off", "disallowPrivileged": false}]
    });
    let status = json!({
        "observedGeneration": 3,
        "conditions": [{"type": "Ready", "status": "False", "reason": "InvalidPolicy",
                        "message": "x", "lastTransitionTime": "2026-01-01T00:00:00Z"}]
    });
    let (action, outcome, requests) = run(&policy(&spec, Some(status)), no_scan()).await;
    assert_eq!(outcome, Outcome::Unchanged);
    assert_eq!(action, Action::await_change());
    assert!(requests.is_empty(), "{requests:?}");
}

#[tokio::test]
async fn recently_scanned_policy_is_not_rescanned() {
    let spec = json!({
        "match": {"resources": [{"kinds": ["Pod"]}]},
        "rules": [{"name": "tag", "disallowLatestTag": true}]
    });
    let status = json!({
        "observedGeneration": 3,
        "lastScanTime": k8s_openapi::jiff::Timestamp::now().to_string(),
        "conditions": [{"type": "Ready", "status": "True", "reason": "Active",
                        "message": "x", "lastTransitionTime": "2026-01-01T00:00:00Z"}]
    });
    let (_, outcome, requests) = run(&policy(&spec, Some(status)), Config::default()).await;
    assert_eq!(outcome, Outcome::Unchanged);
    assert!(requests.is_empty(), "{requests:?}");
}

#[tokio::test]
async fn no_event_when_ready_does_not_change() {
    let spec = json!({
        "match": {"resources": [{"kinds": ["Pod"]}]},
        "rules": [{"name": "tag", "disallowLatestTag": true}]
    });
    // Generation changed (2 -> 3), but the policy was already Ready.
    let status = json!({
        "observedGeneration": 2,
        "conditions": [{"type": "Ready", "status": "True", "reason": "Active",
                        "message": "x", "lastTransitionTime": "2026-01-01T00:00:00Z"}]
    });
    let (_, _, requests) = run(&policy(&spec, Some(status)), no_scan()).await;
    assert!(!requests.iter().any(|r| r.method == "POST"), "{requests:?}");
    // The transition time of an unchanged condition is kept.
    assert_eq!(
        status_patch(&requests)["conditions"][0]["lastTransitionTime"],
        "2026-01-01T00:00:00Z"
    );
}

#[tokio::test]
async fn api_errors_are_returned_for_backoff() {
    let spec = json!({
        "match": {"resources": [{"kinds": ["Pod"]}]},
        "rules": [{"name": "tag", "disallowLatestTag": true}]
    });
    let (client, _) = mock::client(Box::new(|_, _, _| mock::status(500, "InternalError")));
    let ctx = context(client, no_scan());
    assert!(reconcile_inner(&policy(&spec, None), &ctx).await.is_err());
}

mod native {
    use super::*;
    use kube::core::ApiResource;

    fn listable(group: &str, kind: &str, plural: &str) -> scan::Listable {
        scan::Listable {
            resource: ApiResource {
                group: group.into(),
                version: "v1".into(),
                api_version: if group.is_empty() {
                    "v1".into()
                } else {
                    format!("{group}/v1")
                },
                kind: kind.into(),
                plural: plural.into(),
            },
            namespaced: true,
        }
    }

    fn api() -> mock::Responder {
        Box::new(|method, path, body| match (method, path) {
            ("PATCH", p) if p.contains("/validatingadmissionpolic") => (200, body.clone()),
            ("PATCH", p) if p.contains("/clustergatepolicies/p/status") => (
                200,
                json!({
                    "apiVersion": "ferrogate.stochasticquant.io/v1alpha1", "kind": "ClusterGatePolicy",
                    "metadata": {"name": "p"}, "spec": {"match": {"resources": [{"kinds": ["Pod"]}]}, "rules": []}
                }),
            ),
            ("DELETE", _) => (
                200,
                json!({"kind": "Status", "apiVersion": "v1", "status": "Success"}),
            ),
            ("POST", p) if p.contains("/events") => (201, body.clone()),
            _ => mock::status(404, "NotFound"),
        })
    }

    async fn run_native(policy: &ClusterGatePolicy) -> (Outcome, Vec<Recorded>) {
        let (client, log) = mock::client(api());
        let mut ctx = context(client, no_scan());
        ctx.discovery = scan::DiscoveryCache::with(vec![listable("", "Pod", "pods")]);
        let (_, outcome) = reconcile_inner(policy, &ctx).await.unwrap();
        let requests = log.lock().unwrap().clone();
        (outcome, requests)
    }

    #[tokio::test]
    async fn native_policy_applies_vap_and_binding() {
        let spec = json!({
            "mode": "Enforce", "enforcement": "Native",
            "match": {"resources": [{"kinds": ["Pod"]}]},
            "rules": [{"name": "tag", "disallowLatestTag": true}]
        });
        let (outcome, requests) = run_native(&policy(&spec, None)).await;
        assert_eq!(outcome, Outcome::Active);

        let vap = requests
            .iter()
            .find(|r| r.path.ends_with("/validatingadmissionpolicies/ferrogate-p"))
            .expect("policy applied");
        assert_eq!(vap.method, "PATCH");
        assert_eq!(vap.body["metadata"]["ownerReferences"][0]["uid"], "uid-1");
        assert_eq!(
            vap.body["spec"]["matchConstraints"]["resourceRules"][0]["resources"],
            json!(["pods"])
        );
        let binding = requests
            .iter()
            .find(|r| {
                r.path
                    .ends_with("/validatingadmissionpolicybindings/ferrogate-p")
            })
            .expect("binding applied");
        assert_eq!(binding.body["spec"]["validationActions"], json!(["Deny"]));

        let status = status_patch(&requests);
        assert_eq!(status["enforcement"], "Native");
        assert!(
            status["conditions"][0]["message"]
                .as_str()
                .unwrap()
                .ends_with("enforced natively by ValidatingAdmissionPolicy ferrogate-p")
        );
    }

    #[tokio::test]
    async fn unsupported_native_policy_is_rejected_and_cleaned_up() {
        let spec = json!({
            "enforcement": "Native",
            "match": {"resources": [{"kinds": ["Pod"]}]},
            "rules": [{"name": "reg", "allowedRegistries": ["ghcr.io"]}]
        });
        let previous = json!({"observedGeneration": 2, "enforcement": "Native"});
        let (outcome, requests) = run_native(&policy(&spec, Some(previous))).await;
        assert_eq!(outcome, Outcome::Invalid);
        let status = status_patch(&requests);
        assert_eq!(status["conditions"][0]["reason"], "NativeUnsupported");
        assert_eq!(status["enforcement"], Value::Null);
        let deletes: Vec<&str> = requests
            .iter()
            .filter(|r| r.method == "DELETE")
            .map(|r| r.path.as_str())
            .collect();
        assert_eq!(deletes.len(), 2, "{deletes:?}");
        assert!(
            !requests
                .iter()
                .any(|r| r.method == "PATCH" && r.path.contains("validatingadmission"))
        );
    }

    #[tokio::test]
    async fn switching_back_to_webhook_removes_generated_objects() {
        let spec = json!({
            "match": {"resources": [{"kinds": ["Pod"]}]},
            "rules": [{"name": "tag", "disallowLatestTag": true}]
        });
        let previous = json!({"observedGeneration": 2, "enforcement": "Native"});
        let (outcome, requests) = run_native(&policy(&spec, Some(previous))).await;
        assert_eq!(outcome, Outcome::Active);
        let deleted: Vec<&str> = requests
            .iter()
            .filter(|r| r.method == "DELETE")
            .map(|r| r.path.as_str())
            .collect();
        assert_eq!(
            deleted,
            [
                "/apis/admissionregistration.k8s.io/v1/validatingadmissionpolicybindings/ferrogate-p",
                "/apis/admissionregistration.k8s.io/v1/validatingadmissionpolicies/ferrogate-p"
            ]
        );
        assert_eq!(status_patch(&requests)["enforcement"], "Webhook");
    }
}

mod leader {
    use super::*;
    use crate::leader::LeaseConfig;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn lease_json(holder: &str, renewed: &str) -> Value {
        json!({
            "apiVersion": "coordination.k8s.io/v1", "kind": "Lease",
            "metadata": {"name": "ferrogate-operator", "namespace": "ferrogate-system", "resourceVersion": "5"},
            "spec": {"holderIdentity": holder, "leaseDurationSeconds": 15, "renewTime": renewed, "leaseTransitions": 1}
        })
    }

    fn config() -> LeaseConfig {
        LeaseConfig::new("ferrogate-system", "me")
    }

    #[tokio::test]
    async fn creates_a_missing_lease() {
        let (client, log) = mock::client(Box::new(|method, _, body| match method {
            "GET" => mock::status(404, "NotFound"),
            "POST" => (201, body.clone()),
            _ => mock::status(400, "BadRequest"),
        }));
        let (tx, rx) = tokio::sync::watch::channel(false);
        let task = tokio::spawn(crate::leader::run(client, config(), tx));
        let mut rx = rx;
        rx.wait_for(|leader| *leader).await.unwrap();
        task.abort();
        let requests = log.lock().unwrap().clone();
        let created = requests.iter().find(|r| r.method == "POST").unwrap();
        assert_eq!(created.body["spec"]["holderIdentity"], "me");
        assert_eq!(created.body["spec"]["leaseDurationSeconds"], 15);
    }

    #[tokio::test]
    async fn waits_while_another_replica_holds_a_fresh_lease() {
        let now = k8s_openapi::jiff::Timestamp::now().to_string();
        let writes = Arc::new(AtomicUsize::new(0));
        let counted = writes.clone();
        let (client, _) = mock::client(Box::new(move |method, _, _| {
            if method == "GET" {
                (200, lease_json("other", &now))
            } else {
                counted.fetch_add(1, Ordering::SeqCst);
                mock::status(400, "BadRequest")
            }
        }));
        let (tx, rx) = tokio::sync::watch::channel(false);
        let mut cfg = config();
        cfg.retry_period = std::time::Duration::from_millis(10);
        let task = tokio::spawn(crate::leader::run(client, cfg, tx));
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        task.abort();
        assert!(!*rx.borrow());
        assert_eq!(writes.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn takes_over_an_expired_lease_with_optimistic_locking() {
        let (client, log) = mock::client(Box::new(|method, _, body| match method {
            "GET" => (200, lease_json("other", "2020-01-01T00:00:00.000000Z")),
            "PUT" => (200, body.clone()),
            _ => mock::status(400, "BadRequest"),
        }));
        let (tx, mut rx) = tokio::sync::watch::channel(false);
        let task = tokio::spawn(crate::leader::run(client, config(), tx));
        rx.wait_for(|leader| *leader).await.unwrap();
        task.abort();
        let requests = log.lock().unwrap().clone();
        let put = requests.iter().find(|r| r.method == "PUT").unwrap();
        assert_eq!(put.body["metadata"]["resourceVersion"], "5");
        assert_eq!(put.body["spec"]["holderIdentity"], "me");
        assert_eq!(put.body["spec"]["leaseTransitions"], 2);
    }

    #[tokio::test]
    async fn steps_down_when_another_replica_wins() {
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        let (client, _) = mock::client(Box::new(move |method, _, body| {
            let n = counter.fetch_add(1, Ordering::SeqCst);
            match (method, n) {
                // First round: lease expired, we take it.
                ("GET", 0) => (200, lease_json("other", "2020-01-01T00:00:00.000000Z")),
                ("PUT", 1) => (200, body.clone()),
                // Second round: someone else renewed in between; our update conflicts.
                ("GET", _) => (200, lease_json("me", "2020-01-01T00:00:00.000000Z")),
                _ => mock::status(409, "Conflict"),
            }
        }));
        let (tx, _rx) = tokio::sync::watch::channel(false);
        let mut cfg = config();
        cfg.retry_period = std::time::Duration::from_millis(10);
        let lost = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            crate::leader::run(client, cfg, tx),
        )
        .await
        .expect("the elector should give up leadership");
        assert!(lost.to_string().contains("lost leadership"));
    }

    #[tokio::test]
    async fn release_clears_the_holder() {
        let (client, log) = mock::client(Box::new(|method, _, body| match method {
            "GET" => (200, lease_json("me", "2026-01-01T00:00:00.000000Z")),
            "PUT" => (200, body.clone()),
            _ => mock::status(400, "BadRequest"),
        }));
        crate::leader::release(client, &config()).await;
        let requests = log.lock().unwrap().clone();
        let put = requests.iter().find(|r| r.method == "PUT").unwrap();
        assert_eq!(put.body["spec"].get("holderIdentity"), None);
    }
}
