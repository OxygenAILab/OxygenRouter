//! REST API for CRUD operations
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Json, Response},
    routing::{delete, get, patch, post, put},
    Router,
};
use chrono::Utc;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::broadcast;
use tokio::time::{interval, Duration};

use crate::state::AppState;
use oxygenrouter_core::{
    ApiKey, ApiKeyUsage, ApiResponse, Channel, DashboardBreakdown, ModelMap, PaymentOrder,
    RedemptionCode, RequestLog, RouteRule, RouteType, SubscriptionPlan, User, UserRole,
};

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/channels", get(list_channels))
        .route("/api/channels", post(create_channel))
        .route("/api/channels/:id", get(get_channel))
        .route("/api/channels/:id", put(update_channel))
        .route("/api/channels/:id", delete(delete_channel))
        .route("/api/channels/:id/test", post(test_channel))
        .route("/api/channels/:id/models/fetch", post(fetch_channel_models))
        .route("/api/channels/:id/keys", get(list_channel_keys))
        .route("/api/channels/:id/keys", post(manage_channel_keys))
        .route("/api/models-metadata", get(list_model_metadata))
        .route("/api/models-metadata", post(create_model_metadata))
        .route("/api/models-metadata/:id", put(update_model_metadata))
        .route("/api/models-metadata/:id", delete(delete_model_metadata))
        .route("/api/models-metadata/missing", get(missing_model_metadata))
        .route("/api/models-metadata/sync", post(sync_model_metadata))
        // Vendors, matching the reference's `/api/vendors` group
        // (router/api-router.go:376-386).
        .route("/api/vendors", get(list_vendors))
        .route("/api/vendors", post(create_vendor))
        .route("/api/vendors/search", get(search_vendors))
        .route("/api/vendors/:id", get(get_vendor))
        .route("/api/vendors/:id", put(update_vendor))
        .route("/api/vendors/:id", delete(delete_vendor))
        .route("/api/channels/batch", patch(batch_update_channels))
        .route("/api/channels/batch", delete(batch_delete_channels))
        .route("/api/log/stats", get(log_stats))
        .route("/api/keys", get(list_keys))
        .route("/api/keys", post(create_key))
        .route("/api/keys/:id", delete(delete_key))
        .route("/api/keys/usage", get(keys_usage))
        .route("/api/keys/query", get(query_keys))
        .route("/api/model-maps", get(list_model_maps))
        .route("/api/model-maps", post(create_model_map))
        .route("/api/model-maps/:id", delete(delete_model_map))
        .route("/api/rules", get(list_rules))
        .route("/api/rules", post(create_rule))
        .route("/api/rules/:id", delete(delete_rule))
        .route("/api/logs", get(list_logs).delete(clear_logs))
        .route("/api/logs/stream", get(stream_logs))
        .route("/api/status", get(get_status))
        .route("/api/dashboard", get(get_dashboard))
        .route("/api/analytics/flow", get(get_analytics_flow))
        .route("/api/settings", get(get_settings))
        .route("/api/settings", put(update_settings))
        .route("/api/options", get(get_options))
        .route("/api/options", put(update_option))
        .route("/api/system/info", get(system_info))
        .route("/api/backup/create", post(create_backup))
        .route("/api/auth/register", post(register))
        .route("/api/auth/login", post(login))
        .route("/api/auth/logout", post(logout))
        .route("/api/auth/me", get(me))
        // Session management, matching the reference's `/api/user/sessions`
        // (router/api-router.go:94-96).
        .route("/api/user/sessions", get(list_sessions))
        .route("/api/user/sessions/:sid", delete(delete_session))
        .route(
            "/api/user/sessions/revoke-others",
            post(revoke_other_sessions),
        )
        // Access tokens, matching the reference's `/api/user/token`
        // (router/api-router.go:102-105). The reference serves GET and POST on
        // the same path; both generate.
        .route("/api/user/token", get(generate_access_token))
        .route("/api/user/token", post(generate_access_token))
        .route("/api/user/token", delete(revoke_access_token))
        .route("/api/user/token/status", get(access_token_status))
        // Two-factor, matching the reference's `/api/user/2fa/*`
        // (router/api-router.go:129-133).
        .route("/api/user/2fa/status", get(two_fa_status))
        .route("/api/user/2fa/setup", post(two_fa_setup))
        .route("/api/user/2fa/enable", post(two_fa_enable))
        .route("/api/user/2fa/disable", post(two_fa_disable))
        .route("/api/user/2fa/backup_codes", post(two_fa_backup_codes))
        .route("/api/wallet", get(wallet))
        .route("/api/subscriptions/me", get(my_subscriptions))
        .route("/api/subscriptions/subscribe", post(subscribe))
        .route("/api/plans", get(public_plans))
        .route("/api/redemption/redeem", post(redeem_code))
        .route("/api/orders/manual", post(create_manual_order))
        .route("/api/admin/users", get(admin_users).post(admin_create_user))
        .route(
            "/api/admin/users/:id",
            put(admin_update_user).delete(admin_delete_user),
        )
        .route(
            "/api/admin/users/:id/balance-adjust",
            post(admin_adjust_balance),
        )
        .route("/api/admin/plans", get(admin_plans).post(admin_create_plan))
        // Subscription lifecycle, matching the reference's `/api/subscription/admin`
        // group (router/api-router.go — `subscriptionAdminRoute`).
        .route(
            "/api/subscription/admin/bind",
            post(admin_bind_subscription),
        )
        .route(
            "/api/subscription/admin/users/:id/subscriptions",
            get(admin_user_subscriptions),
        )
        .route(
            "/api/subscription/admin/users/:id/subscriptions/reset",
            post(admin_reset_user_subscriptions),
        )
        .route(
            "/api/subscription/admin/user_subscriptions/:id/invalidate",
            post(admin_invalidate_subscription),
        )
        .route(
            "/api/subscription/admin/user_subscriptions/:id",
            delete(admin_delete_subscription),
        )
        .route(
            "/api/subscription/admin/plans/:id/subscriptions",
            get(admin_plan_subscriptions),
        )
        .route(
            "/api/subscription/admin/plans/:id/subscriptions/reset",
            post(admin_reset_plan_subscriptions),
        )
        .route(
            "/api/admin/plans/:id",
            put(admin_update_plan).delete(admin_delete_plan),
        )
        .route(
            "/api/admin/redemption-codes",
            get(admin_codes).post(admin_create_code),
        )
        .route(
            "/api/admin/redemption-codes/:id",
            put(admin_update_code).delete(admin_delete_code),
        )
        .route("/api/admin/orders", get(admin_orders))
        .route("/api/admin/orders/:id/complete", post(admin_complete_order))
        .route("/api/admin/ledger", get(admin_ledger))
        // The permission schema, matching the reference's `/api/authz/catalog`
        // (router/authz-router.go:14-17).
        .route("/api/authz/catalog", get(authz_catalog))
        .with_state(state)
}

fn session_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .or_else(|| {
            headers
                .get("x-session-token")
                .and_then(|value| value.to_str().ok())
        })
}

fn auth_user(s: &AppState, headers: &HeaderMap) -> Result<User, Response> {
    let Some(token) = session_token(headers) else {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(ApiResponse::<()>::err("authentication required")),
        )
            .into_response());
    };
    match s.db.session_user(token) {
        Ok(Some((user, _))) => Ok(user),
        // Not a live session: it may be a long-lived access token, which is what
        // lets a script drive the user API without holding a login session.
        Ok(None) => match s.db.user_by_access_token(token) {
            Ok(Some(user)) => Ok(user),
            Ok(None) => Err((
                StatusCode::UNAUTHORIZED,
                Json(ApiResponse::<()>::err("invalid or expired session")),
            )
                .into_response()),
            Err(_) => Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse::<()>::err("authentication lookup failed")),
            )
                .into_response()),
        },
        Err(_) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse::<()>::err("authentication lookup failed")),
        )
            .into_response()),
    }
}
fn admin_user(s: &AppState, headers: &HeaderMap) -> Result<User, Response> {
    let user = auth_user(s, headers)?;
    if user.role == UserRole::Admin {
        Ok(user)
    } else {
        Err((
            StatusCode::FORBIDDEN,
            Json(ApiResponse::<()>::err("admin role required")),
        )
            .into_response())
    }
}
fn db_error<T: Serialize>(error: rusqlite::Error) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(ApiResponse::<T>::err(error.to_string())),
    )
        .into_response()
}

#[derive(Deserialize)]
struct RegisterInput {
    username: String,
    email: String,
    password: String,
}
#[derive(Deserialize)]
struct LoginInput {
    username: String,
    password: String,
    /// The second factor, when the account has 2FA enabled. Optional so the
    /// first attempt can return "code required" rather than a bare rejection.
    #[serde(default)]
    code: Option<String>,
}
#[derive(Serialize)]
struct LoginResponse {
    user: User,
    session_token: String,
    expires_at: chrono::DateTime<Utc>,
}
async fn register(State(s): State<Arc<AppState>>, Json(input): Json<RegisterInput>) -> Response {
    match s.db.create_user(
        &input.username,
        &input.email,
        &input.password,
        UserRole::User,
    ) {
        Ok(user) => (StatusCode::CREATED, Json(ApiResponse::ok(user))).into_response(),
        Err(e) => db_error::<User>(e),
    }
}
async fn login(State(s): State<Arc<AppState>>, Json(input): Json<LoginInput>) -> Response {
    match s.db.authenticate(&input.username, &input.password) {
        Ok(Some(user)) => {
            // Two-factor gate. When a user has 2FA on, a correct password is only
            // the first factor: no session is issued until a valid code is
            // supplied. A password alone must not produce a usable credential, or
            // the second factor would be decorative.
            if let Ok(Some(_)) = s.db.enabled_two_fa_secret(&user.id) {
                let Some(code) = input.code.as_deref().filter(|c| !c.trim().is_empty()) else {
                    return (
                        StatusCode::UNAUTHORIZED,
                        Json(ApiResponse::<TwoFactorRequired>::err(
                            "two-factor code required",
                        )),
                    )
                        .into_response();
                };
                let accepted = verify_two_factor(&s, &user.id, code);
                if !accepted {
                    return (
                        StatusCode::UNAUTHORIZED,
                        Json(ApiResponse::<LoginResponse>::err("invalid verification code")),
                    )
                        .into_response();
                }
            }
            match s.db.create_session(&user.id, chrono::Duration::days(7)) {
                Ok(session) => Json(ApiResponse::ok(LoginResponse {
                    user,
                    session_token: session.token,
                    expires_at: session.expires_at,
                }))
                .into_response(),
                Err(e) => db_error::<LoginResponse>(e),
            }
        }
        Ok(None) => (
            StatusCode::UNAUTHORIZED,
            Json(ApiResponse::<LoginResponse>::err(
                "invalid username or password",
            )),
        )
            .into_response(),
        Err(e) => db_error::<LoginResponse>(e),
    }
}
async fn logout(State(s): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let Some(token) = session_token(&headers) else {
        return (
            StatusCode::UNAUTHORIZED,
            Json(ApiResponse::<()>::err("authentication required")),
        )
            .into_response();
    };
    match s.db.revoke_session(token) {
        Ok(_) => Json(ApiResponse::ok("logged out")).into_response(),
        Err(e) => db_error::<&str>(e),
    }
}
async fn me(State(s): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    match auth_user(&s, &headers) {
        Ok(user) => Json(ApiResponse::ok(user)).into_response(),
        Err(response) => response,
    }
}

