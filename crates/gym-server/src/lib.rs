use axum::{
    extract::{DefaultBodyLimit, Query, State},
    http::{Request, StatusCode},
    middleware::{self, Next},
    response::Response,
    routing::{get, post},
    Json, Router,
};
use gym_core::*;
use gym_storage::Store;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex};
use subtle::ConstantTimeEq;

#[derive(Clone)]
struct AppState {
    store: Arc<Mutex<Store>>,
    token_hash: [u8; 32],
}
pub fn router(store: Store, token: &str) -> Router {
    let state = AppState {
        store: Arc::new(Mutex::new(store)),
        token_hash: Sha256::digest(token.as_bytes()).into(),
    };
    let api = Router::new()
        .route("/api/v1/push", post(push))
        .route("/api/v1/changes", get(changes))
        .route_layer(middleware::from_fn_with_state(state.clone(), auth));
    Router::new()
        .route("/healthz", get(|| async { "ok\n" }))
        .merge(api)
        .layer(DefaultBodyLimit::max(1024 * 1024))
        .with_state(state)
}
async fn auth(
    State(state): State<AppState>,
    request: Request<axum::body::Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    let token = request
        .headers()
        .get("authorization")
        .and_then(|s| s.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .unwrap_or("");
    let supplied: [u8; 32] = Sha256::digest(token.as_bytes()).into();
    if bool::from(supplied.ct_eq(&state.token_hash)) {
        Ok(next.run(request).await)
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}
async fn push(
    State(state): State<AppState>,
    Json(request): Json<PushRequest>,
) -> Result<Json<PushResponse>, StatusCode> {
    if request.operations.is_empty() || request.operations.len() > MAX_BATCH {
        return Err(StatusCode::BAD_REQUEST);
    }
    tokio::task::spawn_blocking(move || {
        let mut store = state
            .store
            .lock()
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let mut outcomes = Vec::with_capacity(request.operations.len());
        for op in request.operations {
            outcomes.push(store.server_apply(&op).map_err(storage_error)?);
        }
        Ok(Json(PushResponse { outcomes }))
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PullQuery {
    #[serde(default)]
    cursor: u64,
    #[serde(default = "page_size")]
    limit: usize,
}
fn page_size() -> usize {
    PAGE_SIZE
}
async fn changes(
    State(state): State<AppState>,
    Query(query): Query<PullQuery>,
) -> Result<Json<Page>, StatusCode> {
    tokio::task::spawn_blocking(move || {
        state
            .store
            .lock()
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .server_page(query.cursor, query.limit)
            .map(Json)
            .map_err(storage_error)
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
}

fn storage_error(error: anyhow::Error) -> StatusCode {
    if error.downcast_ref::<rusqlite::Error>().is_some() {
        eprintln!("gym-server: fallo SQLite; revisa permisos, espacio e integridad (sin registrar el contenido de datos)");
        StatusCode::INTERNAL_SERVER_ERROR
    } else {
        StatusCode::BAD_REQUEST
    }
}
