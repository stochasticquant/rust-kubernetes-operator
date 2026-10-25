// SPDX-License-Identifier: Apache-2.0

//! Ferrogate admission webhook and the operational HTTP endpoints.
//!
//! Two servers:
//! - the admission server (HTTPS only, `POST /validate`), called by the Kubernetes API server;
//! - the operations server (plain HTTP, `/healthz`, `/readyz`, `/metrics`) for probes and
//!   Prometheus. It serves no request data.
//!
//! Security notes (see `docs/SECURITY_ARCHITECTURE.md` and `docs/security/threat-model.md`):
//! request bodies are size-limited, handlers time out before the API server does, concurrency is
//! capped, and object contents are never logged.

pub mod metrics;
pub mod review;
pub mod tls;

use std::collections::BTreeMap;
use std::future::Future;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use ferrogate_core::Policy;
use prometheus_client::registry::Registry;
use tower::limit::ConcurrencyLimitLayer;
use tower_http::timeout::TimeoutLayer;

pub use metrics::WebhookMetrics;
pub use review::{AdmissionResponse, Outcome};
pub use tls::TlsFiles;

/// Default limit for an `AdmissionReview` body. Kubernetes objects are at most ~3 MiB, and a review
/// can carry the old and the new object.
pub const DEFAULT_MAX_BODY_BYTES: usize = 7 * 1024 * 1024;
/// Handlers give up before the API server does (`timeoutSeconds: 5` in the shipped configuration).
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(4);
/// Maximum admission requests handled at once.
pub const DEFAULT_CONCURRENCY: usize = 512;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("read {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("invalid PEM: {0}")]
    Pem(String),
    #[error("TLS configuration: {0}")]
    Tls(#[from] rustls::Error),
    #[error("server: {0}")]
    Io(#[from] std::io::Error),
}

/// Where the webhook gets policies and namespace labels from. Implemented over Kubernetes watch
/// caches by `ferrogate-operator`, and by fixed data in tests.
pub trait PolicySource: Send + Sync + 'static {
    /// The current valid policies.
    fn policies(&self) -> Arc<Vec<Policy>>;
    /// Labels of a namespace, if known.
    fn namespace_labels(&self, namespace: &str) -> Option<BTreeMap<String, String>>;
    /// True once the caches have synced. Until then the webhook answers 503 so the API server
    /// applies the configured `failurePolicy`.
    fn is_ready(&self) -> bool;
}

/// Limits for the admission server.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_body_bytes: usize,
    pub timeout: Duration,
    pub concurrency: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_body_bytes: DEFAULT_MAX_BODY_BYTES,
            timeout: DEFAULT_TIMEOUT,
            concurrency: DEFAULT_CONCURRENCY,
        }
    }
}

#[derive(Clone)]
struct AdmissionState {
    source: Arc<dyn PolicySource>,
    metrics: WebhookMetrics,
}

/// Routes of the admission server.
pub fn admission_router(
    source: Arc<dyn PolicySource>,
    metrics: WebhookMetrics,
    limits: Limits,
) -> Router {
    Router::new()
        .route("/validate", post(validate))
        .route("/healthz", get(|| async { "ok" }))
        .layer(DefaultBodyLimit::max(limits.max_body_bytes))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            limits.timeout,
        ))
        .layer(ConcurrencyLimitLayer::new(limits.concurrency))
        .with_state(AdmissionState { source, metrics })
}

async fn validate(State(state): State<AdmissionState>, body: Bytes) -> Response {
    let started = Instant::now();
    let review: review::AdmissionReview = match serde_json::from_slice(&body) {
        Ok(review) => review,
        Err(error) => {
            state.metrics.error();
            tracing::warn!(%error, "rejected malformed AdmissionReview");
            return (StatusCode::BAD_REQUEST, "malformed AdmissionReview").into_response();
        }
    };
    let Some(request) = review
        .request
        .filter(|_| review.api_version == review::API_VERSION && review.kind == review::KIND)
    else {
        state.metrics.error();
        return (
            StatusCode::BAD_REQUEST,
            "expected an admission.k8s.io/v1 AdmissionReview with a request",
        )
            .into_response();
    };
    if !state.source.is_ready() {
        state.metrics.error();
        tracing::warn!(uid = %request.uid, "not ready; letting the API server apply failurePolicy");
        return (StatusCode::SERVICE_UNAVAILABLE, "ferrogate is not ready").into_response();
    }

    let policies = state.source.policies();
    let labels = request
        .namespace
        .as_deref()
        .and_then(|ns| state.source.namespace_labels(ns));
    let (response, outcome, decision) = review::admit(&request, &policies, labels.as_ref());

    let failed: Vec<&str> = decision.failures().map(|r| r.policy.as_str()).collect();
    // Identity and verdict only: object contents are never logged (threat T-30).
    tracing::info!(
        uid = %request.uid,
        operation = %request.operation,
        kind = %request.kind.kind,
        namespace = request.namespace.as_deref().unwrap_or(""),
        name = request.name.as_deref().unwrap_or(""),
        user = %request.user_info.username,
        dry_run = request.dry_run.unwrap_or(false),
        outcome = outcome.as_str(),
        failed_policies = ?failed,
        "admission decision"
    );
    state
        .metrics
        .record(outcome, started.elapsed().as_secs_f64(), &decision);
    axum::Json(response.wrap()).into_response()
}