/// Marker type for the "send a code" response, so the client can tell the
/// difference between a rejected password and a pending second factor.
#[derive(serde::Serialize)]
struct TwoFactorRequired {
    two_factor_required: bool,
}

/// Accept a TOTP code or a single-use recovery code.
///
/// The recovery path consumes the code, so a leaked code cannot be replayed.
fn verify_two_factor(state: &AppState, user_id: &str, code: &str) -> bool {
    let totp_ok = state
        .db
        .enabled_two_fa_secret(user_id)
        .ok()
        .flatten()
        .and_then(|secret| oxygenrouter_core::totp::base32_decode(&secret))
        .map(|decoded| oxygenrouter_core::totp::verify(&decoded, code))
        .unwrap_or(false);
    if totp_ok {
        return true;
    }
    state
        .db
        .consume_two_fa_backup_code(user_id, code)
        .unwrap_or(false)
}

// ── Sessions ──────────────────────────────────────────────────────────────

/// A session as the console sees it.
///
/// The raw token is deliberately absent: the console only needs to point at a
/// session, and echoing tokens back would put a live credential in an HTTP
/// response body and in any log that captures one.
#[derive(serde::Serialize)]
struct SessionView {
    id: String,
    created_at: chrono::DateTime<Utc>,
    expires_at: chrono::DateTime<Utc>,
    /// True for the session making this request.
    current: bool,
}

/// `GET /api/user/sessions`
async fn list_sessions(State(s): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let user = match auth_user(&s, &headers) {
        Ok(user) => user,
        Err(response) => return response,
    };
    let current_id = session_token(&headers)
        .and_then(|token| s.db.session_by_token(token).ok().flatten())
        .map(|session| session.id);

    match s.db.sessions_for_user(&user.id) {
        Ok(sessions) => {
            let views: Vec<SessionView> = sessions
                .into_iter()
                .map(|session| SessionView {
                    current: current_id.as_deref() == Some(session.id.as_str()),
                    id: session.id,
                    created_at: session.created_at,
                    expires_at: session.expires_at,
                })
                .collect();
            Json(ApiResponse::ok(views)).into_response()
        }
        Err(error) => Json(ApiResponse::<Vec<SessionView>>::err(error.to_string())).into_response(),
    }
}

/// `DELETE /api/user/sessions/:sid`
async fn delete_session(
    State(s): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let user = match auth_user(&s, &headers) {
        Ok(user) => user,
        Err(response) => return response,
    };
    match s.db.revoke_session_by_id(&user.id, &session_id) {
        Ok(true) => Json(ApiResponse::ok("revoked")).into_response(),
        Ok(false) => Json(ApiResponse::<&str>::err("session not found")).into_response(),
        Err(error) => Json(ApiResponse::<&str>::err(error.to_string())).into_response(),
    }
}

/// `POST /api/user/sessions/revoke-others`
///
/// Keeps the calling session, so the user is not signed out by the action they
/// just took; the reference does the same.
async fn revoke_other_sessions(State(s): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let user = match auth_user(&s, &headers) {
        Ok(user) => user,
        Err(response) => return response,
    };
    let keep = session_token(&headers)
        .and_then(|token| s.db.session_by_token(token).ok().flatten())
        .map(|session| session.id);
    match s.db.revoke_other_sessions(&user.id, keep.as_deref()) {
        Ok(count) => Json(ApiResponse::ok(count)).into_response(),
        Err(error) => Json(ApiResponse::<usize>::err(error.to_string())).into_response(),
    }
}

// ── Access tokens ─────────────────────────────────────────────────────────

/// A generated token is 29 + 0..4 random alphanumerics, the reference's shape.
///
/// Entropy comes from `Uuid::new_v4`, which is a CSPRNG-backed 122-bit value;
/// the alphabet mapping preserves that rather than narrowing it with a weak
/// source.
fn new_access_token() -> String {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let bytes = uuid::Uuid::new_v4().into_bytes();
    let extra = uuid::Uuid::new_v4().into_bytes();
    let length = 29 + (bytes[0] as usize % 4);
    (0..length)
        .map(|i| {
            let byte = if i < 16 { bytes[i] } else { extra[i - 16] };
            ALPHABET[byte as usize % ALPHABET.len()] as char
        })
        .collect()
}

/// `GET /api/user/token/status`
///
/// Reports whether a token exists and when it was made, never the value itself.
async fn access_token_status(State(s): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let user = match auth_user(&s, &headers) {
        Ok(user) => user,
        Err(response) => return response,
    };
    match s.db.access_token_status(&user.id) {
        Ok(created) => Json(ApiResponse::ok(serde_json::json!({
            "enabled": created.is_some(),
            "created_at": created,
        })))
        .into_response(),
        Err(error) => Json(ApiResponse::<serde_json::Value>::err(error.to_string())).into_response(),
    }
}

/// `POST /api/user/token` (and the `GET` the reference also accepts)
///
/// The value is returned exactly once, here. The status endpoint deliberately
/// cannot re-read it, so a lost token has to be rotated rather than recovered —
/// which is what makes storing it recoverable in the database acceptable.
async fn generate_access_token(State(s): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let user = match auth_user(&s, &headers) {
        Ok(user) => user,
        Err(response) => return response,
    };
    // Retry on the (vanishingly unlikely) collision rather than handing out a
    // credential another user already holds.
    let mut token = new_access_token();
    for _ in 0..5 {
        match s.db.access_token_exists(&token) {
            Ok(false) => break,
            Ok(true) => token = new_access_token(),
            Err(error) => {
                return Json(ApiResponse::<String>::err(error.to_string())).into_response()
            }
        }
    }
    match s.db.set_access_token(&user.id, &token) {
        Ok(()) => Json(ApiResponse::ok(token)).into_response(),
        Err(error) => Json(ApiResponse::<String>::err(error.to_string())).into_response(),
    }
}

/// `DELETE /api/user/token`
async fn revoke_access_token(State(s): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let user = match auth_user(&s, &headers) {
        Ok(user) => user,
        Err(response) => return response,
    };
    match s.db.revoke_access_token(&user.id) {
        Ok(removed) => Json(ApiResponse::ok(removed)).into_response(),
        Err(error) => Json(ApiResponse::<bool>::err(error.to_string())).into_response(),
    }
}

// ── Two-factor authentication ─────────────────────────────────────────────

/// How many recovery codes a setup issues. The reference uses four 8-character
/// codes (`common.BackupCodeCount` / `BackupCodeLength`).
const BACKUP_CODE_COUNT: usize = 4;
const BACKUP_CODE_LENGTH: usize = 8;

/// A recovery code from an unambiguous alphabet.
///
/// `I`/`O`/`0`/`1` are excluded so a code read off a screen or paper cannot be
/// mistyped into a different valid code.
fn new_backup_code() -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut bytes = Vec::new();
    while bytes.len() < BACKUP_CODE_LENGTH {
        bytes.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
    }
    bytes
        .into_iter()
        .take(BACKUP_CODE_LENGTH)
        .map(|b| ALPHABET[b as usize % ALPHABET.len()] as char)
        .collect()
}

/// `GET /api/user/2fa/status`
///
/// Reports whether 2FA is on and how many recovery codes remain. Never the secret.
async fn two_fa_status(State(s): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let user = match auth_user(&s, &headers) {
        Ok(user) => user,
        Err(response) => return response,
    };
    let enabled = match s.db.enabled_two_fa_secret(&user.id) {
        Ok(secret) => secret.is_some(),
        Err(error) => {
            return Json(ApiResponse::<serde_json::Value>::err(error.to_string())).into_response()
        }
    };
    let remaining = s.db.two_fa_backup_code_digests(&user.id).map(|d| d.len()).unwrap_or(0);
    Json(ApiResponse::ok(serde_json::json!({
        "enabled": enabled,
        "backup_codes_remaining": if enabled { remaining } else { 0 },
    })))
    .into_response()
}

/// `POST /api/user/2fa/setup`
///
/// Generates and stages a secret, returning it once with a provisioning URI. The
/// secret is not active until a code is confirmed, so a mis-scanned or
/// discarded QR code cannot lock the account out.
async fn two_fa_setup(State(s): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let user = match auth_user(&s, &headers) {
        Ok(user) => user,
        Err(response) => return response,
    };
    let secret = oxygenrouter_core::totp::generate_secret();
    if let Err(error) = s.db.set_pending_two_fa_secret(&user.id, &secret) {
        return Json(ApiResponse::<serde_json::Value>::err(error.to_string())).into_response();
    }
    let issuer = s
        .db
        .get_setting("SiteName")
        .ok()
        .flatten()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "OxygenRouter".to_string());
    let uri = oxygenrouter_core::totp::provisioning_uri(&issuer, &user.username, &secret);
    Json(ApiResponse::ok(serde_json::json!({
        "secret": secret,
        "otpauth_uri": uri,
        "issuer": issuer,
        "account": user.username,
    })))
    .into_response()
}

#[derive(serde::Deserialize)]
struct TwoFaCodeInput {
    code: String,
}

/// `POST /api/user/2fa/enable`
///
/// Activates two-factor once the caller proves they can generate a valid code,
/// and returns the recovery codes — the only time they are shown.
async fn two_fa_enable(
    State(s): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(input): Json<TwoFaCodeInput>,
) -> Response {
    let user = match auth_user(&s, &headers) {
        Ok(user) => user,
        Err(response) => return response,
    };
    let secret = match s.db.pending_two_fa_secret(&user.id) {
        Ok(Some(secret)) => secret,
        Ok(None) => {
            return Json(ApiResponse::<serde_json::Value>::err(
                "start a two-factor setup first",
            ))
            .into_response()
        }
        Err(error) => {
            return Json(ApiResponse::<serde_json::Value>::err(error.to_string())).into_response()
        }
    };
    let decoded = match oxygenrouter_core::totp::base32_decode(&secret) {
        Some(bytes) => bytes,
        None => {
            return Json(ApiResponse::<serde_json::Value>::err(
                "stored secret is not valid base32",
            ))
            .into_response()
        }
    };
    if !oxygenrouter_core::totp::verify(&decoded, &input.code) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(ApiResponse::<serde_json::Value>::err("invalid verification code")),
        )
            .into_response();
    }

    let codes: Vec<String> = (0..BACKUP_CODE_COUNT).map(|_| new_backup_code()).collect();
    if let Err(error) = s.db.enable_two_fa(&user.id, &codes) {
        return Json(ApiResponse::<serde_json::Value>::err(error.to_string())).into_response();
    }
    Json(ApiResponse::ok(serde_json::json!({
        "enabled": true,
        "backup_codes": codes,
    })))
    .into_response()
}

