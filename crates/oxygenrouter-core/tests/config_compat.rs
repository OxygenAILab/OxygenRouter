//! `config.json` backward/forward compatibility.
//!
//! Regression guard for a real defect: `AppSettings` had no per-field serde
//! defaults, so a hand-written `config.json` that omitted any field failed to
//! deserialize. `main` then saved the in-memory defaults over the operator's
//! file, silently discarding settings such as a custom listen port.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use oxygenrouter_core::AppSettings;

#[test]
fn partial_config_json_parses_and_keeps_supplied_fields() {
    // The exact shape an operator writes by hand: a port and a token, nothing else.
    let raw = r#"{
        "listen_host": "0.0.0.0",
        "listen_port": 3099,
        "open_browser_on_start": false,
        "local_api_token": "e2e-token",
        "log_level": "debug"
    }"#;

    let cfg: AppSettings =
        serde_json::from_str(raw).expect("partial config must deserialize, not error");

    assert_eq!(cfg.listen_host, "0.0.0.0");
    assert_eq!(cfg.listen_port, 3099);
    assert!(!cfg.open_browser_on_start);
    assert_eq!(cfg.local_api_token, "e2e-token");
    assert_eq!(cfg.log_level, "debug");

    // Omitted fields fall back to the documented defaults.
    assert_eq!(cfg.max_retries, 3);
    assert_eq!(cfg.upstream_timeout_ms, 120_000);
    assert_eq!(cfg.max_concurrent_requests, 64);
    assert_eq!(cfg.request_log_retention_days, 30);
    assert_eq!(cfg.retry_backoff, "exponential");
    assert_eq!(cfg.theme, "dark");
    assert_eq!(cfg.language, "auto");
}

#[test]
fn empty_object_yields_all_defaults() {
    let cfg: AppSettings = serde_json::from_str("{}").expect("empty object must parse");
    let default = AppSettings::default();

    assert_eq!(cfg.listen_host, default.listen_host);
    assert_eq!(cfg.listen_port, default.listen_port);
    assert_eq!(cfg.max_retries, default.max_retries);
    assert_eq!(cfg.db_path, default.db_path);
}

#[test]
fn a_generated_token_is_never_empty() {
    // `local_api_token` has a random default; an omitted field must still yield a
    // usable token rather than an empty string that would lock clients out.
    let cfg: AppSettings = serde_json::from_str("{}").unwrap();
    assert!(!cfg.local_api_token.is_empty());
}

#[test]
fn round_trip_preserves_every_field() {
    let mut original = AppSettings::default();
    original.listen_port = 4123;
    original.user_agent = "custom-agent/9".to_string();
    original.max_retries = 7;
    original.retry_backoff = "none".to_string();

    let json = serde_json::to_string(&original).unwrap();
    let parsed: AppSettings = serde_json::from_str(&json).unwrap();

    assert_eq!(parsed.listen_port, 4123);
    assert_eq!(parsed.user_agent, "custom-agent/9");
    assert_eq!(parsed.max_retries, 7);
    assert_eq!(parsed.retry_backoff, "none");
}

#[test]
fn malformed_json_is_an_error_not_a_silent_default() {
    // The caller must be able to distinguish "no file" from "broken file".
    assert!(serde_json::from_str::<AppSettings>("{ not json").is_err());
}
