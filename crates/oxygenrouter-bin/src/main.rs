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

    // One-time migration into the single authority.
    //
    // The instance's token and the two display preferences used to live in
    // `config.json`, under the same names but in a different store, and were
    // copied into `settings` at every start. `settings` is now the authority, so
    // the copy runs once: if the table has nothing yet, the config file's value
    // is carried over and then never consulted again. Later edits go through the
    // console and cannot be overwritten by a stale file.
    let bootstrap = {
        let cfg = APP_CONFIG.read();
        vec![
            ("LocalApiToken", cfg.local_api_token.clone()),
            ("Theme", cfg.theme.clone()),
            ("Language", cfg.language.clone()),
            // These were previously read straight from the config file at
            // startup, so an instance that has been running since before this
            // change has its real values there and nowhere else. Seeding them is
            // what stops the switch to the table from silently binding a
            // different port than the operator configured.
            ("ListenHost", cfg.listen_host.clone()),
            ("ListenPort", cfg.listen_port.to_string()),
            (
                "MaxConcurrentRequests",
                cfg.max_concurrent_requests.to_string(),
            ),
            ("UpstreamTimeoutMs", cfg.upstream_timeout_ms.to_string()),
            ("UserAgent", cfg.user_agent.clone()),
            ("LogLevel", cfg.log_level.clone()),
            ("RetryTimes", cfg.max_retries.to_string()),
            ("RetryIntervalMs", cfg.retry_delay_ms.to_string()),
            ("RetryBackoff", cfg.retry_backoff.clone()),
            (
                "LogRetentionDays",
                cfg.request_log_retention_days.to_string(),
            ),
        ]
    };
    for (key, value) in bootstrap {
        let stored = db
            .get_setting(key)
            .ok()
            .flatten()
            .filter(|v| !v.trim().is_empty());
        if let Some(_existing) = stored {
            continue;
        }
        if !value.trim().is_empty() {
            let _ = db.set_setting(key, &value);
        }
    }

    let selector = ChannelSelector::new(db.clone());
    // Read from the settings table, which is now the single authority. Each key
    // falls back to its schema default, so an unset table behaves like a fresh
    // install rather than an empty configuration.
    let (max_retries, user_agent, timeout_ms, backoff_base_ms, immediate_retry) = {
        let max_retries = db.typed_setting("RetryTimes", 3_i64).max(0) as u32;
        let configured_agent: String = db.typed_setting("UserAgent", String::new());
        let user_agent = if configured_agent.trim().is_empty() {
            format!("OxygenRouter/{}", env!("CARGO_PKG_VERSION"))
        } else {
            configured_agent
        };
        let timeout_ms = db.typed_setting("UpstreamTimeoutMs", 120_000_u64);
        let backoff_base_ms = db.typed_setting("RetryIntervalMs", 500_i64).max(0) as u64;
        // "none" reproduces NewAPI's immediate retry; anything else backs off.
        let immediate_retry = db
            .typed_setting("RetryBackoff", "exponential".to_string())
            .eq_ignore_ascii_case("none");
        (
            max_retries,
            user_agent,
            timeout_ms,
            backoff_base_ms,
            immediate_retry,
        )
    };
    let relay = RelayClient::new(user_agent, timeout_ms);
    let scheduler = ChannelScheduler::new(selector, relay)
        .with_max_retries(max_retries)
        .with_backoff(
            if immediate_retry { 0 } else { backoff_base_ms },
            backoff_base_ms.saturating_mul(60).max(30_000),
        )
        // `ChannelDisableThreshold` was advertised and read by nothing: the
        // scheduler carried a builder for it and always ran on the hard-coded
        // default, so an operator could not make a flaky channel stand down
        // sooner or later than the shipped five.
        .with_disable_threshold(db.typed_setting("ChannelDisableThreshold", 5_i64).max(0) as u32);

    // The local token is now read from the table; the seeded value above is what
    // carries an existing instance's token across.
    let local_token: String = db.typed_setting("LocalApiToken", String::new());
    // The concurrency ceiling is enforced globally; NewAPI does not enforce one.
    let max_concurrent = db.typed_setting("MaxConcurrentRequests", 64_i32);
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
                // The audit trail shares the window. A trail that outlived the
                // retention its operator configured would be the opposite of
                // auditable.
                match state.db.prune_audit_logs(days) {
                    Ok(0) => {}
                    Ok(removed) => println!(
                        "[OxygenRouter] retention: removed {removed} audit rows older than {days}d"
                    ),
                    Err(error) => eprintln!("[OxygenRouter] audit prune failed: {error}"),
                }
            }
        });
    }

    /// How often a task is polled.
    ///
    /// The reference exposes no per-instance interval for this pass, so neither
    /// does this: a knob the reference lacks would make the two behave differently
    /// with nothing to compare against.
    const TASK_POLL_SECONDS: u64 = 5;

    // Task polling. A plugin-backed protocol is `fetchMode: per_task`: the
    // submission stores a task and the work finishes upstream, so something has to
    // ask. This is that something, and it is the reason a task bridge is worth
    // having rather than a synchronous call with a long timeout.
    //
    // The wait is sliced and the interval is re-read on every tick, for the same
    // reason the retention and model-sync tasks above do it: a single long sleep
    // would read the interval before sleeping, so shortening it would take effect
    // only after the old, longer one elapsed.
    {
        let state = state.clone();
        tokio::spawn(async move {
            // A one-second floor, so the due check is responsive without spinning.
            let tick = std::time::Duration::from_secs(1);
            let mut ticker = tokio::time::interval(tick);
            let mut last_run: Option<std::time::Instant> = None;
            loop {
                ticker.tick().await;
                // A fixed cadence rather than an option: the reference's poll pass
                // is driven by its system-task runner and exposes no per-instance
                // interval for this. Inventing one would give an operator a knob
                // the reference does not have -- exactly the divergence this
                // project keeps removing.
                let due = last_run
                    .map(|last| last.elapsed() >= std::time::Duration::from_secs(TASK_POLL_SECONDS))
                    .unwrap_or(true);
                if !due {
                    continue;
                }
                last_run = Some(std::time::Instant::now());
                let now = chrono::Utc::now().timestamp();
                let (polled, advanced) = oxygenrouter_webui::proxy::poll_tasks_once(&state, now).await;
                if advanced > 0 {
                    println!("[OxygenRouter] tasks: {advanced} of {polled} polled tasks advanced");
                }
            }
        });
    }

    // Usage summary flush. `DataExportEnabled` promises "Aggregate usage into
    // quota_data for analytics"; the counters live in memory on the relay path and
    // are written out here, so a busy gateway pays one transaction every few
    // minutes rather than one per request. A drain that fails leaves the counters
    // gone rather than duplicated, which is the safer direction for a chart.
    {
        let state = state.clone();
        tokio::spawn(async move {
            // Sliced wait, for the reason given on the model-sync task above:
            // reading the interval before a single long sleep would make a
            // shorter setting take effect only after the old one elapsed.
            let mut ticker = tokio::time::interval(std::time::Duration::from_secs(5));
            let mut last_flush: Option<std::time::Instant> = None;
            loop {
                ticker.tick().await;
                let minutes = state
                    .db
                    .get_setting("DataExportInterval")
                    .ok()
                    .flatten()
                    .and_then(|raw| raw.trim().parse::<u64>().ok())
                    .filter(|m| *m >= 1)
                    .unwrap_or(5);
                let due = last_flush
                    .map(|last| last.elapsed() >= std::time::Duration::from_secs(minutes * 60))
                    .unwrap_or(true);
                if !due {
                    continue;
                }
                last_flush = Some(std::time::Instant::now());
                let rows = state.usage.take();
                if rows.is_empty() {
                    continue;
                }
                let count = rows.len();
                match state.db.upsert_quota_data(&rows) {
                    Ok(()) => println!("[OxygenRouter] usage summary: flushed {count} buckets"),
                    Err(error) => eprintln!("[OxygenRouter] usage summary flush failed: {error}"),
                }
            }
        });
    }

    // Channel model refresh. `UpstreamModelSyncEnabled` and
    // `ModelSyncIntervalMinutes` were advertised and read by nothing, so a
    // channel's model list only ever changed when an operator pressed the button.
    //
    // The wait is sliced rather than taken in one sleep. A single
    // `sleep(minutes * 60)` reads the interval *before* sleeping, so shortening
    // it while the task is asleep would not take effect until the old, longer
    // sleep finished — with the shipped default that is an hour of the console
    // saying one thing and the task doing another. Re-evaluating on a short tick
    // costs two tiny reads every five seconds and makes a change visible promptly.
    {
        let state = state.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(std::time::Duration::from_secs(5));
            // When the last sweep ran. Cleared while sync is off so that turning
            // it on sweeps promptly instead of waiting out a stale interval.
            let mut last_sweep: Option<std::time::Instant> = None;
            loop {
                ticker.tick().await;
                let minutes = state
                    .db
                    .get_setting("ModelSyncIntervalMinutes")
                    .ok()
                    .flatten()
                    .and_then(|raw| raw.trim().parse::<u64>().ok())
                    .filter(|m| *m >= 1)
                    .unwrap_or(60);
                let enabled = state
                    .db
                    .get_setting("UpstreamModelSyncEnabled")
                    .ok()
                    .flatten()
                    .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "true" | "1"))
                    .unwrap_or(false);
                if !enabled {
                    last_sweep = None;
                    continue;
                }
                let due = last_sweep
                    .map(|last| last.elapsed() >= std::time::Duration::from_secs(minutes * 60))
                    .unwrap_or(true);
                if !due {
                    continue;
                }
                last_sweep = Some(std::time::Instant::now());
                let channels = state.db.get_enabled_channels().unwrap_or_default();
                // Sequential rather than concurrent: this is a background chore,
                // and a burst of simultaneous requests to every upstream at once
                // is exactly the load spike an upstream may throttle.
                for channel in channels {
                    match api::refresh_channel_models(&state, &channel).await {
                        Ok(models) => {
                            // Only announce a real change; an unchanged list every
                            // interval would bury the log line that matters.
                            if models != channel.model_list {
                                println!(
                                    "[OxygenRouter] model sync: {} now exposes {} models",
                                    channel.name,
                                    models.len()
                                );
                            }
                        }
                        // A failure is expected for a disabled or unreachable
                        // upstream and must not stop the sweep or the task.
                        Err(reason) => eprintln!(
                            "[OxygenRouter] model sync: {} skipped: {reason}",
                            channel.name
                        ),
                    }
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