/// `POST /api/user/2fa/disable`
///
/// Requires a valid code or recovery code: turning 2FA off is exactly the action
/// an attacker with a stolen session would want, so it is gated by the second
/// factor rather than by the session alone.
async fn two_fa_disable(
    State(s): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(input): Json<TwoFaCodeInput>,
) -> Response {
    let user = match auth_user(&s, &headers) {
        Ok(user) => user,
        Err(response) => return response,
    };
    let secret = match s.db.enabled_two_fa_secret(&user.id) {
        Ok(Some(secret)) => secret,
        Ok(None) => {
            return Json(ApiResponse::<serde_json::Value>::err(
                "two-factor is not enabled",
            ))
            .into_response()
        }
        Err(error) => {
            return Json(ApiResponse::<serde_json::Value>::err(error.to_string())).into_response()
        }
    };
    let valid = oxygenrouter_core::totp::base32_decode(&secret)
        .map(|decoded| oxygenrouter_core::totp::verify(&decoded, &input.code))
        .unwrap_or(false)
        || s.db
            .consume_two_fa_backup_code(&user.id, &input.code)
            .unwrap_or(false);
    if !valid {
        return (
            StatusCode::UNAUTHORIZED,
            Json(ApiResponse::<serde_json::Value>::err("invalid verification code")),
        )
            .into_response();
    }
    match s.db.disable_two_fa(&user.id) {
        Ok(()) => Json(ApiResponse::ok("disabled")).into_response(),
        Err(error) => Json(ApiResponse::<&str>::err(error.to_string())).into_response(),
    }
}

/// `POST /api/user/2fa/backup_codes`
///
/// Reissues recovery codes, invalidating the previous set.
async fn two_fa_backup_codes(
    State(s): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(input): Json<TwoFaCodeInput>,
) -> Response {
    let user = match auth_user(&s, &headers) {
        Ok(user) => user,
        Err(response) => return response,
    };
    let secret = match s.db.enabled_two_fa_secret(&user.id) {
        Ok(Some(secret)) => secret,
        Ok(None) => {
            return Json(ApiResponse::<serde_json::Value>::err(
                "two-factor is not enabled",
            ))
            .into_response()
        }
        Err(error) => {
            return Json(ApiResponse::<serde_json::Value>::err(error.to_string())).into_response()
        }
    };
    let valid = oxygenrouter_core::totp::base32_decode(&secret)
        .map(|decoded| oxygenrouter_core::totp::verify(&decoded, &input.code))
        .unwrap_or(false);
    if !valid {
        return (
            StatusCode::UNAUTHORIZED,
            Json(ApiResponse::<serde_json::Value>::err("invalid verification code")),
        )
            .into_response();
    }
    let codes: Vec<String> = (0..BACKUP_CODE_COUNT).map(|_| new_backup_code()).collect();
    if let Err(error) = s.db.enable_two_fa(&user.id, &codes) {
        return Json(ApiResponse::<serde_json::Value>::err(error.to_string())).into_response();
    }
    Json(ApiResponse::ok(serde_json::json!({"backup_codes": codes}))).into_response()
}
async fn wallet(State(s): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let user = match auth_user(&s, &headers) {
        Ok(user) => user,
        Err(response) => return response,
    };
    match s.db.list_ledger(&user.id) {
        Ok(ledger) => Json(ApiResponse::ok(
            serde_json::json!({"balance_micros":user.balance_micros,"ledger":ledger}),
        ))
        .into_response(),
        Err(e) => db_error::<serde_json::Value>(e),
    }
}
async fn my_subscriptions(State(s): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let user = match auth_user(&s, &headers) {
        Ok(user) => user,
        Err(response) => return response,
    };
    match s.db.list_subscriptions(&user.id) {
        Ok(value) => Json(ApiResponse::ok(value)).into_response(),
        Err(e) => db_error::<Vec<oxygenrouter_core::Subscription>>(e),
    }
}
#[derive(Deserialize)]
struct SubscribeInput {
    plan_id: String,
}
#[derive(Serialize)]
struct SubscribeResponse {
    subscription: oxygenrouter_core::Subscription,
    balance_micros: i64,
}
async fn subscribe(
    State(s): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(input): Json<SubscribeInput>,
) -> Response {
    let user = match auth_user(&s, &headers) {
        Ok(user) => user,
        Err(response) => return response,
    };
    match s.db.subscribe(&user.id, &input.plan_id) {
        Ok(subscription) => {
            let balance_micros = match s.db.get_user(&user.id) {
                Ok(Some(user)) => user.balance_micros,
                Ok(None) => {
                    return db_error::<SubscribeResponse>(rusqlite::Error::QueryReturnedNoRows)
                }
                Err(e) => return db_error::<SubscribeResponse>(e),
            };
            Json(ApiResponse::ok(SubscribeResponse {
                subscription,
                balance_micros,
            }))
            .into_response()
        }
        Err(e) if e.to_string().contains("subscription already active") => (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse::<SubscribeResponse>::err(
                "subscription already active for this plan",
            )),
        )
            .into_response(),
        Err(e) if e.to_string().contains("insufficient balance") => (
            StatusCode::PAYMENT_REQUIRED,
            Json(ApiResponse::<SubscribeResponse>::err(
                "insufficient balance for subscription purchase",
            )),
        )
            .into_response(),
        Err(e) => db_error::<SubscribeResponse>(e),
    }
}
async fn public_plans(State(s): State<Arc<AppState>>) -> Response {
    match s.db.list_plans(true) {
        Ok(value) => Json(ApiResponse::ok(value)).into_response(),
        Err(e) => db_error::<Vec<SubscriptionPlan>>(e),
    }
}
#[derive(Deserialize)]
struct RedeemInput {
    code: String,
}
async fn redeem_code(
    State(s): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(input): Json<RedeemInput>,
) -> Response {
    let user = match auth_user(&s, &headers) {
        Ok(user) => user,
        Err(response) => return response,
    };
    match s.db.redeem(&user.id, &input.code) {
        Ok(()) => Json(ApiResponse::ok("redeemed")).into_response(),
        Err(e) => db_error::<&str>(e),
    }
}
#[derive(Deserialize)]
struct ManualOrderInput {
    amount_micros: i64,
}
async fn create_manual_order(
    State(s): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(input): Json<ManualOrderInput>,
) -> Response {
    let user = match auth_user(&s, &headers) {
        Ok(user) => user,
        Err(response) => return response,
    };
    match s.db.create_manual_order(&user.id, input.amount_micros) {
        Ok(order) => Json(ApiResponse::ok(order)).into_response(),
        Err(e) => db_error::<PaymentOrder>(e),
    }
}

// ── SaaS administration ────────────────────────────────────────────────────

#[derive(Deserialize)]
struct UsersQuery {
    search: Option<String>,
}
#[derive(Deserialize)]
struct AdminUserInput {
    username: String,
    email: String,
    password: Option<String>,
    role: UserRole,
    status: String,
}
#[derive(Deserialize)]
struct BalanceAdjustmentInput {
    amount_micros: i64,
    description: String,
}
async fn admin_users(
    State(s): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<UsersQuery>,
) -> Response {
    if let Err(response) = admin_user(&s, &headers) {
        return response;
    };
    match s.db.list_users(query.search.as_deref()) {
        Ok(users) => Json(ApiResponse::ok(users)).into_response(),
        Err(e) => db_error::<Vec<User>>(e),
    }
}
async fn admin_create_user(
    State(s): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(input): Json<AdminUserInput>,
) -> Response {
    if let Err(response) = admin_user(&s, &headers) {
        return response;
    };
    let Some(password) = input.password else {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse::<User>::err("password required")),
        )
            .into_response();
    };
    match s
        .db
        .create_user(&input.username, &input.email, &password, input.role)
    {
        Ok(mut user) => {
            if input.status != "active" {
                match s.db.update_user(
                    &user.id,
                    &user.username,
                    &user.email,
                    user.role.clone(),
                    &input.status,
                ) {
                    Ok(Some(value)) => user = value,
                    Ok(None) => {}
                    Err(e) => return db_error::<User>(e),
                }
            }
            Json(ApiResponse::ok(user)).into_response()
        }
        Err(e) => db_error::<User>(e),
    }
}
async fn admin_update_user(
    State(s): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<AdminUserInput>,
) -> Response {
    if let Err(response) = admin_user(&s, &headers) {
        return response;
    };
    match s.db.update_user(
        &id,
        &input.username,
        &input.email,
        input.role,
        &input.status,
    ) {
        Ok(Some(user)) => Json(ApiResponse::ok(user)).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::<User>::err("user not found")),
        )
            .into_response(),
        Err(e) => db_error::<User>(e),
    }
}
async fn admin_delete_user(
    State(s): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let admin = match admin_user(&s, &headers) {
        Ok(user) => user,
        Err(response) => return response,
    };
    if admin.id == id {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse::<&str>::err("cannot delete current admin")),
        )
            .into_response();
    }
    match s.db.delete_user(&id) {
        Ok(true) => Json(ApiResponse::ok("deleted")).into_response(),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::<&str>::err("user not found")),
        )
            .into_response(),
        Err(e) => db_error::<&str>(e),
    }
}
async fn admin_adjust_balance(
    State(s): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<BalanceAdjustmentInput>,
) -> Response {
    if let Err(response) = admin_user(&s, &headers) {
        return response;
    };
    match s
        .db
        .admin_adjust_balance(&id, input.amount_micros, &input.description)
    {
        Ok(entry) => Json(ApiResponse::ok(entry)).into_response(),
        Err(e) => db_error::<oxygenrouter_core::LedgerEntry>(e),
    }
}

