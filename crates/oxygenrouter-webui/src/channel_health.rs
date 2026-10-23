//! The scheduled channel health test.
//!
//! Mirrors the reference's `channel_test` system task
//! (`controller/system_task_handlers.go` + `controller/channel-test.go`): a
//! scheduled pass tests the configured channel set, disables the ones a failure
//! policy matches, and brings back the ones the engine itself took down.
//!
//! The pass is what makes auto-disable recoverable. Without it a channel the
//! policy stood down stays down until an operator notices, which turns a
//! transient upstream outage into a permanent one from the gateway's point of
//! view; the reference has the same dependency, which is why its monitor
//! settings live beside its retry rules.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Json, Response},
};
use serde::{Deserialize, Serialize};

use crate::state::AppState;
use oxygenrouter_core::{ApiResponse, Channel, SystemTask};
use oxygenrouter_proxy::{ChannelTestMode, RequestPolicy};

/// The task type, matching the reference's `SystemTaskTypeChannelTest`.
pub const CHANNEL_TEST: &str = "channel_test";

/// What one pass did, stored as the task's result.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ChannelTestSummary {
    pub tested: usize,
    pub succeeded: usize,
    pub failed: usize,
    pub disabled: usize,
    pub enabled: usize,
}

/// The payload a manual trigger stores, mirroring `channelTestTaskPayload`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChannelTestPayload {
    /// Empty means "use the configured mode", which is what a scheduled run
    /// does. A manual trigger sends `scheduled_all` so pressing the button tests
    /// every channel regardless of the schedule's own filter.
    #[serde(default)]
    pub mode: String,
}

/// The live progress shape, written to the task's `state`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ChannelTestState {
    pub processed: usize,
    pub total: usize,
    pub progress: usize,
}

/// Which channels a pass should look at, mirroring
/// `selectChannelsForAutomaticTest`.
///
/// A manually disabled channel is never tested: the operator's decision is not
/// something a health check gets to revisit. The three modes differ in what else
/// they filter out.
pub fn select_channels(channels: &[Channel], mode: ChannelTestMode) -> Vec<Channel> {
    channels
        .iter()
        .filter(|channel| !(channel.enabled == false && !channel.auto_disabled))
        .filter(|channel| match mode {
            // Every channel that is not manually disabled.
            ChannelTestMode::ScheduledAll => true,
            // Only the channels that opted in to auto-ban; a channel the
            // operator pinned must not be tested at all, because a test result
            // could only be used to disable it.
            ChannelTestMode::AutoBanOnly => channel.auto_ban,
            // Only the ones the engine disabled: this mode exists to recover
            // them without generating traffic against healthy channels.
            ChannelTestMode::PassiveRecovery => channel.auto_disabled,
        })
        .cloned()
        .collect()
}

/// Whether a probe failure should stand this channel down.
///
/// `passive_recovery` never disables (the reference's `allowDisable`): a mode
/// whose purpose is to bring channels back would otherwise re-disable them for
/// the same reasons it is testing.
fn should_disable(policy: &RequestPolicy, channel: &Channel, error: Option<&oxygenrouter_proxy::ProxyError>) -> bool {
    if !channel.auto_ban || !policy.auto_disable_enabled {
        return false;
    }
    match error {
        Some(error) => policy.should_disable(error),
        None => false,
    }
}

