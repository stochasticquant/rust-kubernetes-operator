// SPDX-License-Identifier: Apache-2.0

//! A scripted fake API server for controller tests.

use std::sync::{Arc, Mutex};

use http::{Request, Response};
use http_body_util::BodyExt;
use kube::Client;
use kube::client::Body;
use serde_json::{Value, json};

/// One request the code under test sent.
#[derive(Clone, Debug)]
pub struct Recorded {
    pub method: String,
    pub path: String,
    pub body: Value,
}

/// Decides the answer to a request: status code and JSON body.
pub type Responder = Box<dyn Fn(&str, &str, &Value) -> (u16, Value) + Send + Sync>;

/// A Kubernetes `Status` error body.
pub fn status(code: u16, reason: &str) -> (u16, Value) {
    (
        code,
        json!({
            "kind": "Status", "apiVersion": "v1", "metadata": {},
            "status": "Failure", "reason": reason, "code": code, "message": reason
        }),
    )
}

/// A client whose requests are answered by `responder`, and the log of those requests.
pub fn client(responder: Responder) -> (Client, Arc<Mutex<Vec<Recorded>>>) {
    let (service, mut handle) = tower_test::mock::pair::<Request<Body>, Response<Body>>();
    let log = Arc::new(Mutex::new(Vec::new()));
    let recorded = log.clone();
    tokio::spawn(async move {
        while let Some((request, send)) = handle.next_request().await {
            let (parts, body) = request.into_parts();
            let bytes = body
                .collect()
                .await
                .map(http_body_util::Collected::to_bytes)
                .unwrap_or_default();
            let value: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
            let path = parts.uri.path().to_owned();
            let (code, answer) = responder(parts.method.as_str(), &path, &value);
            recorded.lock().unwrap().push(Recorded {
                method: parts.method.to_string(),
                path,
                body: value,
            });
            let response = Response::builder()
                .status(code)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&answer).unwrap()))
                .unwrap();
            send.send_response(response);
        }
    });
    (Client::new(service, "default"), log)
}