#[derive(Deserialize)]
struct PlanInput {
    name: String,
    description: String,
    price_micros: i64,
    quota_micros: i64,
    duration_days: i64,
    enabled: bool,
}
fn make_plan(id: String, input: PlanInput, created_at: chrono::DateTime<Utc>) -> SubscriptionPlan {
    SubscriptionPlan {
        id,
        name: input.name,
        description: input.description,
        price_micros: input.price_micros,
        quota_micros: input.quota_micros,
        duration_days: input.duration_days,
        enabled: input.enabled,
        created_at,
        updated_at: Utc::now(),
    }
}
async fn admin_plans(State(s): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    if let Err(response) = admin_user(&s, &headers) {
        return response;
    };
    match s.db.list_plans(false) {
        Ok(value) => Json(ApiResponse::ok(value)).into_response(),
        Err(e) => db_error::<Vec<SubscriptionPlan>>(e),
    }
}
async fn admin_create_plan(
    State(s): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(input): Json<PlanInput>,
) -> Response {
    if let Err(response) = admin_user(&s, &headers) {
        return response;
    };
    let plan = make_plan(uuid::Uuid::new_v4().to_string(), input, Utc::now());
    match s.db.upsert_plan(&plan) {
        Ok(()) => Json(ApiResponse::ok(plan)).into_response(),
        Err(e) => db_error::<SubscriptionPlan>(e),
    }
}
async fn admin_update_plan(
    State(s): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<PlanInput>,
) -> Response {
    if let Err(response) = admin_user(&s, &headers) {
        return response;
    };
    let existing =
        s.db.list_plans(false)
            .ok()
            .and_then(|plans| plans.into_iter().find(|plan| plan.id == id));
    let Some(existing) = existing else {
        return (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::<SubscriptionPlan>::err("plan not found")),
        )
            .into_response();
    };
    let plan = make_plan(id, input, existing.created_at);
    match s.db.upsert_plan(&plan) {
        Ok(()) => Json(ApiResponse::ok(plan)).into_response(),
        Err(e) => db_error::<SubscriptionPlan>(e),
    }
}

// ── Admin: user subscription lifecycle ────────────────────────────────────

#[derive(serde::Deserialize)]
struct GrantSubscriptionInput {
    user_id: String,
    plan_id: String,
}

/// `POST /api/subscription/admin/bind`
///
/// Grants a plan to a user without charging them — an admin gift, matching the
/// reference's `AdminBindSubscription`, which likewise does not debit. The plan
/// need not be enabled: an operator may bind a retired plan deliberately.
async fn admin_bind_subscription(
    State(s): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(input): Json<GrantSubscriptionInput>,
) -> Response {
    if let Err(response) = admin_user(&s, &headers) {
        return response;
    }
    if input.user_id.trim().is_empty() || input.plan_id.trim().is_empty() {
        return Json(ApiResponse::<oxygenrouter_core::Subscription>::err(
            "user_id and plan_id are required",
        ))
        .into_response();
    }
    match s.db.grant_subscription(&input.user_id, &input.plan_id) {
        Ok(subscription) => Json(ApiResponse::ok(subscription)).into_response(),
        Err(e) if e.to_string().contains("subscription already active") => (
            StatusCode::CONFLICT,
            Json(ApiResponse::<oxygenrouter_core::Subscription>::err(
                "subscription already active for this plan",
            )),
        )
            .into_response(),
        Err(e) if e.to_string().contains("Query returned no rows") => (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::<oxygenrouter_core::Subscription>::err(
                "plan not found",
            )),
        )
            .into_response(),
        Err(e) => db_error::<oxygenrouter_core::Subscription>(e),
    }
}

/// `GET /api/subscription/admin/users/:id/subscriptions`
async fn admin_user_subscriptions(
    State(s): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(user_id): Path<String>,
) -> Response {
    if let Err(response) = admin_user(&s, &headers) {
        return response;
    }
    match s.db.list_all_subscriptions(&user_id) {
        Ok(value) => Json(ApiResponse::ok(value)).into_response(),
        Err(e) => db_error::<Vec<oxygenrouter_core::Subscription>>(e),
    }
}

/// `POST /api/subscription/admin/user_subscriptions/:id/invalidate`
///
/// Ends a live subscription. Already-inactive rows are reported as not found
/// rather than silently rewritten, so an operator can tell the difference
/// between "I just ended it" and "it was already over".
async fn admin_invalidate_subscription(
    State(s): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(response) = admin_user(&s, &headers) {
        return response;
    }
    match s.db.invalidate_subscription(&id) {
        Ok(true) => Json(ApiResponse::ok("invalidated")).into_response(),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::<&str>::err("no active subscription with that id")),
        )
            .into_response(),
        Err(e) => db_error::<&str>(e),
    }
}

/// `DELETE /api/subscription/admin/user_subscriptions/:id`
///
/// Erases the record. Distinct from invalidating: this is for removing a mistaken
/// grant, not for ending a live entitlement.
async fn admin_delete_subscription(
    State(s): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(response) = admin_user(&s, &headers) {
        return response;
    }
    match s.db.delete_subscription(&id) {
        Ok(true) => Json(ApiResponse::ok("deleted")).into_response(),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::<&str>::err("subscription not found")),
        )
            .into_response(),
        Err(e) => db_error::<&str>(e),
    }
}

/// `POST /api/subscription/admin/users/:id/subscriptions/reset`
///
/// Ends every active subscription the user holds on one plan.
async fn admin_reset_user_subscriptions(
    State(s): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(user_id): Path<String>,
    Json(input): Json<ResetSubscriptionsInput>,
) -> Response {
    if let Err(response) = admin_user(&s, &headers) {
        return response;
    }
    match s.db.reset_subscriptions_for_plan(&user_id, &input.plan_id) {
        Ok(count) => Json(ApiResponse::ok(count)).into_response(),
        Err(e) => db_error::<usize>(e),
    }
}

#[derive(serde::Deserialize)]
struct ResetSubscriptionsInput {
    plan_id: String,
}

/// `GET /api/subscription/admin/plans/:id/subscriptions`
///
/// Who is on this plan, across all users.
async fn admin_plan_subscriptions(
    State(s): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(plan_id): Path<String>,
) -> Response {
    if let Err(response) = admin_user(&s, &headers) {
        return response;
    }
    match s.db.list_subscriptions_for_plan(&plan_id) {
        Ok(value) => Json(ApiResponse::ok(value)).into_response(),
        Err(e) => db_error::<Vec<oxygenrouter_core::Subscription>>(e),
    }
}

/// `POST /api/subscription/admin/plans/:id/subscriptions/reset`
///
/// Ends every active subscription on a plan, for every user.
async fn admin_reset_plan_subscriptions(
    State(s): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(plan_id): Path<String>,
) -> Response {
    if let Err(response) = admin_user(&s, &headers) {
        return response;
    }
    // Implemented over the per-user primitive so there is one definition of
    // "end a subscription"; a separate bulk UPDATE could drift from it.
    let subscriptions = match s.db.list_subscriptions_for_plan(&plan_id) {
        Ok(value) => value,
        Err(e) => return db_error::<usize>(e),
    };
    let mut ended = 0usize;
    for subscription in subscriptions {
        match s
            .db
            .reset_subscriptions_for_plan(&subscription.user_id, &plan_id)
        {
            Ok(n) => ended += n,
            Err(e) => return db_error::<usize>(e),
        }
    }
    Json(ApiResponse::ok(ended)).into_response()
}

async fn admin_delete_plan(
    State(s): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(response) = admin_user(&s, &headers) {
        return response;
    };
    match s.db.delete_plan(&id) {
        Ok(true) => Json(ApiResponse::ok("deleted")).into_response(),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::<&str>::err("plan not found")),
        )
            .into_response(),
        Err(e) => db_error::<&str>(e),
    }
}

#[derive(Deserialize)]
struct CodeInput {
    code: String,
    amount_micros: i64,
    plan_id: Option<String>,
    enabled: bool,
    max_uses: i64,
    expires_at: Option<chrono::DateTime<Utc>>,
}
fn make_code(
    id: String,
    input: CodeInput,
    created_at: chrono::DateTime<Utc>,
    used_count: i64,
) -> RedemptionCode {
    RedemptionCode {
        id,
        code: input.code,
        amount_micros: input.amount_micros,
        plan_id: input.plan_id,
        enabled: input.enabled,
        max_uses: input.max_uses,
        used_count,
        expires_at: input.expires_at,
        created_at,
    }
}
async fn admin_codes(State(s): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    if let Err(response) = admin_user(&s, &headers) {
        return response;
    };
    match s.db.list_redemption_codes() {
        Ok(value) => Json(ApiResponse::ok(value)).into_response(),
        Err(e) => db_error::<Vec<RedemptionCode>>(e),
    }
}
async fn admin_create_code(
    State(s): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(input): Json<CodeInput>,
) -> Response {
    if let Err(response) = admin_user(&s, &headers) {
        return response;
    };
    let code = make_code(uuid::Uuid::new_v4().to_string(), input, Utc::now(), 0);
    match s.db.upsert_redemption_code(&code) {
        Ok(()) => Json(ApiResponse::ok(code)).into_response(),
        Err(e) => db_error::<RedemptionCode>(e),
    }
}
async fn admin_update_code(
    State(s): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<CodeInput>,
) -> Response {
    if let Err(response) = admin_user(&s, &headers) {
        return response;
    };
    let existing =
        s.db.list_redemption_codes()
            .ok()
            .and_then(|codes| codes.into_iter().find(|code| code.id == id));
    let Some(existing) = existing else {
        return (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::<RedemptionCode>::err(
                "redemption code not found",
            )),
        )
            .into_response();
    };
    let code = make_code(id, input, existing.created_at, existing.used_count);
    match s.db.upsert_redemption_code(&code) {
        Ok(()) => Json(ApiResponse::ok(code)).into_response(),
        Err(e) => db_error::<RedemptionCode>(e),
    }
}
async fn admin_delete_code(
    State(s): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(response) = admin_user(&s, &headers) {
        return response;
    };
    match s.db.delete_redemption_code(&id) {
        Ok(true) => Json(ApiResponse::ok("deleted")).into_response(),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::<&str>::err("redemption code not found")),
        )
            .into_response(),
        Err(e) => db_error::<&str>(e),
    }
}
async fn admin_orders(State(s): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    if let Err(response) = admin_user(&s, &headers) {
        return response;
    };
    match s.db.list_orders() {
        Ok(value) => Json(ApiResponse::ok(value)).into_response(),
        Err(e) => db_error::<Vec<PaymentOrder>>(e),
    }
}
async fn admin_complete_order(
    State(s): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(response) = admin_user(&s, &headers) {
        return response;
    };
    match s.db.complete_manual_order(&id) {
        Ok(Some(order)) => Json(ApiResponse::ok(order)).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::<PaymentOrder>::err("order not found")),
        )
            .into_response(),
        Err(e) => db_error::<PaymentOrder>(e),
    }
}
#[derive(Deserialize)]
struct LedgerQuery {
    user_id: String,
}
async fn admin_ledger(
    State(s): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<LedgerQuery>,
) -> Response {
    if let Err(response) = admin_user(&s, &headers) {
        return response;
    };
    match s.db.list_ledger(&query.user_id) {
        Ok(value) => Json(ApiResponse::ok(value)).into_response(),
        Err(e) => db_error::<Vec<oxygenrouter_core::LedgerEntry>>(e),
    }
}

