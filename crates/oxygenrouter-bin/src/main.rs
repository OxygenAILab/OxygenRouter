//! OxygenRouter CLI binary
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

#![cfg_attr(not(debug_assertions), windows_subsystem = "console")]

use std::sync::Arc;
use std::time::Instant;

use axum::{
    extract::Request,
    http::StatusCode,
    response::{Html, Response},
    routing::get,
    Router,
};
use tower_http::cors::CorsLayer;

#[allow(unused_imports)]
use oxygenrouter_core::save_config;
use oxygenrouter_core::{load_config, ChannelSelector, Database, APP_CONFIG};
use oxygenrouter_proxy::{ChannelScheduler, RelayClient};
use oxygenrouter_webui::{api, proxy, AppState};

#[tokio::main]
async fn main() {
    let start = Instant::now();

    let exe_path = std::env::current_exe().unwrap_or_default();
    let data_dir = exe_path.parent().unwrap_or(&exe_path);
    let db_path = data_dir.join("oxygenrouter.db");
    let config_path = data_dir.join("config.json");

    let config_loaded = match load_config(&config_path) {
        Ok(()) => true,
        Err(e) => {
            eprintln!("[OxygenRouter] config load error: {}", e);
            eprintln!(
                "[OxygenRouter] refusing to write config.json; running with defaults for this session"
            );
            false
        }
    };
    // Only write back when we successfully read (or the file did not exist), so
    // an unreadable file is never clobbered with defaults.
    if config_loaded {
        if let Err(e) = save_config(&config_path) {
            eprintln!("[OxygenRouter] config save error: {}", e);
        }
    }

    let db = match Database::new(&db_path) {
        Ok(d) => Arc::new(d),
        Err(e) => {
            eprintln!("[OxygenRouter] DB init failed: {}", e);
            std::process::exit(1);
        }
    };
    match db.ensure_bootstrap_admin() {
        Ok(Some((username, password))) => {
            println!(
                "[OxygenRouter] Bootstrap admin created. Username: {username} Password: {password}"
            );
        }
        Ok(None) => {}
        Err(e) => eprintln!("[OxygenRouter] Bootstrap admin initialization failed: {e}"),
    }

    if let Ok(Some(saved_token)) = db.get_setting("local_api_token") {
        if !saved_token.is_empty() {
            APP_CONFIG.write().local_api_token = saved_token;
        }
    } else {
        let token = APP_CONFIG.read().local_api_token.clone();
        let _ = db.set_setting("local_api_token", &token);
    }
    if let Ok(Some(saved_theme)) = db.get_setting("theme") {
        if !saved_theme.is_empty() {
            APP_CONFIG.write().theme = saved_theme;
        }
    }
    if let Ok(Some(saved_language)) = db.get_setting("language") {
        if !saved_language.is_empty() {
            APP_CONFIG.write().language = saved_language;
        }
    }

    let selector = ChannelSelector::new(db.clone());
    let (max_retries, user_agent, timeout_ms, backoff_base_ms, immediate_retry) = {
        let cfg = APP_CONFIG.read();
        (
            cfg.max_retries.max(0) as u32,
            if cfg.user_agent.trim().is_empty() {
                format!("OxygenRouter/{}", env!("CARGO_PKG_VERSION"))
            } else {
                cfg.user_agent.clone()
            },
            cfg.upstream_timeout_ms,
            cfg.retry_delay_ms.max(0) as u64,
            // "none" reproduces NewAPI's immediate retry; anything else backs off.
            cfg.retry_backoff.eq_ignore_ascii_case("none"),
        )
    };
    let relay = RelayClient::new(user_agent, timeout_ms);
    let scheduler = ChannelScheduler::new(selector, relay)
        .with_max_retries(max_retries)
        .with_backoff(
            if immediate_retry { 0 } else { backoff_base_ms },
            backoff_base_ms.saturating_mul(60).max(30_000),
        );

    let local_token = APP_CONFIG.read().local_api_token.clone();
    // The concurrency ceiling is enforced globally; NewAPI does not enforce one.
    let max_concurrent = APP_CONFIG.read().max_concurrent_requests;
    let mut state_inner = AppState::new(
        db.clone(),
        db_path.clone(),
        scheduler,
        local_token.clone(),
        config_path.clone(),
    );
    state_inner.configure_limits(max_concurrent, 0);
    // Two logging options are cached because the proxy reads them per request;
    // everything else is derived from the store on demand.
    state_inner.reload_log_policy();
    let state: Arc<AppState> = Arc::new(state_inner);

    state.reload_scheduler_maps().await;
    // Apply this instance's own pricing overrides on top of the shipped pack.
    state.reload_pricing();

    // Retention. `LogRetentionDays` shipped documented as "0 keeps logs forever"
    // and nothing ever pruned, so the table grew without bound. Hourly is often
    // enough for a day-granularity rule and cheap when there is nothing to
    // delete; the first pass runs shortly after start so a restart tidies up.
    {
        let state = state.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(std::time::Duration::from_secs(3600));
            loop {
                ticker.tick().await;
                let days = state
                    .db
                    .get_setting("LogRetentionDays")
                    .ok()
                    .flatten()
                    .and_then(|raw| raw.trim().parse::<i64>().ok())
                    .unwrap_or(30);
                match state.db.prune_request_logs(days) {
                    Ok(0) => {}
                    Ok(removed) => println!(
                        "[OxygenRouter] retention: removed {removed} request logs older than {days}d"
                    ),
                    Err(error) => eprintln!("[OxygenRouter] retention prune failed: {error}"),
                }
            }
        });
    }

    let app = Router::new()
        .route("/", get(index_handler))
        .route("/health", get(health_handler))
        .merge(api::router(state.clone()))
        .merge(proxy::router(state.clone()))
        .route("/ui", get(serve_webui))
        .route("/ui/", get(serve_webui))
        .route("/ui/*path", get(serve_webui))
        .fallback(proxy::fallback_handler)
        .layer(CorsLayer::permissive());

    async fn serve_webui(req: Request) -> Response {
        let raw = req.uri().path();
        let stripped = raw
            .strip_prefix("/ui")
            .unwrap_or(raw)
            .trim_start_matches('/')
            .trim_end_matches('/')
            .to_string();
        eprintln!("[main] serve_webui raw={} stripped={}", raw, stripped);
        oxygenrouter_webui::embed::serve_path(&stripped)
    }

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
    Html(
        r#"<!DOCTYPE html>
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
</html>"#,
    )
}

async fn health_handler() -> StatusCode {
    StatusCode::OK
}