/// Run one pass over the configured channel set.
///
/// Sequential rather than concurrent even though the setting carries a
/// concurrency value: the reference's `channel_test_concurrency` sizes a worker
/// pool it needs because its pass writes progress through a channel, while this
/// pass is one background chore whose whole point is not to spike the upstreams
/// it is probing. The setting is still read and reported by the console so the
/// two instances' knobs stay comparable -- see the note in
/// `docs/research/PAGE_PARITY.md`.
pub async fn run_channel_test(
    state: Arc<AppState>,
    task_id: String,
    mode: Option<ChannelTestMode>,
) -> ChannelTestSummary {
    let policy = state.scheduler.read().await.policy().clone();
    let mode = mode.unwrap_or(policy.health.mode);

    let channels = state.db.list_channels().unwrap_or_default();
    let selected = select_channels(&channels, mode);
    let mut summary = ChannelTestSummary::default();

    let progress = ChannelTestState {
        processed: 0,
        total: selected.len(),
        progress: if selected.is_empty() { 100 } else { 0 },
    };
    if let Err(error) = state.db.update_system_task_state(
        &task_id,
        "running",
        &serde_json::to_string(&progress).unwrap_or_default(),
    ) {
        eprintln!("[OxygenRouter] channel test {task_id} could not start: {error}");
        return summary;
    }

    for (index, channel) in selected.iter().enumerate() {
        let probe = oxygenrouter_proxy::probe_channel(channel).await;
        if let Err(error) = state.db.record_channel_test(&channel.id) {
            eprintln!(
                "[OxygenRouter] could not stamp the test time on channel {}: {error}",
                channel.id
            );
        }
        summary.tested += 1;
        // The response-time threshold is part of the policy: the reference
        // treats "slow" as a failure, which is what makes it a health check
        // rather than a reachability check.
        let slow = policy.health.response_time_threshold_seconds > 0.0
            && (probe.latency_ms as f64) > policy.health.response_time_threshold_seconds * 1000.0
            && probe.succeeded();

        if probe.succeeded() && !slow {
            summary.succeeded += 1;
        } else {
            summary.failed += 1;
        }

        let error = if slow {
            Some(oxygenrouter_proxy::ProxyError::Upstream {
                status: 408,
                body: format!(
                    "response took {} ms, over the {} s health-test threshold",
                    probe.latency_ms, policy.health.response_time_threshold_seconds
                ),
            })
        } else {
            probe.error()
        };

        // Passive recovery only ever enables; it is the mode an operator picks
        // when they want the probe loop to be a recovery path and nothing else.
        if mode != ChannelTestMode::PassiveRecovery
            && channel.enabled
            && should_disable(&policy, channel, error.as_ref())
        {
            let reason = error
                .as_ref()
                .map(|error| policy.disable_reason(error))
                .unwrap_or_else(|| "failed its health test".to_string());
            match state.db.set_channel_auto_disabled(&channel.id, &reason) {
                Ok(true) => {
                    summary.disabled += 1;
                    println!(
                        "[OxygenRouter] channel {} ({}) disabled by health test: {reason}",
                        channel.name, channel.id
                    );
                }
                Ok(false) => {}
                Err(error) => eprintln!(
                    "[OxygenRouter] health test could not disable channel {}: {error}",
                    channel.id
                ),
            }
        }

        // The recovery side reads the *channel's* flag, not the mode: a channel
        // the engine disabled comes back as soon as it answers, in any mode that
        // tests it. `auto_disabled` is what keeps a manual disable untouched.
        if channel.auto_disabled && probe.succeeded() && policy.auto_enable_enabled {
            match state.db.enable_channel_from_auto(&channel.id) {
                Ok(true) => {
                    summary.enabled += 1;
                    state
                        .scheduler
                        .read()
                        .await
                        .note_attempt_success(&channel.id);
                    println!(
                        "[OxygenRouter] channel {} ({}) recovered by health test",
                        channel.name, channel.id
                    );
                }
                Ok(false) => {}
                Err(error) => eprintln!(
                    "[OxygenRouter] health test could not enable channel {}: {error}",
                    channel.id
                ),
            }
        }

        let progress = ChannelTestState {
            processed: index + 1,
            total: selected.len(),
            progress: (index + 1) * 100 / selected.len().max(1),
        };
        let _ = state.db.update_system_task_state(
            &task_id,
            "running",
            &serde_json::to_string(&progress).unwrap_or_default(),
        );
    }

    summary
}

/// Start a pass, refusing while another one is unfinished.
///
/// The refusal lives in the database's partial unique index, so it cannot race
/// with a concurrent trigger; this check only turns the index's error into a
/// readable message.
pub fn start_channel_test(
    state: &AppState,
    mode: Option<ChannelTestMode>,
) -> Result<SystemTask, String> {
    if let Ok(Some(active)) = state.db.active_system_task(CHANNEL_TEST) {
        return Err(format!(
            "a channel test is already {}: {}",
            active.status, active.task_id
        ));
    }
    let payload = serde_json::to_string(&ChannelTestPayload {
        mode: mode.map(|mode| mode.as_str().to_string()).unwrap_or_default(),
    })
    .unwrap_or_else(|_| "{}".to_string());
    state
        .db
        .create_system_task(CHANNEL_TEST, &payload)
        .map_err(|error| error.to_string())
}

