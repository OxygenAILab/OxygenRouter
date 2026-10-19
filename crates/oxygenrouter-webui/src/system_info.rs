//! System instance heartbeats and stale-instance cleanup.
//!
//! Mirrors the reference's `/api/system-info/*` RootAuth group
//! (`router/api-router.go:331-337`): every process reports a heartbeat, the
//! console lists online and stale nodes, and only stale rows may be removed.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
};
use chrono::Utc;
use serde::Serialize;

use crate::state::AppState;
use oxygenrouter_core::ApiResponse;

/// The reference's `SystemInstanceStaleAfterSeconds`.
pub const STALE_AFTER_SECONDS: i64 = 90;
/// The reference's `systemInstanceReportInterval`.
pub const REPORT_INTERVAL_SECONDS: u64 = 30;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SystemInstanceResponse {
    pub node_name: String,
    pub status: String,
    pub stale_after_seconds: i64,
    pub started_at: i64,
    pub last_seen_at: i64,
    pub info: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
struct NodeInfo {
    name: String,
    source: String,
    manually_configured: bool,
    should_configure_manually: bool,
}

#[derive(Debug, Clone, Serialize)]
struct RoleInfo {
    is_master: bool,
}

#[derive(Debug, Clone, Serialize)]
struct RuntimeInfo {
    version: String,
    goos: String,
    goarch: String,
    started_at: i64,
}

#[derive(Debug, Clone, Serialize)]
struct HostInfo {
    hostname: String,
}

#[derive(Debug, Clone, Default, Serialize)]
struct ResourceUsage {
    usage_percent: f64,
}

#[derive(Debug, Clone, Default, Serialize)]
struct StorageMetrics {
    total_bytes: u64,
    used_bytes: u64,
    free_bytes: u64,
    used_percent: f64,
}

#[derive(Debug, Clone, Default, Serialize)]
struct ResourcesInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    cpu: Option<ResourceUsage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    memory: Option<ResourceUsage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    storage: Option<StorageMetrics>,
}

#[derive(Debug, Clone, Serialize)]
struct InstanceInfo {
    schema_version: u32,
    node: NodeInfo,
    role: RoleInfo,
    runtime: RuntimeInfo,
    host: HostInfo,
    resources: ResourcesInfo,
}

fn nonempty(value: String) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn hostname() -> String {
    std::env::var("COMPUTERNAME")
        .ok()
        .and_then(nonempty)
        .or_else(|| std::env::var("HOSTNAME").ok().and_then(nonempty))
        .unwrap_or_else(|| "oxygenrouter".to_string())
}

/// Resolve the stable node identity exactly as the reference describes it.
///
/// `NODE_NAME` is the operator's stable, unique identity. Without it, the host
/// name is used and the console warns the operator, because automatic host
/// names are not a safe multi-instance key in container deployments.
fn node_identity() -> (String, String, bool, bool) {
    match std::env::var("NODE_NAME").ok().and_then(nonempty) {
        Some(name) => (name, "env".to_string(), true, false),
        None => (hostname(), "hostname".to_string(), false, true),
    }
}

/// Mark a heartbeat stale after the same 90-second grace period as the
/// reference. A clock jump backwards is treated as online rather than making a
/// live node disappear.
pub fn response_for(
    instance: &oxygenrouter_core::SystemInstance,
    now: i64,
) -> SystemInstanceResponse {
    let status = if now.saturating_sub(instance.last_seen_at) > STALE_AFTER_SECONDS {
        "stale"
    } else {
        "online"
    };
    SystemInstanceResponse {
        node_name: instance.node_name.clone(),
        status: status.to_string(),
        stale_after_seconds: STALE_AFTER_SECONDS,
        started_at: instance.started_at,
        last_seen_at: instance.last_seen_at,
        info: instance.info.clone(),
    }
}

/// Report this process once. The reporter calls this on a fixed cadence.
pub fn report_current(state: &AppState) -> Result<(), String> {
    let (node_name, source, manually_configured, should_configure_manually) = node_identity();
    let hostname = hostname();
    let now = Utc::now().timestamp();
    let info = serde_json::to_value(InstanceInfo {
        schema_version: 1,
        node: NodeInfo {
            name: node_name.clone(),
            source,
            manually_configured,
            should_configure_manually,
        },
        role: RoleInfo { is_master: true },
        runtime: RuntimeInfo {
            version: env!("CARGO_PKG_VERSION").to_string(),
            goos: std::env::consts::OS.to_string(),
            goarch: std::env::consts::ARCH.to_string(),
            started_at: state.started_at.timestamp(),
        },
        host: HostInfo { hostname },
        resources: collect_resources(&state.db_path),
    })
    .map_err(|error| error.to_string())?;
    state
        .db
        .upsert_system_instance(&node_name, &info, state.started_at.timestamp(), now)
        .map_err(|error| error.to_string())
}

/// Start the 30-second heartbeat task once per process.
pub fn start_reporter(state: Arc<AppState>) {
    tokio::spawn(async move {
        loop {
            if let Err(error) = report_current(&state) {
                eprintln!("[OxygenRouter] system instance heartbeat failed: {error}");
            }
            tokio::time::sleep(Duration::from_secs(REPORT_INTERVAL_SECONDS)).await;
        }
    });
}