/// `GET /api/authz/catalog` — the permission schema for a permission editor.
///
/// Admin-gated like the reference (`router/authz-router.go:14-17`), because the
/// catalog enumerates what an admin may do, including the privileges they do
/// *not* hold.
async fn authz_catalog(State(s): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    if let Err(response) = admin_user(&s, &headers) {
        return response;
    }
    Json(ApiResponse::ok(oxygenrouter_core::authz::catalog())).into_response()
}

// ── Channels ────────────────────────────────────────────────────────────────

async fn list_channels(State(s): State<Arc<AppState>>) -> Json<ApiResponse<Vec<Channel>>> {
    Json(match s.db.list_channels() {
        Ok(chs) => ApiResponse::ok(chs),
        Err(e) => ApiResponse::err(e.to_string()),
    })
}

async fn create_channel(
    State(s): State<Arc<AppState>>,
    Json(ch): Json<Channel>,
) -> Json<ApiResponse<Channel>> {
    let now = chrono::Utc::now();
    let key_count = ch.api_key.lines().map(str::trim).filter(|line| !line.is_empty()).count();
    let mut info = ch.info;
    info.is_multi_key = key_count > 1;
    info.multi_key_size = key_count;
    let ch = Channel {
        id: uuid::Uuid::new_v4().to_string(),
        name: ch.name,
        provider: if ch.provider.is_empty() {
            "openai".to_string()
        } else {
            ch.provider
        },
        base_url: ch.base_url,
        api_key: ch.api_key,
        priority: ch.priority,
        weight: ch.weight,
        enabled: ch.enabled,
        test_model: ch.test_model,
        group_name: ch.group_name,
        tags: ch.tags,
        model_list: ch.model_list,
        response_headers: ch.response_headers,
        status_code_mapping: ch.status_code_mapping,
        override_parameters: ch.override_parameters,
        balance_micros: ch.balance_micros,
        last_test_at: ch.last_test_at,
        info,
        created_at: now,
        updated_at: now,
    };
    Json(match s.db.upsert_channel(&ch) {
        Ok(()) => ApiResponse::ok(ch),
        Err(e) => ApiResponse::err(e.to_string()),
    })
}

async fn get_channel(
    State(s): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Json<ApiResponse<Channel>> {
    Json(match s.db.get_channel(&id) {
        Ok(Some(c)) => ApiResponse::ok(c),
        Ok(None) => ApiResponse::err("channel not found"),
        Err(e) => ApiResponse::err(e.to_string()),
    })
}

async fn update_channel(
    State(s): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(ch): Json<Channel>,
) -> Json<ApiResponse<Channel>> {
    let mut ch = ch;
    ch.id = id;
    ch.updated_at = chrono::Utc::now();
    Json(match s.db.upsert_channel(&ch) {
        Ok(()) => ApiResponse::ok(ch),
        Err(e) => ApiResponse::err(e.to_string()),
    })
}

async fn delete_channel(
    State(s): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Json<ApiResponse<&'static str>> {
    Json(match s.db.delete_channel(&id) {
        Ok(()) => ApiResponse::ok("deleted"),
        Err(e) => ApiResponse::err(e.to_string()),
    })
}

async fn test_channel(
    State(s): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Json<ApiResponse<oxygenrouter_core::ChannelTestResult>> {
    let ch = match s.db.get_channel(&id) {
        Ok(Some(c)) => c,
        Ok(None) => return Json(ApiResponse::err("channel not found")),
        Err(e) => return Json(ApiResponse::err(e.to_string())),
    };
    let result = oxygenrouter_proxy::test_channel(&ch).await;
    Json(ApiResponse::ok(result))
}

async fn fetch_channel_models(
    State(s): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    let ch = match s.db.get_channel(&id) {
        Ok(Some(c)) => c,
        Ok(None) => return (StatusCode::NOT_FOUND, Json(ApiResponse::<Channel>::err("channel not found"))).into_response(),
        Err(e) => return db_error::<Channel>(e),
    };
    let base = ch.base_url.trim_end_matches('/');
    let url = if base.ends_with("/v1") { format!("{}/models", base) } else { format!("{}/v1/models", base) };
    // This is a server-side fetch of a stored URL, so it is SSRF-checked. The
    // relay path deliberately is not: NewAPI exempts provider base URLs because
    // they are operator-managed deployment targets that may legitimately be
    // private (a LAN vLLM or Ollama host), whereas this endpoint tells the
    // server to dereference a URL on demand.
    if let Err(error) = s.fetch_policy.validate_url(&url) {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiResponse::<Channel>::err(format!(
                "ssrf protection refused this target: {error}"
            ))),
        )
            .into_response();
    }
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        // Redirects are not followed: a permitted host could otherwise bounce
        // the request to an internal one and defeat the check above.
        .redirect(reqwest::redirect::Policy::none())
        .build()
    {
        Ok(c) => c,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };
    let resp = match client
        .get(&url)
        .header("Authorization", format!("Bearer {}", ch.api_key.lines().next().unwrap_or_default()))
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => return (StatusCode::BAD_GATEWAY, Json(ApiResponse::<Channel>::err(format!("upstream request failed: {e}")))).into_response(),
    };
    let status = resp.status();
    let body = match resp.text().await {
        Ok(b) => b,
        Err(e) => return (StatusCode::BAD_GATEWAY, Json(ApiResponse::<Channel>::err(format!("reading response: {e}")))).into_response(),
    };
    if !status.is_success() {
        return (StatusCode::BAD_GATEWAY, Json(ApiResponse::<Channel>::err(format!("upstream HTTP {status}: {}", body.chars().take(200).collect::<String>())))).into_response();
    }
    let models: Vec<String> = serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|v| v.get("data").and_then(|d| d.as_array()).map(|arr| {
            arr.iter().filter_map(|m| m.get("id").and_then(|id| id.as_str()).map(String::from)).collect::<Vec<_>>()
        }))
        .unwrap_or_default();
    if models.is_empty() {
        return (StatusCode::BAD_REQUEST, Json(ApiResponse::<Channel>::err("no models returned from upstream"))).into_response();
    }
    match s.db.update_channel_models(&id, &models) {
        Ok(()) => {
            let mut updated = ch;
            updated.model_list = models;
            Json(ApiResponse::ok(updated)).into_response()
        }
        Err(e) => db_error::<Channel>(e),
    }
}

#[derive(serde::Deserialize)]
struct BatchUpdateInput {
    ids: Vec<String>,
    enabled: bool,
}


#[derive(serde::Deserialize)]
struct BatchDeleteInput {
    ids: Vec<String>,
}

async fn batch_update_channels(
    State(s): State<Arc<AppState>>,
    Json(input): Json<BatchUpdateInput>,
) -> Json<ApiResponse<usize>> {
    let n = input.ids.len();
    Json(
        match s.db.batch_update_channels(&input.ids, input.enabled) {
            Ok(()) => ApiResponse::ok(n),
            Err(e) => ApiResponse::err(e.to_string()),
        },
    )
}

async fn batch_delete_channels(
    State(s): State<Arc<AppState>>,
    Json(input): Json<BatchDeleteInput>,
) -> Json<ApiResponse<usize>> {
    Json(match s.db.batch_delete_channels(&input.ids) {
        Ok(n) => ApiResponse::ok(n),
        Err(e) => ApiResponse::err(e.to_string()),
    })
}

// ── Paginated Response ────────────────────────────────────────────────────

#[derive(serde::Serialize)]
pub struct PaginatedResponse<T: serde::Serialize> {
    pub items: Vec<T>,
    pub total: i64,
    pub page: i64,
    pub page_size: i64,
}

impl<T: serde::Serialize> PaginatedResponse<T> {
    pub fn new(items: Vec<T>, total: i64, page: i64, page_size: i64) -> Self {
        Self { items, total, page, page_size }
    }
}

// ── Channel multi-key management ─────────────────────────────────────────

fn key_preview(key: &str) -> String {
    let trimmed = key.trim();
    if trimmed.len() <= 10 {
        return format!("{trimmed}…");
    }
    format!("{}…", &trimmed[..10])
}