/// Run a task row created by [`start_channel_test`], writing a terminal status
/// on every path and reporting what the pass did.
///
/// The terminal write lives here rather than at each call site so a scheduled
/// run and a manual trigger cannot differ in whether the row they created is
/// ever closed -- an unfinished row would block every later pass.
pub async fn run_channel_test_task(
    state: Arc<AppState>,
    task: SystemTask,
) -> ChannelTestSummary {
    let mode = serde_json::from_str::<ChannelTestPayload>(&task.payload)
        .ok()
        .map(|payload| payload.mode)
        .filter(|mode| !mode.trim().is_empty())
        .map(|mode| ChannelTestMode::from_setting(&mode));
    let summary = run_channel_test(Arc::clone(&state), task.task_id.clone(), mode).await;
    let result = serde_json::to_string(&summary).unwrap_or_default();
    if let Err(error) = state
        .db
        .finish_system_task(&task.task_id, "succeeded", &result, "")
    {
        eprintln!(
            "[OxygenRouter] channel test {} could not record its result: {error}",
            task.task_id
        );
    }
    summary
}

/// `POST /api/system-task/channel-test`
///
/// The manual trigger, matching the reference's `GET /api/channel/test`
/// (`controller/channel-test.go:1131`): it always tests every channel that is not
/// manually disabled, whatever mode the schedule is configured for, and one
/// active pass at a time is enforced by the same call the scheduler uses.
pub async fn create_channel_test(State(state): State<Arc<AppState>>) -> Response {
    let task = match start_channel_test(&state, Some(ChannelTestMode::ScheduledAll)) {
        Ok(task) => task,
        Err(message) => {
            // "Already running" is a conflict, not a malformed request: the
            // caller's request was well formed and the state refused it.
            let status = if message.contains("already") {
                StatusCode::CONFLICT
            } else {
                StatusCode::BAD_REQUEST
            };
            return (status, Json(ApiResponse::<SystemTask>::err(message))).into_response();
        }
    };
    let runner_state = Arc::clone(&state);
    let spawned = task.clone();
    tokio::spawn(async move {
        run_channel_test_task(runner_state, spawned).await;
    });
    Json(ApiResponse::ok(task)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxygenrouter_core::{ChannelSelector, Database};
    use oxygenrouter_proxy::{ChannelScheduler, RelayClient};

    fn channel(name: &str, enabled: bool, auto_ban: bool, auto_disabled: bool) -> Channel {
        let mut channel = Channel::new(
            name.to_string(),
            "http://127.0.0.1:9".to_string(),
            "k".to_string(),
            1,
            1,
            "m".to_string(),
        );
        channel.model_list = vec!["m".to_string()];
        channel.enabled = enabled;
        channel.auto_ban = auto_ban;
        channel.auto_disabled = auto_disabled;
        channel
    }

    fn state() -> Arc<AppState> {
        let dir = tempfile::tempdir().expect("temp dir");
        let db_path = dir.path().join("health-test.db");
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

    /// Each mode narrows the set differently, and a manual disable is never
    /// revisited by any of them.
    #[test]
    fn the_three_modes_select_different_channel_sets() {
        let channels = vec![
            channel("healthy", true, true, false),
            channel("pinned", true, false, false),
            channel("auto-down", false, true, true),
            channel("manual-down", false, true, false),
        ];

        let selected = select_channels(&channels, ChannelTestMode::ScheduledAll);
        let all: Vec<&str> = selected.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(all, vec!["healthy", "pinned", "auto-down"]);

        let selected = select_channels(&channels, ChannelTestMode::AutoBanOnly);
        let auto_ban: Vec<&str> = selected.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(auto_ban, vec!["healthy", "auto-down"]);

        let selected = select_channels(&channels, ChannelTestMode::PassiveRecovery);
        let recovery: Vec<&str> = selected.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(recovery, vec!["auto-down"]);
    }

    /// The disable gate needs the switch, the per-channel auto-ban, and a
    /// policy match; passive recovery never disables at all.
    #[test]
    fn disabling_requires_the_switch_and_the_per_channel_flag() {
        let mut policy = RequestPolicy::default();
        let error = oxygenrouter_proxy::ProxyError::Upstream {
            status: 401,
            body: "denied".to_string(),
        };
        let opted_in = channel("opted-in", true, true, false);

        assert!(
            !should_disable(&policy, &opted_in, Some(&error)),
            "the global switch is off by default"
        );
        policy.auto_disable_enabled = true;
        assert!(should_disable(&policy, &opted_in, Some(&error)));
        assert!(
            !should_disable(&policy, &channel("pinned", true, false, false), Some(&error)),
            "a pinned channel is never disabled"
        );
        assert!(
            !should_disable(&policy, &opted_in, None),
            "a successful probe has no failure to judge"
        );
        assert!(
            !should_disable(
                &policy,
                &opted_in,
                Some(&oxygenrouter_proxy::ProxyError::Upstream {
                    status: 200,
                    body: String::new(),
                })
            ),
            "200 is not in the shipped disable range"
        );
    }

    /// A pass against an unreachable upstream counts the failure and, with the
    /// switch on, stands the channel down through the database; turning the
    /// switch off leaves it alone.
    #[test]
    fn a_failing_probe_disables_only_when_the_policy_allows_it() {
        let state = state();
        let channel = channel("dead", true, true, false);
        state.db.upsert_channel(&channel).expect("channel");

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");

        // Off by default: the pass reports the failure and changes nothing.
        let task = start_channel_test(&state, Some(ChannelTestMode::ScheduledAll)).expect("task");
        let summary = runtime.block_on(run_channel_test_task(Arc::clone(&state), task));
        assert_eq!(summary.tested, 1);
        assert_eq!(summary.failed, 1);
        assert_eq!(summary.disabled, 0);
        assert!(
            state
                .db
                .get_channel(&channel.id)
                .expect("read")
                .expect("row")
                .enabled,
            "a failure alone must not disable anything"
        );

        // With the switch on, the same failure stands it down and the reason is
        // stored for the console.
        let mut policy = state.scheduler.blocking_read().policy().clone();
        policy.auto_disable_enabled = true;
        policy.disable_ranges = vec![(500, 599)];
        state.scheduler.blocking_write().set_policy(policy);

        let task = start_channel_test(&state, Some(ChannelTestMode::ScheduledAll)).expect("task");
        let summary = runtime.block_on(run_channel_test_task(Arc::clone(&state), task));
        assert_eq!(summary.disabled, 1);
        let stored = state.db.get_channel(&channel.id).expect("read").expect("row");
        assert!(!stored.enabled);
        assert!(stored.auto_disabled);
        assert!(!stored.status_reason.is_empty());
    }

    /// Passive recovery brings an engine-disabled channel back once it answers,
    /// but never touches one an operator disabled.
    #[test]
    fn passive_recovery_re_enables_only_engine_disables() {
        let state = state();
        let mut policy = state.scheduler.blocking_read().policy().clone();
        policy.auto_enable_enabled = true;
        policy.health.mode = ChannelTestMode::PassiveRecovery;
        state.scheduler.blocking_write().set_policy(policy);

        let manual = channel("manual", false, true, false);
        state.db.upsert_channel(&manual).expect("channel");

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let task = start_channel_test(&state, None).expect("task");
        let summary = runtime.block_on(run_channel_test_task(Arc::clone(&state), task));
        // The unreachable upstream was never tested (it is not auto-disabled),
        // so nothing was enabled and the manual disable survived.
        assert_eq!(summary.enabled, 0);
        assert!(
            !state
                .db
                .get_channel(&manual.id)
                .expect("read")
                .expect("row")
                .enabled
        );
    }

    /// A second pass is refused while the first is unfinished.
    #[test]
    fn one_test_runs_at_a_time() {
        let state = state();
        let first = start_channel_test(&state, None).expect("first");
        let error = start_channel_test(&state, None).expect_err("must refuse");
        assert!(error.contains("already"), "{error}");
        state
            .db
            .finish_system_task(&first.task_id, "succeeded", "{}", "")
            .expect("finish");
        assert!(start_channel_test(&state, None).is_ok());
    }
}
