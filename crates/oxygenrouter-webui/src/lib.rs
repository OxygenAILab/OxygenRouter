//! oxygenrouter-webui: REST API + static WebUI serving
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

pub mod api;
pub mod artifact_limit;
pub mod billing_store;
pub mod embed;
pub mod model_list;
pub mod perf_metrics;
pub mod pricing_config;
pub mod pricing_public;
pub mod proxy;
pub mod rankings;
pub mod ratio_sync;
mod state;
pub mod system_tasks;
pub mod task_transport;
pub mod uptime;

pub use proxy::fallback_handler;
pub use state::AppState;

use axum::{routing::get, Router};

/// Router that serves the embedded WebUI static assets under `/ui/`.
/// Falls back to `index.html` for unknown sub-paths (SPA routing).
pub fn webui_static_router() -> Router {
    Router::new().fallback(get(embed_handler))
}

async fn embed_handler(uri: axum::http::Uri) -> axum::response::Response {
    embed::serve_uri(&uri)
}
