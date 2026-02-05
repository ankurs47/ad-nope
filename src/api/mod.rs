use crate::config::Config;
use crate::engine::AppState as EngineState;
use axum::{
    extract::{Json as AxumJson, State},
    http::{header, StatusCode, Uri},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use rust_embed::RustEmbed;
use std::sync::Arc;
use tokio::sync::mpsc::Sender;

pub mod db_stats_source;
pub mod in_memory_stats;
pub mod source;

pub use db_stats_source::PersistentStatsSource;
pub use in_memory_stats::InMemoryStatsSource;
pub use source::ApiDataSource;

#[derive(RustEmbed)]
#[folder = "$OUT_DIR/ui"]
struct Asset;

struct ApiState {
    data_source: Arc<dyn ApiDataSource>,
    engine: EngineState,
    config: Config,
    refresh_sender: Sender<()>,
}

pub async fn start_api_server(
    data_source: Arc<dyn ApiDataSource>,
    engine: EngineState,
    config: Config,
    refresh_sender: Sender<()>,
    port: u16,
) {
    let state = Arc::new(ApiState {
        data_source,
        engine,
        config,
        refresh_sender,
    });

    let app = Router::new()
        .route("/api/stats", get(get_stats))
        .route("/api/pause", post(pause_blocking))
        .route("/api/resume", post(resume_blocking))
        .route("/api/status", get(get_status))
        .route("/api/config", get(get_config))
        .route("/api/refresh", post(trigger_refresh))
        .route("/api/logs", get(get_logs))
        .fallback(static_handler)
        .with_state(state);

    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!("API Server listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn get_stats(State(state): State<Arc<ApiState>>) -> impl IntoResponse {
    Json(state.data_source.get_stats().await)
}

async fn get_config(State(state): State<Arc<ApiState>>) -> impl IntoResponse {
    Json(state.config.clone())
}

async fn trigger_refresh(State(state): State<Arc<ApiState>>) -> impl IntoResponse {
    let _ = state.refresh_sender.send(()).await;
    Json(serde_json::json!({ "status": "refresh_triggered" }))
}

async fn get_logs(State(state): State<Arc<ApiState>>) -> impl IntoResponse {
    // Limit to 100 logs for now
    let logs = state.data_source.get_logs(100).await;
    Json(logs)
}

#[derive(serde::Deserialize)]
struct PauseRequest {
    duration_minutes: u64,
}

async fn pause_blocking(
    State(state): State<Arc<ApiState>>,
    AxumJson(payload): AxumJson<PauseRequest>,
) -> impl IntoResponse {
    let duration = std::time::Duration::from_secs(payload.duration_minutes * 60);
    state.engine.pause_blocking(duration);
    Json(serde_json::json!({ "status": "paused", "duration_min": payload.duration_minutes }))
}

async fn resume_blocking(State(state): State<Arc<ApiState>>) -> impl IntoResponse {
    state.engine.resume_blocking();
    Json(serde_json::json!({ "status": "resumed" }))
}

async fn get_status(State(state): State<Arc<ApiState>>) -> impl IntoResponse {
    let active = state.engine.is_blocking_active();
    let remaining = state.engine.get_pause_remaining_secs();
    Json(serde_json::json!({
        "blocking_active": active,
        "pause_remaining_secs": remaining
    }))
}

async fn static_handler(uri: Uri) -> impl IntoResponse {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };

    match Asset::get(path) {
        Some(content) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            ([(header::CONTENT_TYPE, mime.as_ref())], content.data).into_response()
        }
        None => (StatusCode::NOT_FOUND, "404 Not Found").into_response(),
    }
}