async fn list_channel_keys(
    State(s): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Json<ApiResponse<Vec<oxygenrouter_core::ChannelKeyStatus>>> {
    let channel = match s.db.get_channel(&id) {
        Ok(Some(channel)) => channel,
        Ok(None) => return Json(ApiResponse::err("channel not found")),
        Err(error) => return Json(ApiResponse::err(error.to_string())),
    };
    let statuses = channel
        .keys()
        .iter()
        .enumerate()
        .map(|(index, key)| oxygenrouter_core::ChannelKeyStatus {
            index,
            preview: key_preview(key),
            status: channel.key_status(index),
            disabled_reason: channel.info.multi_key_disabled_reason.get(&index).cloned(),
            disabled_time: channel.info.multi_key_disabled_time.get(&index).copied(),
        })
        .collect::<Vec<_>>();
    Json(ApiResponse::ok(statuses))
}

#[derive(serde::Deserialize)]
struct ManageKeysInput {
    action: String,
    #[serde(default)]
    index: Option<usize>,
    #[serde(default)]
    reason: Option<String>,
    /// newline-separated keys when appending
    #[serde(default)]
    keys: Option<String>,
}

async fn manage_channel_keys(
    State(s): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(input): Json<ManageKeysInput>,
) -> Json<ApiResponse<oxygenrouter_core::Channel>> {
    let mut channel = match s.db.get_channel(&id) {
        Ok(Some(channel)) => channel,
        Ok(None) => return Json(ApiResponse::err("channel not found")),
        Err(error) => return Json(ApiResponse::err(error.to_string())),
    };
    let mut keys = channel.keys();
    match input.action.as_str() {
        "get_key_status" => {}
        "disable_key" => {
            let Some(index) = input.index else {
                return Json(ApiResponse::err("index is required"));
            };
            if index >= keys.len() {
                return Json(ApiResponse::err("index out of range"));
            }
            channel.info.multi_key_status_list.insert(index, 2);
            channel.info.multi_key_disabled_reason.insert(
                index,
                input.reason.unwrap_or_else(|| "disabled by operator".to_string()),
            );
            channel.info.multi_key_disabled_time.insert(index, Utc::now().timestamp());
        }
        "enable_key" => {
            let Some(index) = input.index else {
                return Json(ApiResponse::err("index is required"));
            };
            channel.info.multi_key_status_list.remove(&index);
            channel.info.multi_key_disabled_reason.remove(&index);
            channel.info.multi_key_disabled_time.remove(&index);
        }
        "enable_all_keys" => {
            channel.info.multi_key_status_list.clear();
            channel.info.multi_key_disabled_reason.clear();
            channel.info.multi_key_disabled_time.clear();
        }
        "disable_all_keys" => {
            for index in 0..keys.len() {
                channel.info.multi_key_status_list.insert(index, 2);
            }
        }
        "delete_key" => {
            let Some(index) = input.index else {
                return Json(ApiResponse::err("index is required"));
            };
            if keys.len() <= 1 {
                return Json(ApiResponse::err("cannot delete the last key"));
            }
            if index >= keys.len() {
                return Json(ApiResponse::err("index out of range"));
            }
            keys.remove(index);
            // re-index status maps
            let mut status = std::collections::BTreeMap::new();
            let mut reason = std::collections::BTreeMap::new();
            let mut time = std::collections::BTreeMap::new();
            for (old_index, key_status) in channel.info.multi_key_status_list.clone() {
                if old_index == index {
                    continue;
                }
                let new_index = if old_index > index { old_index - 1 } else { old_index };
                status.insert(new_index, key_status);
                if let Some(value) = channel.info.multi_key_disabled_reason.get(&old_index) {
                    reason.insert(new_index, value.clone());
                }
                if let Some(value) = channel.info.multi_key_disabled_time.get(&old_index) {
                    time.insert(new_index, *value);
                }
            }
            channel.info.multi_key_status_list = status;
            channel.info.multi_key_disabled_reason = reason;
            channel.info.multi_key_disabled_time = time;
        }
        "delete_disabled_keys" => {
            let removed = channel
                .info
                .multi_key_status_list
                .iter()
                .filter(|(_, status)| **status == 3)
                .map(|(index, _)| *index)
                .collect::<Vec<_>>();
            for index in removed.into_iter().rev() {
                if keys.len() <= 1 {
                    break;
                }
                keys.remove(index);
            }
            channel.info.multi_key_status_list.clear();
            channel.info.multi_key_disabled_reason.clear();
            channel.info.multi_key_disabled_time.clear();
        }
        "set_mode" => {
            channel.info.multi_key_mode = input.reason.unwrap_or_else(|| "random".to_string());
        }
        "append_keys" => {
            if let Some(raw) = input.keys {
                for key in raw.lines().map(str::trim).filter(|line| !line.is_empty()) {
                    if !keys.iter().any(|existing| existing == key) {
                        keys.push(key.to_string());
                    }
                }
            }
        }
        other => return Json(ApiResponse::err(format!("unknown action: {other}"))),
    }
    channel.info.is_multi_key = keys.len() > 1;
    channel.info.multi_key_size = keys.len();
    // prune stale statuses beyond the new size
    let size = keys.len();
    channel.info.multi_key_status_list.retain(|index, _| *index < size);
    let result = s.db.replace_channel_keys(&id, &keys, &channel.info);
    let keys_joined = keys.join("\n");
    match result {
        Ok(()) => {
            channel.api_key = keys_joined;
            Json(ApiResponse::ok(channel))
        }
        Err(error) => Json(ApiResponse::err(error.to_string())),
    }
}

// ── Model metadata registry ───────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct ModelMetadataQuery {
    page: Option<i64>,
    page_size: Option<i64>,
    search: Option<String>,
}

async fn list_model_metadata(
    State(s): State<Arc<AppState>>,
    Query(q): Query<ModelMetadataQuery>,
) -> Response {
    let page = q.page.unwrap_or(1).max(1);
    let page_size = q.page_size.unwrap_or(20).clamp(1, 200);
    match s.db.list_model_metadata_paged(page, page_size, q.search.as_deref()) {
        Ok((items, total)) => Json(PaginatedResponse::new(items, total, page, page_size)).into_response(),
        Err(error) => db_error::<Vec<oxygenrouter_core::ModelMetadata>>(error),
    }
}

async fn create_model_metadata(
    State(s): State<Arc<AppState>>,
    Json(mut input): Json<oxygenrouter_core::ModelMetadata>,
) -> Json<ApiResponse<oxygenrouter_core::ModelMetadata>> {
    let now = Utc::now();
    if input.id.is_empty() {
        input.id = uuid::Uuid::new_v4().to_string();
    }
    input.created_at = now;
    input.updated_at = now;
    Json(match s.db.upsert_model_metadata(&input) {
        Ok(()) => ApiResponse::ok(input),
        Err(error) => ApiResponse::err(error.to_string()),
    })
}

async fn update_model_metadata(
    State(s): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(mut input): Json<oxygenrouter_core::ModelMetadata>,
) -> Json<ApiResponse<oxygenrouter_core::ModelMetadata>> {
    input.id = id;
    input.updated_at = Utc::now();
    Json(match s.db.upsert_model_metadata(&input) {
        Ok(()) => ApiResponse::ok(input),
        Err(error) => ApiResponse::err(error.to_string()),
    })
}

async fn delete_model_metadata(
    State(s): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Json<ApiResponse<&'static str>> {
    Json(match s.db.delete_model_metadata(&id) {
        Ok(()) => ApiResponse::ok("deleted"),
        Err(error) => ApiResponse::err(error.to_string()),
    })
}

// ── Vendors ───────────────────────────────────────────────────────────────

/// Every vendor, each carrying a model count derived from the registry.
async fn list_vendors(
    State(s): State<Arc<AppState>>,
) -> Json<ApiResponse<Vec<oxygenrouter_core::Vendor>>> {
    Json(match s.db.list_vendors() {
        Ok(rows) => ApiResponse::ok(rows),
        Err(error) => ApiResponse::err(error.to_string()),
    })
}

/// Substring search over vendor name and description, case-insensitive.
async fn search_vendors(
    State(s): State<Arc<AppState>>,
    Query(q): Query<SearchQuery>,
) -> Json<ApiResponse<Vec<oxygenrouter_core::Vendor>>> {
    let needle = q.search.unwrap_or_default().trim().to_lowercase();
    Json(match s.db.list_vendors() {
        Ok(rows) => {
            if needle.is_empty() {
                return Json(ApiResponse::ok(rows));
            }
            let matches = rows
                .into_iter()
                .filter(|v| {
                    v.name.to_lowercase().contains(&needle)
                        || v.description.to_lowercase().contains(&needle)
                })
                .collect();
            ApiResponse::ok(matches)
        }
        Err(error) => ApiResponse::err(error.to_string()),
    })
}

#[derive(serde::Deserialize)]
struct SearchQuery {
    search: Option<String>,
}

