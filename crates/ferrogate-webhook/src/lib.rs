// SPDX-License-Identifier: Apache-2.0

//! Ferrogate admission webhook.
//!
//! Phase A provides the routes only; the server is not started by the `ferrogate` binary yet.
//! Phase B adds `AdmissionReview` handling and TLS (the webhook is HTTPS-only by design, see
//! `docs/SECURITY_ARCHITECTURE.md`).

use axum::Router;
use axum::http::StatusCode;
use axum::routing::{get, post};

/// Routes served by the webhook.
pub fn router() -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/validate", post(validate))
}

async fn healthz() -> &'static str {
    "ok"
}

/// Not implemented yet: answers 501 so a misconfigured cluster never mistakes it for "allowed".
async fn validate() -> (StatusCode, &'static str) {
    (
        StatusCode::NOT_IMPLEMENTED,
        "admission validation is not implemented yet",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    async fn call(method: &str, uri: &str) -> (StatusCode, String) {
        let request = Request::builder()
            .method(method)
            .uri(uri)
            .body(Body::empty())
            .unwrap();
        let response = router().oneshot(request).await.unwrap();
        let status = response.status();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8(body.to_vec()).unwrap())
    }

    #[tokio::test]
    async fn healthz_is_ok() {
        assert_eq!(call("GET", "/healthz").await, (StatusCode::OK, "ok".into()));
    }

    #[tokio::test]
    async fn validate_is_not_implemented() {
        let (status, _) = call("POST", "/validate").await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
    }
}
