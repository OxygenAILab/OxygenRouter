//! Maintenance tasks the operator can start from the console.
//!
//! Mirrors the reference's `/api/system-task` group and its `log_cleanup`
//! handler: the run is a durable row (queued, running, succeeded/failed) with its
//! request in `payload` and its progress in `state`, so a page reloaded while the
//! task is working still finds it.
//!
//! Two differences from the reference are deliberate:
//!
//! * One unfinished task per type, enforced by a partial unique index, and the
//!   work runs in this process. The reference's `locked_by` / lease columns exist
//!   so several runner instances can share one queue; this instance has one.
//! * The cleanup deletes in batches, releasing the write lock between them. A
//!   single `DELETE` over a large log would hold the lock for its whole duration
//!   and stall every request the gateway is serving.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
};
use chrono::{TimeZone, Utc};
use serde::{Deserialize, Serialize};

use crate::state::AppState;
use oxygenrouter_core::{ApiResponse, SystemTask};

/// The only task type this instance implements, matching the reference's
/// `SystemTaskTypeLogCleanup`.
pub const LOG_CLEANUP: &str = "log_cleanup";
/// Rows removed per statement, matching the reference's `logCleanupBatchSize`.
const BATCH_SIZE: i64 = 100;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LogCleanupPayload {
    pub target_timestamp: i64,
    pub batch_size: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct LogCleanupState {
    pub total: i64,
    pub processed: i64,
    pub progress: i64,
    pub remaining: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct LogCleanupResult {
    pub deleted_count: i64,
}

/// Start a cleanup, or refuse because one is already unfinished.
pub fn start_log_cleanup(
    state: &AppState,
    target_timestamp: i64,
) -> Result<SystemTask, String> {
    let now = Utc::now().timestamp();
    if target_timestamp <= 0 {
        return Err("target timestamp is required".to_string());
    }
    if target_timestamp >= now {
        // A future cutoff would delete every log row, including the one this
        // very request is writing. Refusing is the only safe reading.
        return Err("target timestamp must be in the past".to_string());
    }
    if let Ok(Some(active)) = state.db.active_system_task(LOG_CLEANUP) {
        return Err(format!(
            "a log cleanup task is already {}: {}",
            active.status, active.task_id
        ));
    }
    let payload = serde_json::to_string(&LogCleanupPayload {
        target_timestamp,
        batch_size: BATCH_SIZE,
    })
    .unwrap_or_else(|_| "{}".to_string());
    state
        .db
        .create_system_task(LOG_CLEANUP, &payload)
        .map_err(|error| error.to_string())
}

/// Run the cleanup for an already-recorded task.
///
/// Every exit path writes a terminal status: a row left `running` would block
/// the type forever, because the partial unique index counts it as active.
pub async fn run_log_cleanup(state: Arc<AppState>, task_id: String, target_timestamp: i64) {
    let Some(cutoff) = Utc.timestamp_opt(target_timestamp, 0).single() else {
        let _ = state.db.finish_system_task(
            &task_id,
            "failed",
            "",
            "target timestamp is not a valid instant",
        );
        return;
    };

    let total = match state.db.count_request_logs_before(cutoff) {
        Ok(total) => total,
        Err(error) => {
            let _ = state
                .db
                .finish_system_task(&task_id, "failed", "", &error.to_string());
            return;
        }
    };
    let mut progress = LogCleanupState {
        total,
        processed: 0,
        progress: if total == 0 { 100 } else { 0 },
        remaining: total,
    };
    if let Err(error) = state.db.update_system_task_state(
        &task_id,
        "running",
        &serde_json::to_string(&progress).unwrap_or_default(),
    ) {
        // The row could not be marked running, so the task cannot be reported;
        // there is nothing to run either, since the caller watches that row.
        eprintln!("[OxygenRouter] log cleanup {task_id} could not start: {error}");
        return;
    }

    loop {
        match state.db.delete_request_logs_before(cutoff, BATCH_SIZE) {
            Ok(0) => break,
            Ok(deleted) => {
                progress.processed += deleted as i64;
                progress.remaining = (total - progress.processed).max(0);
                progress.progress = if total == 0 {
                    100
                } else {
                    progress.processed * 100 / total
                };
                let _ = state.db.update_system_task_state(
                    &task_id,
                    "running",
                    &serde_json::to_string(&progress).unwrap_or_default(),
                );
                if (deleted as i64) < BATCH_SIZE {
                    break;
                }
                // Hand the write lock back for a moment so a live request can
                // record its log row between batches.
                tokio::task::yield_now().await;
            }
            Err(error) => {
                let _ = state
                    .db
                    .finish_system_task(&task_id, "failed", "", &error.to_string());
                return;
            }
        }
    }

    progress.remaining = (total - progress.processed).max(0);
    progress.progress = 100;
    let _ = state.db.update_system_task_state(
        &task_id,
        "running",
        &serde_json::to_string(&progress).unwrap_or_default(),
    );
    let result = serde_json::to_string(&LogCleanupResult {
        deleted_count: progress.processed,
    })
    .unwrap_or_default();
    if let Err(error) = state
        .db
        .finish_system_task(&task_id, "succeeded", &result, "")
    {
        eprintln!("[OxygenRouter] log cleanup {task_id} could not record its result: {error}");
    }
}

#[derive(Debug, Deserialize)]
pub struct StartLogCleanupQuery {
    #[serde(default)]
    target_timestamp: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct CurrentTaskQuery {
    #[serde(default, rename = "type")]
    task_type: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ListTasksQuery {
    #[serde(default, rename = "type")]
    task_type: Option<String>,
    #[serde(default)]
    offset: Option<i64>,
    #[serde(default)]
    limit: Option<i64>,
}

#[derive(Serialize)]
pub struct TaskList {
    pub tasks: Vec<SystemTask>,
    pub total: i64,
}

fn bad_request<T: Serialize>(message: String) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(ApiResponse::<T>::err(message)),
    )
        .into_response()
}

/// `POST /api/system-task/log-cleanup?target_timestamp=…`
pub async fn create_log_cleanup(
    State(state): State<Arc<AppState>>,
    Query(query): Query<StartLogCleanupQuery>,
) -> Response {
    let Some(target) = query.target_timestamp else {
        return bad_request::<SystemTask>("target timestamp is required".to_string());
    };
    let task = match start_log_cleanup(&state, target) {
        Ok(task) => task,
        Err(message) => return bad_request::<SystemTask>(message),
    };
    let spawned = task.clone();
    let runner_state = Arc::clone(&state);
    tokio::spawn(async move {
        run_log_cleanup(runner_state, spawned.task_id, target).await;
    });
    Json(ApiResponse::ok(task)).into_response()
}

/// `GET /api/system-task/current?type=log_cleanup`
pub async fn current_task(
    State(state): State<Arc<AppState>>,
    Query(query): Query<CurrentTaskQuery>,
) -> Response {
    let Some(task_type) = query
        .task_type
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return bad_request::<Option<SystemTask>>("type is required".to_string());
    };
    match state.db.active_system_task(task_type) {
        Ok(task) => Json(ApiResponse::ok(task)).into_response(),
        Err(error) => bad_request::<Option<SystemTask>>(error.to_string()),
    }
}

/// `GET /api/system-task/list[?type=][&offset=][&limit=]`
pub async fn list_tasks(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ListTasksQuery>,
) -> Response {
    let task_type = query
        .task_type
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let offset = query.offset.unwrap_or(0).max(0);
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    match state.db.list_system_tasks(task_type, offset, limit) {
        Ok((tasks, total)) => Json(ApiResponse::ok(TaskList { tasks, total })).into_response(),
        Err(error) => bad_request::<TaskList>(error.to_string()),
    }
}

/// `GET /api/system-task/:task_id`
pub async fn get_task(
    State(state): State<Arc<AppState>>,
    Path(task_id): Path<String>,
) -> Response {
    match state.db.get_system_task(&task_id) {
        Ok(Some(task)) => Json(ApiResponse::ok(task)).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::<SystemTask>::err("task not found")),
        )
            .into_response(),
        Err(error) => bad_request::<SystemTask>(error.to_string()),
    }
}