async fn get_vendor(
    State(s): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Json<ApiResponse<oxygenrouter_core::Vendor>> {
    match s.db.get_vendor(&id) {
        Ok(Some(vendor)) => Json(ApiResponse::ok(vendor)),
        Ok(None) => Json(ApiResponse::err("vendor not found")),
        Err(error) => Json(ApiResponse::err(error.to_string())),
    }
}

async fn create_vendor(
    State(s): State<Arc<AppState>>,
    Json(mut input): Json<oxygenrouter_core::Vendor>,
) -> Json<ApiResponse<oxygenrouter_core::Vendor>> {
    if input.name.trim().is_empty() {
        return Json(ApiResponse::err("vendor name is required"));
    }
    // The name is unique, so a duplicate is refused with a readable message
    // rather than surfacing a raw SQLite constraint error.
    match s.db.find_vendor_by_name(&input.name) {
        Ok(Some(_)) => return Json(ApiResponse::err("a vendor with that name already exists")),
        Err(error) => return Json(ApiResponse::err(error.to_string())),
        Ok(None) => {}
    }
    if input.id.is_empty() {
        input.id = uuid::Uuid::new_v4().to_string();
    }
    let now = Utc::now();
    input.created_at = now;
    input.updated_at = now;
    input.model_count = 0;
    Json(match s.db.upsert_vendor(&input) {
        Ok(()) => ApiResponse::ok(input),
        Err(error) => ApiResponse::err(error.to_string()),
    })
}

async fn update_vendor(
    State(s): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(mut input): Json<oxygenrouter_core::Vendor>,
) -> Json<ApiResponse<oxygenrouter_core::Vendor>> {
    if input.name.trim().is_empty() {
        return Json(ApiResponse::err("vendor name is required"));
    }
    // A rename must not collide with a different vendor's name; matching our own
    // row is fine, which is why the check excludes the id being updated.
    match s.db.find_vendor_by_name(&input.name) {
        Ok(Some(existing)) if existing.id != id => {
            return Json(ApiResponse::err("a vendor with that name already exists"))
        }
        Err(error) => return Json(ApiResponse::err(error.to_string())),
        _ => {}
    }
    input.id = id;
    input.updated_at = Utc::now();
    input.model_count = 0;
    Json(match s.db.upsert_vendor(&input) {
        Ok(()) => ApiResponse::ok(input),
        Err(error) => ApiResponse::err(error.to_string()),
    })
}

async fn delete_vendor(
    State(s): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Json<ApiResponse<&'static str>> {
    Json(match s.db.delete_vendor(&id) {
        Ok(()) => ApiResponse::ok("deleted"),
        Err(error) => ApiResponse::err(error.to_string()),
    })
}

/// Model names discovered on channels that have no metadata row yet.
async fn missing_model_metadata(
    State(s): State<Arc<AppState>>,
) -> Json<ApiResponse<Vec<String>>> {
    let known = match s.db.list_model_metadata() {
        Ok(rows) => rows.into_iter().map(|row| row.model_name).collect::<std::collections::BTreeSet<_>>(),
        Err(error) => return Json(ApiResponse::err(error.to_string())),
    };
    match s.db.distinct_channel_models() {
        Ok(names) => {
            let missing = names.into_iter().filter(|name| !known.contains(name)).collect::<Vec<_>>();
            Json(ApiResponse::ok(missing))
        }
        Err(error) => Json(ApiResponse::err(error.to_string())),
    }
}

/// Create metadata rows for every channel model that lacks one.
async fn sync_model_metadata(
    State(s): State<Arc<AppState>>,
) -> Json<ApiResponse<usize>> {
    let known = match s.db.list_model_metadata() {
        Ok(rows) => rows.into_iter().map(|row| row.model_name).collect::<std::collections::BTreeSet<_>>(),
        Err(error) => return Json(ApiResponse::err(error.to_string())),
    };
    let names = match s.db.distinct_channel_models() {
        Ok(names) => names,
        Err(error) => return Json(ApiResponse::err(error.to_string())),
    };
    let mut created = 0usize;
    for name in names {
        if known.contains(&name) {
            continue;
        }
        let now = Utc::now();
        let metadata = oxygenrouter_core::ModelMetadata {
            id: uuid::Uuid::new_v4().to_string(),
            model_name: name,
            description: String::new(),
            icon: String::new(),
            tags: String::new(),
            vendor: String::new(),
            endpoints: Vec::new(),
            name_rule: 0,
            status: 1,
            sync_official: 1,
            created_at: now,
            updated_at: now,
        };
        if s.db.upsert_model_metadata(&metadata).is_ok() {
            created += 1;
        }
    }
    Json(ApiResponse::ok(created))
}

// ── Log statistics ───────────────────────────────────────────────────────

#[derive(serde::Serialize)]
struct LogStats {
    total: i64,
    success: i64,
    errors: i64,
    success_rate: f64,
    avg_latency_ms: f64,
    total_tokens: i64,
    model_breakdown: Vec<DashboardBreakdown>,
    channel_breakdown: Vec<DashboardBreakdown>,
}

#[derive(serde::Deserialize)]
struct LogSummaryQuery {
    time_range: Option<String>,
    model: Option<String>,
    channel_id: Option<String>,
    api_key_id: Option<String>,
    status: Option<String>,
}

async fn log_stats(
    State(s): State<Arc<AppState>>,
    Query(q): Query<LogSummaryQuery>,
) -> Json<ApiResponse<LogStats>> {
    let time_range = q.time_range.as_deref().unwrap_or("24h");
    let filters = oxygenrouter_core::AnalyticsFilters {
        model: q.model,
        channel_id: q.channel_id,
        api_key_id: q.api_key_id,
        status: q.status,
    };
    let snap = match s.db.get_dashboard_snapshot(time_range, &filters) {
        Ok(sn) => sn,
        Err(e) => return Json(ApiResponse::err(e.to_string())),
    };
    Json(ApiResponse::ok(LogStats {
        total: snap.total_requests,
        success: snap.successful_requests,
        errors: snap.failed_requests,
        success_rate: snap.success_rate,
        avg_latency_ms: snap.average_latency_ms,
        total_tokens: snap.total_tokens,
        model_breakdown: snap.model_breakdown,
        channel_breakdown: snap.channel_breakdown,
    }))
}

// ── API Keys ───────────────────────────────────────────────────────────────

async fn list_keys(State(s): State<Arc<AppState>>) -> Json<ApiResponse<Vec<ApiKey>>> {
    Json(match s.db.list_api_keys() {
        Ok(k) => ApiResponse::ok(k),
        Err(e) => ApiResponse::err(e.to_string()),
    })
}

async fn create_key(
    State(s): State<Arc<AppState>>,
    Json(k): Json<ApiKey>,
) -> Json<ApiResponse<ApiKey>> {
    let now = chrono::Utc::now();
    let k = ApiKey {
        id: uuid::Uuid::new_v4().to_string(),
        key: k.key,
        name: k.name,
        priority: k.priority,
        enabled: k.enabled,
        created_at: now,
        expires_at: k.expires_at,
        quota_micros: k.quota_micros,
        used_micros: 0,
        allowed_models: k.allowed_models,
        ip_allowlist: k.ip_allowlist,
        group_name: k.group_name,
        cross_group_retry: k.cross_group_retry,
        user_id: k.user_id,
    };
    Json(match s.db.upsert_api_key(&k) {
        Ok(()) => ApiResponse::ok(k),
        Err(e) => ApiResponse::err(e.to_string()),
    })
}

async fn delete_key(
    State(s): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Json<ApiResponse<&'static str>> {
    Json(match s.db.delete_api_key(&id) {
        Ok(()) => ApiResponse::ok("deleted"),
        Err(e) => ApiResponse::err(e.to_string()),
    })
}

async fn keys_usage(State(s): State<Arc<AppState>>) -> Json<ApiResponse<Vec<ApiKeyUsage>>> {
    Json(match s.db.api_key_usage() {
        Ok(u) => ApiResponse::ok(u),
        Err(e) => ApiResponse::err(e.to_string()),
    })
}

#[derive(Deserialize)]
struct KeysQuery {
    page: Option<i64>,
    page_size: Option<i64>,
    search: Option<String>,
}

async fn query_keys(
    State(s): State<Arc<AppState>>,
    Query(q): Query<KeysQuery>,
) -> Response {
    let page = q.page.unwrap_or(1).max(1);
    let page_size = q.page_size.unwrap_or(20).clamp(1, 200);
    match s.db.list_api_keys_paged(page, page_size, q.search.as_deref()) {
        Ok((items, total)) => Json(PaginatedResponse::new(items, total, page, page_size)).into_response(),
        Err(error) => db_error::<Vec<ApiKey>>(error),
    }
}

// ── Model Maps ────────────────────────────────────────────────────────────

async fn list_model_maps(State(s): State<Arc<AppState>>) -> Json<ApiResponse<Vec<ModelMap>>> {
    Json(match s.db.list_model_maps() {
        Ok(m) => ApiResponse::ok(m),
        Err(e) => ApiResponse::err(e.to_string()),
    })
}

async fn create_model_map(
    State(s): State<Arc<AppState>>,
    Json(m): Json<ModelMap>,
) -> Json<ApiResponse<ModelMap>> {
    let m = ModelMap {
        id: uuid::Uuid::new_v4().to_string(),
        channel_id: m.channel_id,
        pattern: m.pattern,
        target_model: m.target_model,
        enabled: m.enabled,
        created_at: chrono::Utc::now(),
    };
    let result = match s.db.upsert_model_map(&m) {
        Ok(()) => ApiResponse::ok(m),
        Err(e) => ApiResponse::err(e.to_string()),
    };
    s.reload_scheduler_maps().await;
    Json(result)
}

async fn delete_model_map(
    State(s): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Json<ApiResponse<&'static str>> {
    let result = match s.db.delete_model_map(&id) {
        Ok(()) => ApiResponse::ok("deleted"),
        Err(e) => ApiResponse::err(e.to_string()),
    };
    s.reload_scheduler_maps().await;
    Json(result)
}

// ── Route Rules ──────────────────────────────────────────────────────────

async fn list_rules(State(s): State<Arc<AppState>>) -> Json<ApiResponse<Vec<RouteRule>>> {
    Json(match s.db.list_route_rules() {
        Ok(r) => ApiResponse::ok(r),
        Err(e) => ApiResponse::err(e.to_string()),
    })
}

#[derive(Deserialize)]
struct CreateRuleInput {
    name: String,
    rule_type: RouteType,
    priority: i32,
    config: serde_json::Value,
}

async fn create_rule(
    State(s): State<Arc<AppState>>,
    Json(input): Json<CreateRuleInput>,
) -> Json<ApiResponse<RouteRule>> {
    let r = RouteRule {
        id: uuid::Uuid::new_v4().to_string(),
        name: input.name,
        rule_type: input.rule_type,
        priority: input.priority,
        enabled: true,
        config: input.config,
        created_at: chrono::Utc::now(),
    };
    Json(match s.db.upsert_route_rule(&r) {
        Ok(()) => ApiResponse::ok(r),
        Err(e) => ApiResponse::err(e.to_string()),
    })
}

async fn delete_rule(
    State(s): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Json<ApiResponse<&'static str>> {
    Json(match s.db.delete_route_rule(&id) {
        Ok(()) => ApiResponse::ok("deleted"),
        Err(e) => ApiResponse::err(e.to_string()),
    })
}

// ── Logs ──────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct LogsQuery {
    limit: Option<i64>,
    page: Option<i64>,
    page_size: Option<i64>,
    start: Option<String>,
    end: Option<String>,
    model: Option<String>,
    channel_id: Option<String>,
    api_key_id: Option<String>,
    status: Option<String>,
    search: Option<String>,
    paginated: Option<bool>,
}

async fn list_logs(
    State(s): State<Arc<AppState>>,
    Query(q): Query<LogsQuery>,
) -> Response {
    if q.paginated.unwrap_or(false) {
        let page = q.page.unwrap_or(1).max(1);
        let page_size = q.page_size.unwrap_or(20).clamp(1, 200);
        return match s.db.query_request_logs(
            page,
            page_size,
            q.start.as_deref(),
            q.end.as_deref(),
            q.model.as_deref(),
            q.channel_id.as_deref(),
            q.api_key_id.as_deref(),
            q.status.as_deref(),
            q.search.as_deref(),
        ) {
            Ok((items, total)) => Json(PaginatedResponse::new(items, total, page, page_size)).into_response(),
            Err(e) => db_error::<Vec<RequestLog>>(e),
        };
    }
    let limit = q.limit.unwrap_or(100).min(1000);
    Json(match s.db.list_request_logs(limit) {
        Ok(logs) => ApiResponse::ok(logs),
        Err(e) => ApiResponse::err(e.to_string()),
    })
    .into_response()
}

async fn clear_logs(State(s): State<Arc<AppState>>) -> Json<ApiResponse<usize>> {
    Json(match s.db.clear_request_logs() {
        Ok(n) => ApiResponse::ok(n),
        Err(e) => ApiResponse::err(e.to_string()),
    })
}

