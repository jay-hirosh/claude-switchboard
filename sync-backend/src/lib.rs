use axum::extract::DefaultBodyLimit;
use axum::{routing::get, Json, Router};
use serde::Serialize;
use sqlx::PgPool;
use std::sync::Arc;

pub mod accounts;
pub mod archive;
pub mod auth;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
}

#[derive(Serialize)]
struct Health {
    status: &'static str,
}

async fn health() -> Json<Health> {
    Json(Health { status: "ok" })
}

pub fn app(pool: PgPool) -> Router {
    use axum::routing::post;
    // Axum's default body-size limit is 2MB, but a full push batch (up to
    // 500 rows, individually tens of KB) can exceed that and would
    // otherwise be silently rejected with 413 — permanently stalling that
    // device's sync, since the watermark-advance rule never skips a failed
    // batch. 32MB is generous headroom above the spec's batch size.
    Router::new()
        .route("/health", get(health))
        .route("/v1/accounts", post(accounts::create_account))
        .route("/v1/devices/pair-code", post(accounts::pair_code))
        .route("/v1/devices/join", post(accounts::join))
        .route("/v1/archive/push", post(archive::push))
        .route("/v1/archive/pull", get(archive::pull))
        .layer(DefaultBodyLimit::max(32 * 1024 * 1024))
        .with_state(Arc::new(AppState { pool }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    #[sqlx::test]
    async fn health_returns_ok(pool: PgPool) {
        let response = app(pool)
            .oneshot(Request::builder().uri("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
    }
}
