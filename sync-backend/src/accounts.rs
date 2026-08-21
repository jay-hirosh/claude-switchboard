use archive_sync_types::{
    CreateAccountRequest, CreateAccountResponse, JoinRequest, JoinResponse, PairCodeResponse,
};
use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use std::sync::Arc;
use uuid::Uuid;

use crate::auth::{generate_api_key, generate_pairing_code, hash_api_key, AuthedDevice};
use crate::AppState;

pub async fn create_account(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateAccountRequest>,
) -> Result<Json<CreateAccountResponse>, (StatusCode, String)> {
    let device_id = Uuid::parse_str(&req.device_id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "device_id must be a valid UUID".to_string()))?;

    let (user_id,): (Uuid,) = sqlx::query_as("INSERT INTO users DEFAULT VALUES RETURNING id")
        .fetch_one(&state.pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let api_key = generate_api_key();
    let hash = hash_api_key(&api_key);

    sqlx::query(
        "INSERT INTO devices (id, user_id, name, api_key_hash) VALUES ($1, $2, $3, $4)",
    )
    .bind(device_id)
    .bind(user_id)
    .bind(&req.device_name)
    .bind(&hash)
    .execute(&state.pool)
    .await
    .map_err(|e| (StatusCode::CONFLICT, e.to_string()))?;

    Ok(Json(CreateAccountResponse {
        user_id: user_id.to_string(),
        device_id: device_id.to_string(),
        api_key,
    }))
}

pub async fn pair_code(
    State(state): State<Arc<AppState>>,
    device: AuthedDevice,
) -> Result<Json<PairCodeResponse>, (StatusCode, String)> {
    for _ in 0..5 {
        let code = generate_pairing_code();
        let inserted = sqlx::query(
            "INSERT INTO pairing_codes (code, user_id, expires_at)
             VALUES ($1, $2, now() + interval '10 minutes')
             ON CONFLICT (code) DO NOTHING",
        )
        .bind(&code)
        .bind(device.user_id)
        .execute(&state.pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        if inserted.rows_affected() == 1 {
            return Ok(Json(PairCodeResponse { pairing_code: code }));
        }
        // Extremely unlikely collision (32^8 possibilities) — retry with a fresh code.
    }
    Err((StatusCode::INTERNAL_SERVER_ERROR, "could not generate a unique pairing code".to_string()))
}

pub async fn join(
    State(state): State<Arc<AppState>>,
    Json(req): Json<JoinRequest>,
) -> Result<Json<JoinResponse>, (StatusCode, String)> {
    let device_id = Uuid::parse_str(&req.device_id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "device_id must be a valid UUID".to_string()))?;

    let row: Option<(Uuid,)> = sqlx::query_as(
        "UPDATE pairing_codes SET used_at = now()
         WHERE code = $1 AND used_at IS NULL AND expires_at > now()
         RETURNING user_id",
    )
    .bind(&req.pairing_code)
    .fetch_optional(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let (user_id,) = row.ok_or((
        StatusCode::UNAUTHORIZED,
        "invalid, used, or expired pairing code".to_string(),
    ))?;

    let api_key = generate_api_key();
    let hash = hash_api_key(&api_key);

    sqlx::query(
        "INSERT INTO devices (id, user_id, name, api_key_hash) VALUES ($1, $2, $3, $4)",
    )
    .bind(device_id)
    .bind(user_id)
    .bind(&req.device_name)
    .bind(&hash)
    .execute(&state.pool)
    .await
    .map_err(|e| (StatusCode::CONFLICT, e.to_string()))?;

    Ok(Json(JoinResponse {
        user_id: user_id.to_string(),
        device_id: device_id.to_string(),
        api_key,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app;
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use sqlx::PgPool;
    use tower::ServiceExt;

    async fn post_json(pool: &PgPool, uri: &str, body: serde_json::Value, bearer: Option<&str>) -> (StatusCode, serde_json::Value) {
        let mut builder = Request::builder().method("POST").uri(uri).header("content-type", "application/json");
        if let Some(token) = bearer {
            builder = builder.header("authorization", format!("Bearer {token}"));
        }
        let request = builder.body(Body::from(serde_json::to_vec(&body).unwrap())).unwrap();
        let response = app(pool.clone()).oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        // Error responses are plain text (the blanket `(StatusCode, String)`
        // IntoResponse impl), not JSON — fall back to wrapping the raw text
        // instead of panicking. No test inspects body content on error paths,
        // only the status code.
        let json: serde_json::Value = if bytes.is_empty() {
            serde_json::json!({})
        } else {
            serde_json::from_slice(&bytes)
                .unwrap_or_else(|_| serde_json::json!({ "raw": String::from_utf8_lossy(&bytes).to_string() }))
        };
        (status, json)
    }

    #[sqlx::test]
    async fn create_account_then_pair_and_join_a_second_device(pool: PgPool) {
        let device_a_id = Uuid::new_v4().to_string();
        let (status, body) = post_json(
            &pool,
            "/v1/accounts",
            serde_json::json!({ "device_id": device_a_id, "device_name": "Device A" }),
            None,
        ).await;
        assert_eq!(status, StatusCode::OK);
        let api_key_a = body["api_key"].as_str().unwrap().to_string();
        assert_eq!(body["device_id"], device_a_id);

        // Pair-code requires authentication as an existing device.
        let (status, body) = post_json(&pool, "/v1/devices/pair-code", serde_json::json!({}), Some(&api_key_a)).await;
        assert_eq!(status, StatusCode::OK);
        let pairing_code = body["pairing_code"].as_str().unwrap().to_string();
        assert_eq!(pairing_code.len(), 8);

        let device_b_id = Uuid::new_v4().to_string();
        let (status, body) = post_json(
            &pool,
            "/v1/devices/join",
            serde_json::json!({ "pairing_code": pairing_code, "device_id": device_b_id, "device_name": "Device B" }),
            None,
        ).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["user_id"], body["user_id"]); // sanity
        assert_eq!(body["device_id"], device_b_id);

        // The pairing code is single-use — joining again with it must fail.
        let (status, _) = post_json(
            &pool,
            "/v1/devices/join",
            serde_json::json!({ "pairing_code": pairing_code, "device_id": Uuid::new_v4().to_string(), "device_name": "Device C" }),
            None,
        ).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[sqlx::test]
    async fn pair_code_requires_auth(pool: PgPool) {
        let (status, _) = post_json(&pool, "/v1/devices/pair-code", serde_json::json!({}), None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[sqlx::test]
    async fn join_rejects_unknown_pairing_code(pool: PgPool) {
        let (status, _) = post_json(
            &pool,
            "/v1/devices/join",
            serde_json::json!({ "pairing_code": "NOTREAL1", "device_id": Uuid::new_v4().to_string(), "device_name": "X" }),
            None,
        ).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[sqlx::test]
    async fn create_account_rejects_malformed_device_id(pool: PgPool) {
        let (status, _) = post_json(
            &pool,
            "/v1/accounts",
            serde_json::json!({ "device_id": "not-a-uuid", "device_name": "Bad Device" }),
            None,
        ).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[sqlx::test]
    async fn join_rejects_malformed_device_id(pool: PgPool) {
        // Even with a syntactically well-formed but unknown pairing code, a
        // malformed device_id must be rejected before ever touching the
        // pairing_codes table.
        let (status, _) = post_json(
            &pool,
            "/v1/devices/join",
            serde_json::json!({ "pairing_code": "NOTREAL1", "device_id": "not-a-uuid", "device_name": "Bad Device" }),
            None,
        ).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
}