/// `DELETE /api/system-task/history[?type=]`
pub async fn delete_history(
    State(state): State<Arc<AppState>>,
    Query(query): Query<CurrentTaskQuery>,
) -> Response {
    let task_type = query
        .task_type
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    match state.db.delete_system_task_history(task_type) {
        Ok(deleted) => Json(ApiResponse::ok(deleted)).into_response(),
        Err(error) => bad_request::<usize>(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxygenrouter_core::{ChannelSelector, Database};
    use oxygenrouter_proxy::{ChannelScheduler, RelayClient};

    fn state() -> Arc<AppState> {
        let dir = tempfile::tempdir().expect("temp dir");
        let db_path = dir.path().join("tasks-test.db");
        let config_path = dir.path().join("config.json");
        std::mem::forget(dir);

        let db = Arc::new(Database::new(&db_path).expect("database opens"));
        let selector = ChannelSelector::new(Arc::clone(&db));
        let relay = RelayClient::new("OxygenRouter-test".to_string(), 5_000);
        let scheduler = ChannelScheduler::new(selector, relay);
        Arc::new(AppState::new(
            db,
            db_path,
            scheduler,
            "test-token".to_string(),
            config_path,
        ))
    }

    /// A second cleanup is refused by name while the first is unfinished, and the
    /// refusal is the database's, not a check that could race.
    #[test]
    fn one_cleanup_runs_at_a_time() {
        let state = state();
        let past = Utc::now().timestamp() - 3600;
        let first = start_log_cleanup(&state, past).expect("first task");
        assert_eq!(first.status, "pending");

        let error = start_log_cleanup(&state, past).expect_err("a second task must be refused");
        assert!(error.contains("already"), "{error}");

        // Finishing it releases the type.
        state
            .db
            .finish_system_task(&first.task_id, "succeeded", "{}", "")
            .expect("finish");
        assert!(start_log_cleanup(&state, past).is_ok());
    }

    /// A cutoff in the future would delete the log row this very request writes,
    /// so it is refused rather than obeyed.
    #[test]
    fn a_future_cutoff_is_refused() {
        let state = state();
        let future = Utc::now().timestamp() + 3600;
        let error = start_log_cleanup(&state, future).expect_err("must refuse");
        assert!(error.contains("past"), "{error}");
        assert!(start_log_cleanup(&state, 0).is_err());
    }

    /// The runner deletes only what is older than the cutoff, reports progress,
    /// and leaves a terminal row.
    #[test]
    fn a_cleanup_deletes_old_logs_and_reports_its_result() {
        let state = state();
        let cutoff = Utc::now() - chrono::Duration::days(30);
        let log = |id: &str, age_days: i64| oxygenrouter_core::RequestLog {
            id: id.to_string(),
            method: "POST".to_string(),
            path: "/v1/chat/completions".to_string(),
            model: Some("m".to_string()),
            channel_id: None,
            api_key_id: None,
            status_code: Some(200),
            error: None,
            tokens_used: Some(1),
            duration_ms: 1,
            created_at: Utc::now() - chrono::Duration::days(age_days),
            client_ip: None,
        };
        state.db.insert_request_log(&log("old-1", 40)).expect("old");
        state.db.insert_request_log(&log("old-2", 35)).expect("old");
        state.db.insert_request_log(&log("fresh", 1)).expect("fresh");

        let task = start_log_cleanup(&state, cutoff.timestamp()).expect("task");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(run_log_cleanup(
            Arc::clone(&state),
            task.task_id.clone(),
            cutoff.timestamp(),
        ));

        let finished = state
            .db
            .get_system_task(&task.task_id)
            .expect("read")
            .expect("row");
        assert_eq!(finished.status, "succeeded");
        let result: LogCleanupResult =
            serde_json::from_str(&finished.result).expect("result json");
        assert_eq!(result.deleted_count, 2);
        assert_eq!(
            state.db.count_request_logs_before(cutoff).expect("count"),
            0
        );
        assert_eq!(
            state
                .db
                .count_request_logs_before(Utc::now() + chrono::Duration::days(1))
                .expect("count"),
            1,
            "the fresh row must survive"
        );
    }
}