/// Readiness check used by `/readyz`.
pub type ReadyCheck = Arc<dyn Fn() -> bool + Send + Sync>;

/// Routes of the operations server: liveness, readiness and Prometheus metrics.
pub fn ops_router(registry: Arc<Registry>, ready: ReadyCheck) -> Router {
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route(
            "/readyz",
            get(move || {
                let ready = ready();
                async move {
                    if ready {
                        (StatusCode::OK, "ready")
                    } else {
                        (StatusCode::SERVICE_UNAVAILABLE, "not ready")
                    }
                }
            }),
        )
        .route(
            "/metrics",
            get(move || {
                let registry = registry.clone();
                async move { metrics_response(&registry) }
            }),
        )
}

fn metrics_response(registry: &Registry) -> Response {
    let mut body = String::new();
    match prometheus_client::encoding::text::encode(&mut body, registry) {
        Ok(()) => (
            [(
                header::CONTENT_TYPE,
                "application/openmetrics-text; version=1.0.0; charset=utf-8",
            )],
            body,
        )
            .into_response(),
        Err(error) => {
            tracing::warn!(%error, "could not encode metrics");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// Serves `router` over HTTPS until `shutdown` completes, then drains for up to 10 seconds.
pub async fn serve_tls(
    addr: SocketAddr,
    router: Router,
    files: TlsFiles,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<(), Error> {
    let config = tls::load_and_watch(files)?;
    let handle = axum_server::Handle::<SocketAddr>::new();
    let on_shutdown = handle.clone();
    tokio::spawn(async move {
        shutdown.await;
        on_shutdown.graceful_shutdown(Some(Duration::from_secs(10)));
    });
    tracing::info!(%addr, "admission server listening (HTTPS)");
    axum_server::bind_rustls(addr, config)
        .handle(handle)
        .serve(router.into_make_service())
        .await?;
    Ok(())
}

/// Serves `router` over plain HTTP until `shutdown` completes.
pub async fn serve_plain(
    addr: SocketAddr,
    router: Router,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<(), Error> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(%addr, "operations server listening (HTTP: /healthz, /readyz, /metrics)");
    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use ferrogate_core::PolicySpec;
    use http_body_util::BodyExt;
    use serde_json::{Value, json};
    use std::sync::atomic::{AtomicBool, Ordering};
    use tower::ServiceExt;

    struct Fixed {
        policies: Arc<Vec<Policy>>,
        ready: AtomicBool,
    }

    impl PolicySource for Fixed {
        fn policies(&self) -> Arc<Vec<Policy>> {
            self.policies.clone()
        }
        fn namespace_labels(&self, _: &str) -> Option<BTreeMap<String, String>> {
            None
        }
        fn is_ready(&self) -> bool {
            self.ready.load(Ordering::Relaxed)
        }
    }

    fn source(ready: bool) -> Arc<Fixed> {
        let spec: PolicySpec = serde_json::from_value(json!({
            "mode": "Enforce",
            "match": {"resources": [{"kinds": ["Pod"]}]},
            "rules": [{"name": "no-latest", "disallowLatestTag": true}]
        }))
        .unwrap();
        Arc::new(Fixed {
            policies: Arc::new(vec![Policy::compile("tags", &spec).unwrap()]),
            ready: AtomicBool::new(ready),
        })
    }

    fn review(image: &str) -> Value {
        json!({
            "apiVersion": "admission.k8s.io/v1",
            "kind": "AdmissionReview",
            "request": {
                "uid": "123",
                "kind": {"group": "", "version": "v1", "kind": "Pod"},
                "namespace": "default",
                "operation": "CREATE",
                "userInfo": {"username": "alice"},
                "object": {"spec": {"containers": [{"name": "a", "image": image}]}}
            }
        })
    }

    async fn post(router: Router, body: String) -> (StatusCode, String) {
        let request = Request::post("/validate")
            .header("content-type", "application/json")
            .body(Body::from(body))
            .unwrap();
        let response = router.oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    fn router(source: Arc<Fixed>, limits: Limits) -> Router {
        let mut registry = Registry::default();
        admission_router(source, WebhookMetrics::register(&mut registry), limits)
    }

    #[tokio::test]
    async fn denies_and_allows_over_http() {
        let (status, body) = post(
            router(source(true), Limits::default()),
            review("nginx").to_string(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let json: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(json["response"]["uid"], "123");
        assert_eq!(json["response"]["allowed"], false);

        let (_, body) = post(
            router(source(true), Limits::default()),
            review("nginx:1").to_string(),
        )
        .await;
        let json: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(json["response"]["allowed"], true);
    }

    #[tokio::test]
    async fn not_ready_returns_503() {
        let (status, _) = post(
            router(source(false), Limits::default()),
            review("nginx").to_string(),
        )
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn malformed_and_wrong_version_return_400() {
        let r = || router(source(true), Limits::default());
        assert_eq!(
            post(r(), "{not json".into()).await.0,
            StatusCode::BAD_REQUEST
        );
        let mut v1beta1 = review("nginx");
        v1beta1["apiVersion"] = json!("admission.k8s.io/v1beta1");
        assert_eq!(
            post(r(), v1beta1.to_string()).await.0,
            StatusCode::BAD_REQUEST
        );
        let no_request = json!({"apiVersion": "admission.k8s.io/v1", "kind": "AdmissionReview"});
        assert_eq!(
            post(r(), no_request.to_string()).await.0,
            StatusCode::BAD_REQUEST
        );
    }

    #[tokio::test]
    async fn oversized_bodies_are_rejected() {
        let limits = Limits {
            max_body_bytes: 1024,
            ..Limits::default()
        };
        let mut big = review("nginx");
        big["request"]["object"]["metadata"] = json!({"annotations": {"x": "y".repeat(4096)}});
        let (status, _) = post(router(source(true), limits), big.to_string()).await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn ops_endpoints() {
        let mut registry = Registry::default();
        let metrics = WebhookMetrics::register(&mut registry);
        // Record one decision so the counters appear.
        let admission = admission_router(source(true), metrics, Limits::default());
        post(admission, review("nginx").to_string()).await;

        let ready = Arc::new(AtomicBool::new(false));
        let flag = ready.clone();
        let ops = ops_router(
            Arc::new(registry),
            Arc::new(move || flag.load(Ordering::Relaxed)),
        );
        let get = |uri: &str| Request::get(uri).body(Body::empty()).unwrap();

        assert_eq!(
            ops.clone().oneshot(get("/healthz")).await.unwrap().status(),
            StatusCode::OK
        );
        assert_eq!(
            ops.clone().oneshot(get("/readyz")).await.unwrap().status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
        ready.store(true, Ordering::Relaxed);
        assert_eq!(
            ops.clone().oneshot(get("/readyz")).await.unwrap().status(),
            StatusCode::OK
        );

        let response = ops.oneshot(get("/metrics")).await.unwrap();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let text = String::from_utf8_lossy(&body);
        assert!(
            text.contains("ferrogate_admission_requests_total{outcome=\"denied\"} 1"),
            "{text}"
        );
        assert!(text.contains("ferrogate_admission_violations_total{policy=\"tags\",mode=\"Enforce\",severity=\"Medium\"} 1"), "{text}");
    }

    #[test]
    fn tls_config_rejects_garbage_and_accepts_generated_cert() {
        assert!(tls::server_config(b"nope", b"nope").is_err());
        let cert =
            rcgen::generate_simple_self_signed(vec!["ferrogate-webhook.ferrogate.svc".into()])
                .unwrap();
        let config = tls::server_config(
            cert.cert.pem().as_bytes(),
            cert.signing_key.serialize_pem().as_bytes(),
        )
        .unwrap();
        assert_eq!(config.alpn_protocols[0], b"h2");
    }
}