/// `GET /api/system-info/instances`
pub async fn list_instances(
    State(state): State<Arc<AppState>>,
) -> Json<ApiResponse<Vec<SystemInstanceResponse>>> {
    match state.db.list_system_instances() {
        Ok(instances) => {
            let now = Utc::now().timestamp();
            Json(ApiResponse::ok(
                instances
                    .iter()
                    .map(|instance| response_for(instance, now))
                    .collect(),
            ))
        }
        Err(error) => Json(ApiResponse::err(error.to_string())),
    }
}

#[derive(Debug, Serialize)]
struct DeleteResult {
    deleted_count: usize,
}

fn bad_request(message: String) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(ApiResponse::<DeleteResult>::err(message)),
    )
        .into_response()
}

/// `DELETE /api/system-info/stale-instances`
pub async fn delete_stale_instances(State(state): State<Arc<AppState>>) -> Response {
    let cutoff = Utc::now().timestamp() - STALE_AFTER_SECONDS;
    match state.db.delete_stale_system_instances(cutoff) {
        Ok(deleted_count) => Json(ApiResponse::ok(DeleteResult { deleted_count })).into_response(),
        Err(error) => bad_request(error.to_string()),
    }
}

/// `DELETE /api/system-info/instances/:node_name`
pub async fn delete_stale_instance(
    State(state): State<Arc<AppState>>,
    Path(node_name): Path<String>,
) -> Response {
    let node_name = node_name.trim();
    if node_name.is_empty() {
        return bad_request("node name is required".to_string());
    }
    let cutoff = Utc::now().timestamp() - STALE_AFTER_SECONDS;
    match state.db.delete_stale_system_instance(node_name, cutoff) {
        Ok(0) => bad_request("instance is not stale or no longer exists".to_string()),
        Ok(deleted_count) => Json(ApiResponse::ok(DeleteResult { deleted_count })).into_response(),
        Err(error) => bad_request(error.to_string()),
    }
}

fn collect_resources(db_path: &std::path::Path) -> ResourcesInfo {
    #[cfg(windows)]
    {
        windows_resources(db_path)
    }
    #[cfg(not(windows))]
    {
        let _ = db_path;
        ResourcesInfo::default()
    }
}

#[cfg(windows)]
fn windows_resources(db_path: &std::path::Path) -> ResourcesInfo {
    use std::os::windows::ffi::OsStrExt;
    use std::sync::Mutex;
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    use windows_sys::Win32::System::Threading::GetSystemTimes;

    static CPU_SAMPLE: once_cell::sync::Lazy<Mutex<Option<(u64, u64, u64)>>> =
        once_cell::sync::Lazy::new(|| Mutex::new(None));

    fn ticks(time: FILETIME) -> u64 {
        ((time.dwHighDateTime as u64) << 32) | time.dwLowDateTime as u64
    }

    let mut resources = ResourcesInfo::default();

    let mut idle = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let mut kernel = idle;
    let mut user = idle;
    if unsafe { GetSystemTimes(&mut idle, &mut kernel, &mut user) } != 0 {
        let current = (ticks(idle), ticks(kernel), ticks(user));
        if let Ok(mut previous) = CPU_SAMPLE.lock() {
            if let Some(old) = previous.replace(current) {
                let idle_delta = current.0.saturating_sub(old.0);
                let total_delta = current
                    .1
                    .saturating_sub(old.1)
                    .saturating_add(current.2.saturating_sub(old.2));
                if total_delta > 0 {
                    let busy_delta = total_delta.saturating_sub(idle_delta);
                    let percent = (busy_delta as f64 / total_delta as f64) * 100.0;
                    resources.cpu = Some(ResourceUsage {
                        usage_percent: percent.clamp(0.0, 100.0),
                    });
                }
            }
        }
    }

    let mut memory = MEMORYSTATUSEX {
        dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
        dwMemoryLoad: 0,
        ullTotalPhys: 0,
        ullAvailPhys: 0,
        ullTotalPageFile: 0,
        ullAvailPageFile: 0,
        ullTotalVirtual: 0,
        ullAvailVirtual: 0,
        ullAvailExtendedVirtual: 0,
    };
    if unsafe { GlobalMemoryStatusEx(&mut memory) } != 0 {
        resources.memory = Some(ResourceUsage {
            usage_percent: memory.dwMemoryLoad as f64,
        });
    }

    let directory = db_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::Path::new("."));
    let wide: Vec<u16> = directory
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut free_to_caller = 0u64;
    let mut total = 0u64;
    let mut free = 0u64;
    if unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut free_to_caller, &mut total, &mut free) }
        != 0
        && total > 0
    {
        let used = total.saturating_sub(free);
        resources.storage = Some(StorageMetrics {
            total_bytes: total,
            used_bytes: used,
            free_bytes: free,
            used_percent: (used as f64 / total as f64) * 100.0,
        });
    }

    resources
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use serde_json::json;

    fn instance(last_seen_at: i64) -> oxygenrouter_core::SystemInstance {
        oxygenrouter_core::SystemInstance {
            node_name: "node-a".to_string(),
            info: json!({"runtime": {"version": "test"}}),
            started_at: 1,
            last_seen_at,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn the_status_flips_only_after_the_reference_grace_period() {
        assert_eq!(response_for(&instance(100), 190).status, "online");
        assert_eq!(response_for(&instance(100), 191).status, "stale");
        assert_eq!(response_for(&instance(100), 90).status, "online");
    }

    #[test]
    fn the_response_keeps_the_reported_environment() {
        let response = response_for(&instance(100), 110);
        assert_eq!(response.node_name, "node-a");
        assert_eq!(response.info["runtime"]["version"], "test");
        assert_eq!(response.stale_after_seconds, 90);
    }
}