/// SSE endpoint that streams new request log rows as they're added.
///
/// Each `insert_request_log` call from the proxy broadcasts the new log via
/// `AppState::log_broadcast`. Subscribers receive those events as SSE messages.
/// As a fallback for missed broadcasts, the endpoint also polls the database
/// every second for rows whose `created_at` is newer than the last seen one.
async fn stream_logs(State(s): State<Arc<AppState>>) -> Response {
    let rx = s.log_broadcast.subscribe();
    let db = s.db.clone();
    let initial_cutoff = Utc::now();

    let stream = async_stream::stream! {
        // Send a comment immediately so the client knows the connection is alive.
        yield Ok::<_, std::convert::Infallible>(": connected\n\n".to_string());

        let mut ticker = interval(Duration::from_secs(1));
        let mut rx = rx;
        let mut last_seen: chrono::DateTime<Utc> = initial_cutoff;
        // Skip any broadcasts that already fired before this subscriber attached.
        loop {
            tokio::select! {
                msg = rx.recv() => {
                    match msg {
                        Ok(log) => {
                            last_seen = std::cmp::max(last_seen, log.created_at);
                            if let Ok(json) = serde_json::to_string(&log) {
                                yield Ok(format!("data: {json}\n\n"));
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => {
                            // Subscriber fell behind. Catch up via DB poll.
                            match db.list_request_logs_since(last_seen, 200) {
                                Ok(rows) => {
                                    for row in rows {
                                        last_seen = std::cmp::max(last_seen, row.created_at);
                                        if let Ok(json) = serde_json::to_string(&row) {
                                            yield Ok(format!("data: {json}\n\n"));
                                        }
                                    }
                                }
                                Err(_) => {}
                            }
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
                _ = ticker.tick() => {
                    match db.list_request_logs_since(last_seen, 200) {
                        Ok(rows) => {
                            for row in rows {
                                last_seen = std::cmp::max(last_seen, row.created_at);
                                if let Ok(json) = serde_json::to_string(&row) {
                                    yield Ok(format!("data: {json}\n\n"));
                                }
                            }
                        }
                        Err(_) => {}
                    }
                }
            }
        }
    };

    let body = axum::body::Body::from_stream(stream.map(|r| match r {
        Ok(s) => Ok::<_, std::io::Error>(axum::body::Bytes::from(s)),
        Err(_) => Ok(axum::body::Bytes::new()),
    }));

    Response::builder()
        .status(axum::http::StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache")
        .header("x-accel-buffering", "no")
        .body(body)
        .unwrap_or_else(|_| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "").into_response())
}

// ── Status ────────────────────────────────────────────────────────────────

async fn get_status(
    State(s): State<Arc<AppState>>,
) -> Json<ApiResponse<oxygenrouter_core::SystemStatus>> {
    Json(match s.db.get_system_status(s.start_time) {
        Ok(st) => ApiResponse::ok(st),
        Err(e) => ApiResponse::err(e.to_string()),
    })
}

async fn get_dashboard(
    State(s): State<Arc<AppState>>,
    Query(q): Query<DashboardQuery>,
) -> Json<ApiResponse<oxygenrouter_core::DashboardSnapshot>> {
    let time_range = match q.time_range.as_deref() {
        Some("1h") => "1h",
        Some("24h") => "24h",
        Some("7d") => "7d",
        Some("30d") => "30d",
        _ => "24h",
    };
    let filters = oxygenrouter_core::AnalyticsFilters {
        model: q.model,
        channel_id: q.channel_id,
        api_key_id: q.api_key_id,
        status: q.status,
    };
    Json(match s.db.get_dashboard_snapshot(time_range, &filters) {
        Ok(snapshot) => ApiResponse::ok(snapshot),
        Err(e) => ApiResponse::err(e.to_string()),
    })
}

#[derive(serde::Deserialize)]
struct DashboardQuery {
    time_range: Option<String>,
    model: Option<String>,
    channel_id: Option<String>,
    api_key_id: Option<String>,
    status: Option<String>,
}

async fn get_analytics_flow(
    State(s): State<Arc<AppState>>,
    Query(q): Query<DashboardQuery>,
) -> Json<ApiResponse<oxygenrouter_core::AnalyticsFlow>> {
    let time_range = match q.time_range.as_deref() {
        Some("1h") => "1h",
        Some("24h") => "24h",
        Some("7d") => "7d",
        Some("30d") => "30d",
        _ => "24h",
    };
    let filters = oxygenrouter_core::AnalyticsFilters {
        model: q.model,
        channel_id: q.channel_id,
        api_key_id: q.api_key_id,
        status: q.status,
    };
    Json(match s.db.get_analytics_flow(time_range, &filters) {
        Ok(flow) => ApiResponse::ok(flow),
        Err(e) => ApiResponse::err(e.to_string()),
    })
}

// ── Settings ──────────────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct UpdateSettingsInput {
    pub listen_host: Option<String>,
    pub listen_port: Option<u16>,
    pub local_api_token: Option<String>,
    pub open_browser_on_start: Option<bool>,
    pub log_level: Option<String>,
    pub max_retries: Option<i32>,
    pub retry_delay_ms: Option<i64>,
    pub retry_backoff: Option<String>,
    pub upstream_timeout_ms: Option<u64>,
    pub user_agent: Option<String>,
    pub max_concurrent_requests: Option<i32>,
    pub request_log_retention_days: Option<i32>,
    pub theme: Option<String>,
    pub language: Option<String>,
}

async fn get_settings() -> Json<ApiResponse<oxygenrouter_core::AppSettings>> {
    let cfg = oxygenrouter_core::APP_CONFIG.read();
    Json(ApiResponse::ok((*cfg).clone()))
}

// ── Option system (typed key-value settings, NewAPI parity) ───────────────

#[derive(Serialize)]
struct OptionEntry {
    key: &'static str,
    section: oxygenrouter_core::OptionSection,
    kind: oxygenrouter_core::OptionKind,
    value: String,
    default: &'static str,
    description: &'static str,
    secret: bool,
}

async fn get_options(State(s): State<Arc<AppState>>) -> Json<ApiResponse<Vec<OptionEntry>>> {
    let stored = s.db.list_settings().unwrap_or_default();
    let entries = oxygenrouter_core::OPTION_SCHEMA
        .iter()
        .map(|schema| {
            let value = stored.get(schema.key).cloned().unwrap_or_else(|| schema.default.to_string());
            OptionEntry {
                key: schema.key,
                section: schema.section,
                kind: schema.kind,
                value: if schema.secret && !value.is_empty() {
                    // Never echo secrets in full; expose a masked preview.
                    format!("{}…", &value[..value.len().min(6)])
                } else {
                    value
                },
                default: schema.default,
                description: schema.description,
                secret: schema.secret,
            }
        })
        .collect::<Vec<_>>();
    Json(ApiResponse::ok(entries))
}

#[derive(serde::Deserialize)]
struct UpdateOptionInput {
    key: String,
    value: String,
}

async fn update_option(
    State(s): State<Arc<AppState>>,
    Json(input): Json<UpdateOptionInput>,
) -> Json<ApiResponse<&'static str>> {
    let Some(schema) = oxygenrouter_core::find_schema(&input.key) else {
        return Json(ApiResponse::err(format!("unknown option: {}", input.key)));
    };
    if let Err(error) = oxygenrouter_core::validate_option(schema.kind, &input.value) {
        return Json(ApiResponse::err(error));
    }
    Json(match s.db.set_setting(schema.key, &input.value) {
        Ok(()) => ApiResponse::ok("updated"),
        Err(error) => ApiResponse::err(error.to_string()),
    })
}

async fn update_settings(
    State(s): State<Arc<AppState>>,
    Json(input): Json<UpdateSettingsInput>,
) -> Json<ApiResponse<oxygenrouter_core::AppSettings>> {
    {
        let mut cfg = oxygenrouter_core::APP_CONFIG.write();
        if let Some(v) = input.listen_host {
            cfg.listen_host = v;
        }
        if let Some(v) = input.listen_port {
            cfg.listen_port = v;
        }
        if let Some(v) = input.local_api_token.as_ref() {
            cfg.local_api_token = v.clone();
        }
        if let Some(v) = input.open_browser_on_start {
            cfg.open_browser_on_start = v;
        }
        if let Some(v) = input.log_level {
            cfg.log_level = v;
        }
        if let Some(v) = input.max_retries {
            cfg.max_retries = v;
        }
        if let Some(v) = input.retry_delay_ms {
            cfg.retry_delay_ms = v;
        }
        if let Some(v) = input.retry_backoff {
            cfg.retry_backoff = v;
        }
        if let Some(v) = input.upstream_timeout_ms {
            cfg.upstream_timeout_ms = v;
        }
        if let Some(v) = input.user_agent {
            cfg.user_agent = v;
        }
        if let Some(v) = input.max_concurrent_requests {
            cfg.max_concurrent_requests = v;
        }
        if let Some(v) = input.request_log_retention_days {
            cfg.request_log_retention_days = v;
        }
        if let Some(v) = input.theme.as_ref().cloned() {
            cfg.theme = v;
        }
        if let Some(v) = input.language.as_ref().cloned() {
            cfg.language = v;
        }
    }
    let cfg = oxygenrouter_core::APP_CONFIG.read();
    if let Some(v) = input.local_api_token.as_ref() {
        let _ = s.db.set_setting("local_api_token", v);
    }
    if let Some(v) = input.theme.as_ref() {
        let _ = s.db.set_setting("theme", v);
    }
    if let Some(v) = input.language.as_ref() {
        let _ = s.db.set_setting("language", v);
    }
    let _ = oxygenrouter_core::save_config(s.config_path.as_path());
    Json(ApiResponse::ok((*cfg).clone()))
}

// ── System Info ─────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct SystemInfo {
    version: String,
    uptime_seconds: u64,
    db_size_bytes: u64,
    log_count: i64,
    channel_count: i64,
    enabled_channel_count: i64,
    key_count: i64,
    model_map_count: i64,
    rule_count: i64,
    local_api_token: String,
    listen_host: String,
    listen_port: u16,
    max_concurrent_requests: u32,
    upstream_timeout_ms: u64,
    max_retries: u32,
    platform: String,
    arch: String,
    rustc_version: String,
    build_profile: String,
    started_at: String,
}

async fn system_info(State(s): State<Arc<AppState>>) -> Json<ApiResponse<SystemInfo>> {
    let cfg = oxygenrouter_core::APP_CONFIG.read();
    let log_count = s.db.count_logs().unwrap_or(0);
    let channel_count = s.db.count_channels().unwrap_or(0);
    let enabled_channel_count = s.db.count_enabled_channels().unwrap_or(0);
    let key_count = s.db.count_keys().unwrap_or(0);
    let model_map_count = s.db.count_model_maps().unwrap_or(0);
    let rule_count = s.db.count_rules().unwrap_or(0);
    let db_size_bytes = std::fs::metadata(&s.db_path).map(|m| m.len()).unwrap_or(0);
    let uptime_seconds = Utc::now()
        .signed_duration_since(s.started_at)
        .num_seconds()
        .max(0) as u64;
    Json(ApiResponse::ok(SystemInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        uptime_seconds,
        db_size_bytes,
        log_count,
        channel_count,
        enabled_channel_count,
        key_count,
        model_map_count,
        rule_count,
        local_api_token: cfg.local_api_token.clone(),
        listen_host: cfg.listen_host.clone(),
        listen_port: cfg.listen_port,
        max_concurrent_requests: cfg.max_concurrent_requests as u32,
        upstream_timeout_ms: cfg.upstream_timeout_ms,
        max_retries: cfg.max_retries as u32,
        platform: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        rustc_version: "rustc 1.95 (MSVC)".to_string(),
        build_profile: if cfg!(debug_assertions) {
            "debug".to_string()
        } else {
            "release".to_string()
        },
        started_at: s.started_at.to_rfc3339(),
    }))
}

// ── Backup ─────────────────────────────────────────────────────────────────

async fn create_backup(State(s): State<Arc<AppState>>) -> Response {
    // Use SQLite's VACUUM INTO for safe backup
    let backup_path =
        std::env::temp_dir().join(format!("oxygenrouter-backup-{}.db", Utc::now().timestamp()));
    let result = s.db.backup_to(&backup_path);
    match result {
        Ok(()) => {
            let bytes = match std::fs::read(&backup_path) {
                Ok(b) => b,
                Err(e) => {
                    return (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
                        .into_response()
                }
            };
            let _ = std::fs::remove_file(&backup_path);
            let filename = format!(
                "oxygenrouter-backup-{}.db",
                Utc::now().format("%Y%m%d-%H%M%S")
            );
            (
                [(header::CONTENT_TYPE, "application/octet-stream")],
                [(
                    header::CONTENT_DISPOSITION,
                    format!("attachment; filename=\"{}\"", filename),
                )],
                bytes,
            )
                .into_response()
        }
        Err(e) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}
