use axum::Router;
use axum::routing::post;

pub fn router() -> Router {
    Router::new().route("/validate", post(validate))
}

async fn validate() {
    println!("Validation request received");
}
