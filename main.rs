//! OxygenRouter CLI binary
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

#![cfg_attr(not(debug_assertions), windows_subsystem = "console")]

use std::sync::Arc;
use std::time::Instant;

use axum::{Router, routing::get, response::Html, http::StatusCode};
use tower_http::cors::CorsLayer;

use oxygenrouter_core::{Database, ChannelSelector, APP_CONFIG, load_config};
use oxygenrouter_proxy::{UpstreamClient, ChannelScheduler};
use oxygenrouter_webui::{api, proxy, AppState};
#[allow(unused_imports)]
use oxygenrouter_core::save_config;

#[tokio::main]
async fn main() {
    let start = Instant::now();

    let exe_path = std::env::current_exe().unwrap_or_default();
    let data_dir = exe_path.parent().unwrap_or(&exe_path);
    let db_path = data_dir.join("oxygenrouter.db");
    let config_path = data_dir.join("config.json");

    if let Err(e) = load_config(&config_path) {
        eprintln!("[OxygenRouter] config load error (using defaults): {}", e);
    }
    if let Err(e) = save_config(&config_path) {
        eprintln!("[OxygenRouter] config save error: {}", e);
    }

    let db = match Database::new(&db_path) {
        Ok(d) => Arc::new(d),
        Err(e) => {
            eprintln!("[OxygenRouter] DB init failed: {}", e);
            std::process::exit(1);
        }
    };

    if let Ok(Some(saved_token)) = db.get_setting("local_api_token") {
        if !saved_token.is_empty() {
            APP_CONFIG.write().local_api_token = saved_token;
        }
    } else {
        let token = APP_CONFIG.read().local_api_token.clone();
        let _ = db.set_setting("local_api_token", &token);
    }

    let selector = ChannelSelector::new(db.clone());
    let upstream = UpstreamClient::new();
    let max_retries = APP_CONFIG.read().max_retries as u32;
    let scheduler = ChannelScheduler::new(selector, upstream).with_max_retries(max_retries);

    let local_token = APP_CONFIG.read().local_api_token.clone();
    let state: Arc<AppState> = Arc::new(AppState::new(db.clone(), scheduler, local_token.clone(), config_path.clone()));

    state.reload_scheduler_maps().await;

    let app = Router::new()
        .route("/", get(index_handler))
        .route("/health", get(health_handler))
        .nest("/ui", oxygenrouter_webui::webui_static_router())
        .merge(api::router(state.clone()))
        .merge(proxy::router(state.clone()))
        .layer(CorsLayer::permissive());

    let addr = format!(
        "{}:{}",
        APP_CONFIG.read().listen_host,
        APP_CONFIG.read().listen_port
    );

    println!("═══════════════════════════════════════════");
    println!("  O₂ OxygenRouter  v{}", env!("CARGO_PKG_VERSION"));
    println!("  GitHub@OxygenAILab | OxygenAILab@StarsailsClover");
    println!("═══════════════════════════════════════════");
    println!("  Local API Token: {}", APP_CONFIG.read().local_api_token);
    println!("  WebUI:           http://{}/", addr);
    println!("  Proxy:           http://{}/v1/chat/completions", addr);
    println!("  DB:              {}", db_path.display());
    println!("  Config:          {}", config_path.display());
    println!("═══════════════════════════════════════════");

    let url = format!("http://{}/", addr);

    if APP_CONFIG.read().open_browser_on_start {
        println!("[OxygenRouter] Opening browser...");
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(800));
            #[cfg(windows)]
            {
                std::process::Command::new("cmd")
                    .args(["/C", "start", "", &url])
                    .spawn()
                    .ok();
            }
            #[cfg(not(windows))]
            {
                open::that(&url).ok();
            }
        });
    }

    let listener = match tokio::net::TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("[OxygenRouter] Bind {} failed: {}", addr, e);
            std::process::exit(1);
        }
    };

    println!("[OxygenRouter] Listening on {}", addr);
    println!("[OxygenRouter] Uptime tracker started (start={:?})", start);

    if let Err(e) = axum::serve(listener, app).await {
        eprintln!("[OxygenRouter] Server error: {}", e);
    }
}

async fn index_handler() -> Html<&'static str> {
    Html(r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8" />
<meta name="viewport" content="width=device-width, initial-scale=1.0" />
<title>OxygenRouter</title>
<style>
  *{margin:0;padding:0;box-sizing:border-box}
  body{font-family:'Inter',-apple-system,sans-serif;background:#0a0a0a;color:#fafafa;display:flex;align-items:center;justify-content:center;min-height:100vh}
  .card{background:#141414;border:1px solid #27272a;border-radius:12px;padding:40px;max-width:480px;width:90%;text-align:center}
  .logo{font-size:32px;font-weight:700;margin-bottom:8px}
  .logo span{color:#71717a}
  .subtitle{color:#71717a;font-size:13px;margin-bottom:32px}
  .btn{display:inline-block;background:#fafafa;color:#0a0a0a;padding:10px 24px;border-radius:8px;text-decoration:none;font-weight:500;margin:8px}
  .btn:hover{opacity:0.9}
  .info{background:#1c1c1f;border-radius:8px;padding:16px;margin-top:24px;text-align:left;font-size:13px;font-family:'SF Mono','JetBrains Mono',monospace}
  .info p{margin:4px 0;word-break:break-all}
  .label{color:#71717a}
</style>
</head>
<body>
<div class="card">
  <div class="logo">O₂ <span>OxygenRouter</span></div>
  <div class="subtitle">OpenAI-compatible API gateway · v26.0 Alpha</div>
  <a href="/ui/" class="btn">Open WebUI</a>
  <a href="/docs" class="btn">API Docs</a>
  <div class="info">
    <p><span class="label">Base URL  </span>http://localhost:3001/v1/chat/completions</p>
    <p><span class="label">Token     </span><span id="token">loading...</span></p>
    <p><span class="label">Endpoints </span>/v1/chat/completions · /v1/completions · /v1/embeddings</p>
  </div>
</div>
<script>
fetch('/api/status').then(r=>r.json()).then(d=>{
  if(d.data && d.data.local_api_token) document.getElementById('token').textContent = d.data.local_api_token;
}).catch(()=>{});
</script>
</body>
</html>"#)
}

async fn health_handler() -> StatusCode {
    StatusCode::OK
}
