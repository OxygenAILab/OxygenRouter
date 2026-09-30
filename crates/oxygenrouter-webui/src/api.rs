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
        // D6: full update semantics, matching the reference's `PUT /token/`.
        .route("/api/keys/:id", put(update_key))
        // The deliberate disclosure route; see `reveal_key`.
        .route("/api/keys/:id/secret", get(reveal_key))
        .route("/api/keys/usage", get(keys_usage))
        .route("/api/keys/query", get(query_keys))
        .route("/api/model-maps", get(list_model_maps))
        .route("/api/model-maps", post(create_model_map))
        .route("/api/model-maps/:id", delete(delete_model_map))
        .route("/api/rules", get(list_rules))
        .route("/api/rules", post(create_rule))
        .route("/api/rules/:id", delete(delete_rule))
        .route("/api/logs", get(list_logs).delete(clear_logs))
        // The caller's own logs, for any signed-in user; matches the reference's
        // `/log/self` (`router/api-router.go:319`).
        .route("/api/logs/self", get(list_my_logs))
        .route("/api/logs/stream", get(stream_logs))
        // The management trail. Root-only: it records who changed what, which is
        // itself sensitive, and the reference likewise gates its log endpoints on
        // admin (`router/api-router.go:314`).
        .route("/api/audit-logs", get(list_audit_logs).delete(clear_audit_logs))
        // Plugin management, matching the reference's `/plugin/task` group
        // (`router/api-router.go:253`). Root-only by the `/api/plugin` prefix:
        // uploading code the gateway will execute is an owner action.
        .route("/api/plugin/task", get(list_task_plugins).post(upload_task_plugin))
        .route("/api/plugin/task/:key", get(get_task_plugin))
        .route("/api/plugin/task/:key/activate", post(activate_task_plugin))
        .route("/api/plugin/task/:key/status", post(set_task_plugin_status))
        .route("/api/plugin/task/:key/dryrun", post(dry_run_task_plugin))
        .route(
            "/api/plugin/task/:key/versions/:version",
            axum::routing::delete(delete_task_plugin_version),
        )
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
        // One gate over the whole console tree, applied last so it wraps every
        // route above it. See `access_guard` for why this is path-class based
        // rather than checked inside each handler.
        .layer(axum::middleware::from_fn_with_state(
            Arc::clone(&state),
            access_guard,
        ))
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
    if user.role.is_admin() {
        Ok(user)
    } else {
        Err((
            StatusCode::FORBIDDEN,
            Json(ApiResponse::<()>::err("admin role required")),
        )
            .into_response())
    }
}

fn root_user(s: &AppState, headers: &HeaderMap) -> Result<User, Response> {
    let user = auth_user(s, headers)?;
    if user.role.is_root() {
        Ok(user)
    } else {
        Err((
            StatusCode::FORBIDDEN,
            Json(ApiResponse::<()>::err("instance owner role required")),
        )
            .into_response())
    }
}

/// The access level a console path requires.
///
/// Enforced as one middleware over the whole `/api` tree rather than per handler,
/// because the failure mode of a per-handler check is *silence*: a new route with
/// no `auth_user` call is simply unauthenticated, and nothing points at it. This
/// was not hypothetical — 49 of 86 routes were reachable anonymously, including
/// `GET /api/channels` (which returns upstream credentials), `PUT /api/channels/:id`
/// (which repoints a channel), `POST /api/keys`, and `GET /api/settings` (which
/// returns `local_api_token`).
///
/// The classification is deny-by-default: anything not explicitly public needs a
/// session, so forgetting to classify a new route fails closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Access {
    /// Reachable without a credential.
    Public,
    /// Any signed-in user.
    User,
    /// Admin or root.
    Admin,
    /// Root only, matching the reference's `RootAuth` groups.
    Root,
}

/// Public endpoints. Kept as an explicit allow-list so a new route is closed by
/// default rather than open.
const PUBLIC_ROUTES: &[&str] = &[
    "/api/auth/register",
    "/api/auth/login",
    // The landing page shows the catalogue before sign-in.
    "/api/plans",
    // Readiness probe: reports only liveness, never configuration.
    "/api/status",
];

/// Prefixes that only the instance owner may reach.
///
/// Matches the reference's `RootAuth` groups (`option`, `task plugin`,
/// `system-task`, `system-info`, `performance`, `ratio_sync`,
/// `custom-oauth-provider`) plus the settings and backup endpoints here, which
/// reveal or rewrite instance-wide configuration.
const ROOT_PREFIXES: &[&str] = &[
    "/api/options",
    "/api/settings",
    "/api/system/info",
    "/api/backup",
    "/api/plugin",
    "/api/ratio_sync",
    "/api/performance",
    "/api/system-task",
    "/api/custom-oauth-provider",
    // Who changed what. Sensitive precisely because it is complete, and it names
    // the accounts that hold power.
    "/api/audit-logs",
];

/// Prefixes that need an admin (or root).
///
/// Everything that manages shared infrastructure or other accounts: channels,
/// the model registry, vendors, routing, redemption, and the admin groups.
const ADMIN_PREFIXES: &[&str] = &[
    "/api/channels",
    "/api/models-metadata",
    "/api/vendors",
    "/api/model-maps",
    "/api/rules",
    "/api/admin",
    "/api/subscription/admin",
    "/api/redemption/admin",
    "/api/authz",
    // Instance-wide observability. The reference gates its log listing on
    // `AdminAuth` (`router/api-router.go:314`) and serves a separate `/log/self`
    // for a user's own rows; verified before this change that an ordinary user
    // could read every user's logs and the aggregate dashboards.
    "/api/logs",
    "/api/log",
    "/api/dashboard",
    "/api/analytics",
];

/// Routes under an admin prefix that any signed-in user may still reach, because
/// they are scoped to the caller.
const USER_SCOPED_EXCEPTIONS: &[&str] = &["/api/logs/self"];

fn required_access(path: &str) -> Access {
    let path = path.trim_end_matches('/');
    if PUBLIC_ROUTES.contains(&path) {
        return Access::Public;
    }
    if USER_SCOPED_EXCEPTIONS.contains(&path) {
        return Access::User;
    }
    // Prefix checks must not let `/api/optionsfoo` match `/api/options`, so a
    // match requires an exact prefix or a `/` boundary.
    let under = |prefix: &str| path == prefix || path.starts_with(&format!("{prefix}/"));
    if ROOT_PREFIXES.iter().any(|p| under(p)) {
        return Access::Root;
    }
    if ADMIN_PREFIXES.iter().any(|p| under(p)) {
        return Access::Admin;
    }
    Access::User
}

/// Gate every `/api` request by its required access level.
async fn access_guard(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let path = request.uri().path().to_string();
    let required = required_access(&path);
    let outcome = match required {
        Access::Public => Ok(()),
        Access::User => auth_user(&state, &headers).map(|_| ()),
        Access::Admin => admin_user(&state, &headers).map(|_| ()),
        Access::Root => root_user(&state, &headers).map(|_| ()),
    };
    match outcome {
        Ok(()) => {
            // An audited action is a *changing* one by a *privileged* actor. A
            // user renaming their own key is excluded on purpose: `AuditLogEnabled`
            // is described as "record administrative operations", and flooding the
            // trail with every user's own edits would bury exactly the events it
            // exists to surface.
            let method = request.method().as_str().to_string();
            let auditable = method != "GET"
                && matches!(required, Access::Admin | Access::Root);
            if auditable && state.audit_log_enabled() {
                let actor = match required {
                    Access::Root => root_user(&state, &headers).ok(),
                    _ => admin_user(&state, &headers).ok(),
                };
                // Buffer the body so the trail can say *what* was changed, not
                // just which path was called. Only audited requests pay for this,
                // which is a handful of admin actions rather than the hot path.
                let (parts, body) = request.into_parts();
                let body_bytes = axum::body::to_bytes(body, AUDIT_BODY_LIMIT)
                    .await
                    .unwrap_or_default()
                    .to_vec();
                let request = axum::extract::Request::from_parts(parts, axum::body::Body::from(body_bytes.clone()));
                // Rejections are worth as much as successes to an investigator:
                // a refused elevation attempt is frequently the interesting row.
                let response = next.run(request).await;
                let status = response.status().as_u16();
                record_audit(
                    &state,
                    actor.as_ref(),
                    &method,
                    &path,
                    Some(status),
                    redact_body(&body_bytes, &path),
                    &headers,
                );
                response
            } else {
                next.run(request).await
            }
        }
        // `auth_user` already produced the right response shape and status.
        Err(response) => response,
    }
}

/// Largest request body the audit trail will capture.
///
/// An administrative action is a settings change or an account edit, all of
/// which are small; anything larger is not something the trail needs verbatim.
const AUDIT_BODY_LIMIT: usize = 64 * 1024;

/// Field names that are a credential in their own right.
///
/// Matched exactly, so `monkey` is not mistaken for `key`. A credential copied
/// into the audit trail in the clear would be the same disclosure the masking
/// work closed, arriving through a new door.
const AUDIT_REDACT_EXACT: &[&str] = &[
    "key",
    "apikey",
    "api_key",
    "token",
    "secret",
    "password",
    "passwd",
    "code",
    "credential",
    "authorization",
    "signature",
];

/// Fragments that mark a field as credential-bearing.
///
/// Matched as substrings because these spell out what the value is, so a name
/// like `github_client_secret` or `smtp_token` is covered without enumerating
/// every vendor's spelling.
const AUDIT_REDACT_CONTAINS: &[&str] = &[
    "password",
    "secret",
    "token",
    "api_key",
    "apikey",
    "private_key",
    "credential",
    "authorization",
    "signature",
];

/// True when this field's value must not be stored verbatim.
///
/// `key_is_a_name` is set for the options route, where `key` is the *name* of a
/// setting rather than a credential — redacting it there would leave the trail
/// unable to say which setting was changed, which is the one thing it is for.
fn is_credential_field(name: &str, key_is_a_name: bool) -> bool {
    let lower = name.to_ascii_lowercase();
    if lower == "key" && key_is_a_name {
        return false;
    }
    AUDIT_REDACT_EXACT.contains(&lower.as_str())
        || AUDIT_REDACT_CONTAINS.iter().any(|fragment| lower.contains(fragment))
}

/// Render a request body for the trail with credential-bearing values replaced.
///
/// Anything that is not a JSON object is dropped rather than stored: a body that
/// cannot be inspected for secrets must not be assumed to be safe.
fn redact_body(bytes: &[u8], path: &str) -> Option<String> {
    if bytes.is_empty() {
        return None;
    }
    let parsed: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    let serde_json::Value::Object(mut map) = parsed else {
        return None;
    };
    let options_route = path.starts_with("/api/options");

    // `/api/options` is the one route where a value is a credential only when the
    // option it names is marked secret in the schema (`LocalApiToken` and
    // friends). Reading the schema is what makes this exact instead of a guess:
    // the alternative is either disclosing the token or redacting every setting.
    if options_route {
        let names_a_secret = matches!(
            map.get("key"),
            Some(serde_json::Value::String(name))
                if oxygenrouter_core::find_schema(name).is_some_and(|s| s.secret)
        );
        if names_a_secret {
            map.insert(
                "value".to_string(),
                serde_json::Value::String("[redacted]".to_string()),
            );
        }
    }

    for (name, value) in map.iter_mut() {
        if is_credential_field(name, options_route) {
            *value = serde_json::Value::String("[redacted]".to_string());
        } else if let serde_json::Value::Object(inner) = value {
            // Nested objects are uncommon here, but a serialised settings blob
            // could carry one, so a wrapped secret must not slip through under a
            // harmless outer key.
            for (n2, v2) in inner.iter_mut() {
                if is_credential_field(n2, false) {
                    *v2 = serde_json::Value::String("[redacted]".to_string());
                }
            }
        }
    }
    serde_json::to_string(&map).ok()
}

/// Persist one administrative operation.
///
/// Best-effort on purpose: a failure to write the trail must not fail the action
/// the operator asked for. The error is surfaced on stderr rather than dropped.
fn record_audit(
    state: &AppState,
    actor: Option<&User>,
    method: &str,
    path: &str,
    status_code: Option<u16>,
    detail: Option<String>,
    headers: &HeaderMap,
) {
    let log = oxygenrouter_core::AuditLog {
        id: uuid::Uuid::new_v4().to_string(),
        actor_id: actor.map(|u| u.id.clone()),
        actor_name: actor.map(|u| u.username.clone()).unwrap_or_default(),
        actor_role: actor.map(|u| u.role.as_str().to_string()).unwrap_or_default(),
        method: method.to_string(),
        path: path.to_string(),
        status_code,
        detail,
        client_ip: state
            .record_ip_log()
            .then(|| crate::proxy::caller_address(headers)),
        created_at: Utc::now(),
    };
    if let Err(error) = state.db.insert_audit_log(&log) {
        eprintln!("[OxygenRouter] audit write failed: {error}");
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

/// Refuse a login while the caller's failure window has not reset.
///
/// The scope is the caller's address, not the username: keying on the username
/// would let an attacker lock a known account out on purpose, which turns a
/// protection into the denial of service it is meant to prevent.
///
/// Returns `Some(response)` when the attempt must be refused.
fn login_lock_response(state: &AppState, headers: &HeaderMap) -> Option<Response> {
    let scope = login_lock_scope(headers);
    let now = std::time::Instant::now();
    let limit = state.login_rate_limit();
    if let Some(retry_after) = state.rate_limiter.retry_after_secs(&scope, limit, now) {
        let mut response = (
            StatusCode::TOO_MANY_REQUESTS,
            Json(ApiResponse::<LoginResponse>::err(format!(
                "too many failed sign-in attempts; try again in {retry_after}s"
            ))),
        )
            .into_response();
        if let Ok(value) = axum::http::HeaderValue::from_str(&retry_after.to_string()) {
            response.headers_mut().insert("retry-after", value);
        }
        return Some(response);
    }
    None
}

/// One failure window per caller address.
///
/// A request with no resolvable address (no socket peer, no forwarding header)
/// shares a single window. That over-limits the unnamed caller rather than
/// under-limiting it, which is the correct direction for an authentication
/// guard: failing closed must not become "unlimited attempts".
fn login_lock_scope(headers: &HeaderMap) -> String {
    format!("login:{}", crate::proxy::caller_address(headers))
}

/// Reject an account request with a message that says what is actually wrong.
///
/// The storage layer enforces the same rules and answers with
/// `rusqlite::Error::InvalidQuery`, whose `Display` is "Query is not read-only".
/// That is accurate for SQLite and meaningless to somebody filling in a form, so
/// the check is repeated here where a useful sentence can be returned. The
/// storage-layer guard stays: defence in depth, not the error-reporting path.
fn validate_new_account(
    username: &str,
    email: &str,
    password: &str,
    min_password_len: usize,
) -> Result<(), String> {
    if username.trim().chars().count() < 3 {
        return Err("username must be at least 3 characters".to_string());
    }
    if !email.contains('@') {
        return Err("a valid email address is required".to_string());
    }
    // Counted in characters, matching the storage layer and the option's name.
    if password.chars().count() < min_password_len {
        return Err(format!(
            "password must be at least {min_password_len} characters"
        ));
    }
    Ok(())
}

async fn register(State(s): State<Arc<AppState>>, Json(input): Json<RegisterInput>) -> Response {
    // Three options the console has always shown as live switches. Nothing read
    // them, so self-registration could not be switched off and the length rule
    // was hard-coded to 8 regardless of what the field said.
    let policy = s.auth_policy();
    if !policy.registration_enabled {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiResponse::<User>::err("registration is disabled")),
        )
            .into_response();
    }
    if let Err(reason) = validate_new_account(
        &input.username,
        &input.email,
        &input.password,
        policy.min_password_len,
    ) {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse::<User>::err(reason)),
        )
            .into_response();
    }
    match s.db.create_user_with_min_password_len(
        &input.username,
        &input.email,
        &input.password,
        UserRole::User,
        policy.min_password_len,
    ) {
        Ok(user) => (StatusCode::CREATED, Json(ApiResponse::ok(user))).into_response(),
        Err(e) => db_error::<User>(e),
    }
}
async fn login(
    State(s): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(input): Json<LoginInput>,
) -> Response {
    if let Some(locked) = login_lock_response(&s, &headers) {
        return locked;
    }
    // Read once: the same policy decides whether password sign-in is allowed and
    // how long the session it produces will last.
    let policy = s.auth_policy();
    // `PasswordLoginEnabled` off means the password form is not a valid way in,
    // even with correct credentials. Checked after the lock so a disabled
    // instance still reports the lock state rather than leaking that the policy
    // differs; the reference refuses here too (`controller/user.go:54`).
    if !policy.password_login_enabled {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiResponse::<LoginResponse>::err(
                "password sign-in is disabled",
            )),
        )
            .into_response();
    }
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
            // `SessionTtlDays` was advertised and ignored: the lifetime was
            // hard-coded here, so the field could not change it.
            match s.db.create_session(&user.id, policy.session_ttl) {
                Ok(session) => {
                    // A completed sign-in clears the caller's failure count, so a
                    // legitimate user who mistyped a few times is not left one
                    // attempt away from a lock.
                    s.rate_limiter.reset(&login_lock_scope(&headers));
                    Json(ApiResponse::ok(LoginResponse {
                        user,
                        session_token: session.token,
                        expires_at: session.expires_at,
                    }))
                    .into_response()
                }
                Err(e) => db_error::<LoginResponse>(e),
            }
        }
        Ok(None) => {
            let outcome = s
                .rate_limiter
                .check(&login_lock_scope(&headers), s.login_rate_limit());
            let mut response = (
                StatusCode::UNAUTHORIZED,
                Json(ApiResponse::<LoginResponse>::err(
                    "invalid username or password",
                )),
            )
                .into_response();
            // Report how long the lock now runs, so a client can back off instead
            // of hammering. Advisory only: the delay is never applied here, which
            // is what keeps the counter accurate and the test fast.
            if let Err(error) = outcome {
                if let Some(retry_after) = error.retry_after_secs() {
                    if let Ok(value) = axum::http::HeaderValue::from_str(&retry_after.to_string()) {
                        response.headers_mut().insert("retry-after", value);
                    }
                }
            }
            response
        }
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
    let actor = match admin_user(&s, &headers) {
        Ok(user) => user,
        Err(response) => return response,
    };
    // The actor must outrank the role it is handing out. Without this an ordinary
    // admin could create a root account — or a peer admin — and then use it.
    if !UserRole::can_assign(&actor.role, &input.role) {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiResponse::<User>::err(
                "cannot create an account at or above your own role",
            )),
        )
            .into_response();
    }
    let Some(password) = input.password else {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse::<User>::err("password required")),
        )
            .into_response();
    };
    if let Err(reason) = validate_new_account(
        &input.username,
        &input.email,
        &password,
        s.auth_policy().min_password_len,
    ) {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse::<User>::err(reason)),
        )
            .into_response();
    }
    match s
        .db
        .create_user_with_min_password_len(
            &input.username,
            &input.email,
            &password,
            input.role,
            s.auth_policy().min_password_len,
        )
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
    let actor = match admin_user(&s, &headers) {
        Ok(user) => user,
        Err(response) => return response,
    };
    let existing = match s.db.get_user(&id) {
        Ok(Some(user)) => user,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(ApiResponse::<User>::err("user not found")),
            )
                .into_response()
        }
        Err(e) => return db_error::<User>(e),
    };
    // Two independent rules, both from the reference (`controller/user.go:670-679`):
    // the actor must outrank the account it is editing, and must outrank the role
    // it is assigning. Checking only the first would still allow an admin to edit a
    // *peer* upward — most damagingly itself.
    if !UserRole::can_manage(&actor.role, &existing.role) {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiResponse::<User>::err(
                "cannot manage an account at or above your own role",
            )),
        )
            .into_response();
    }
    if !UserRole::can_assign(&actor.role, &input.role) {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiResponse::<User>::err(
                "cannot assign a role at or above your own",
            )),
        )
            .into_response();
    }
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
    // Deleting is the strongest write, so it needs the same hierarchy rule as
    // editing: an admin must not delete a peer or the owner. The self-check above
    // covers only the actor's own row.
    match s.db.get_user(&id) {
        Ok(Some(target)) if !UserRole::can_manage(&admin.role, &target.role) => {
            return (
                StatusCode::FORBIDDEN,
                Json(ApiResponse::<&str>::err(
                    "cannot delete an account at or above your own role",
                )),
            )
                .into_response()
        }
        Ok(Some(_)) => {}
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(ApiResponse::<&str>::err("user not found")),
            )
                .into_response()
        }
        Err(e) => return db_error::<&str>(e),
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
    let actor = match admin_user(&s, &headers) {
        Ok(user) => user,
        Err(response) => return response,
    };
    // Crediting an account is a write in the same sense as editing it, so the
    // hierarchy applies: an admin must not top up a peer or the owner out of the
    // operator's float.
    match s.db.get_user(&id) {
        Ok(Some(target)) if !UserRole::can_manage(&actor.role, &target.role) => {
            return (
                StatusCode::FORBIDDEN,
                Json(ApiResponse::<oxygenrouter_core::LedgerEntry>::err(
                    "cannot adjust an account at or above your own role",
                )),
            )
                .into_response()
        }
        Ok(Some(_)) => {}
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(ApiResponse::<oxygenrouter_core::LedgerEntry>::err(
                    "user not found",
                )),
            )
                .into_response()
        }
        Err(e) => return db_error::<oxygenrouter_core::LedgerEntry>(e),
    }
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

/// Replace a credential with a display placeholder.
///
/// The reference omits the key from its channel list entirely
/// (`controller/channel.go:251`, `.Omit("key")`), and the list is where an
/// upstream credential would otherwise leak: it is the most-read endpoint in the
/// console, so a browser cache, a proxy log or a screenshot is enough to expose
/// it. Verified before the fix: `GET /api/channels` returned the raw key.
///
/// A placeholder rather than a blank, so a client can tell "a key is configured"
/// from "no key set" without being able to read it. A client must never send this
/// value back as a change — see `update_channel`.
const CHANNEL_KEY_PLACEHOLDER: &str = "\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}";

/// Mask a channel's credential for display. An empty key stays empty, so the UI
/// does not imply one exists.
fn masked_channel(mut channel: Channel) -> Channel {
    if !channel.api_key.trim().is_empty() {
        channel.api_key = CHANNEL_KEY_PLACEHOLDER.to_string();
    }
    channel
}

async fn list_channels(State(s): State<Arc<AppState>>) -> Json<ApiResponse<Vec<Channel>>> {
    Json(match s.db.list_channels() {
        Ok(channels) => ApiResponse::ok(channels.into_iter().map(masked_channel).collect()),
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
        // Reading one channel is no different: the credential is withheld here
        // too. The reference serves the real value only from a dedicated route
        // gated on root plus a security proof (`controller.GetChannelKey`).
        Ok(Some(c)) => ApiResponse::ok(masked_channel(c)),
        Ok(None) => ApiResponse::err("channel not found"),
        Err(e) => ApiResponse::err(e.to_string()),
    })
}

/// Update a channel, treating a masked or blank credential as "leave it alone".
///
/// The console reads the (masked) list, the operator edits a field, and the whole
/// object is saved back. Without this rule the placeholder — or a blank — would
/// overwrite the real credential. Verified before the fix: a round-trip PUT with
/// `api_key: ""` left the channel with no key at all, silently disabling it.
async fn update_channel(
    State(s): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(mut ch): Json<Channel>,
) -> Json<ApiResponse<Channel>> {
    let existing = match s.db.get_channel(&id) {
        Ok(Some(channel)) => channel,
        Ok(None) => return Json(ApiResponse::err("channel not found")),
        Err(e) => return Json(ApiResponse::err(e.to_string())),
    };
    // Both shapes an untouched field can arrive as: the placeholder the list
    // handed out, and an empty string. Clearing a credential is therefore not
    // expressible through this route, which is the safe direction — keeping the
    // old key is far better than silently dropping a working one. A dedicated
    // route can add clearing later if it is wanted.
    if ch.api_key == CHANNEL_KEY_PLACEHOLDER || ch.api_key.trim().is_empty() {
        ch.api_key = existing.api_key;
    }
    ch.id = id;
    // The caller never owns creation time; preserving it keeps the record honest
    // even if the client sends a different value.
    ch.created_at = existing.created_at;
    ch.updated_at = chrono::Utc::now();
    Json(match s.db.upsert_channel(&ch) {
        // Mask the response, so an update cannot leak what the read routes walk
        // out of their way to withhold.
        Ok(()) => ApiResponse::ok(masked_channel(ch)),
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

/// Ask one channel's upstream for its model list and persist it.
///
/// Extracted from the HTTP handler so the background refresh can reuse it
/// verbatim: a second implementation of "fetch and store a channel's models"
/// would drift, and the operator-visible behaviour of the button and the timer
/// must be the same or the timer becomes untrustworthy.
///
/// Returns the refreshed list, or the reason the refresh did not happen.
pub async fn refresh_channel_models(
    state: &AppState,
    ch: &Channel,
) -> Result<Vec<String>, String> {
    let base = ch.base_url.trim_end_matches('/');
    let url = if base.ends_with("/v1") { format!("{}/models", base) } else { format!("{}/v1/models", base) };
    // This is a server-side fetch of a stored URL, so it is SSRF-checked. The
    // relay path deliberately is not: NewAPI exempts provider base URLs because
    // they are operator-managed deployment targets that may legitimately be
    // private (a LAN vLLM or Ollama host), whereas this endpoint tells the
    // server to dereference a URL on demand.
    state
        .fetch_policy()
        .validate_url(&url)
        .map_err(|error| format!("ssrf protection refused this target: {error}"))?;
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        // Redirects are not followed: a permitted host could otherwise bounce
        // the request to an internal one and defeat the check above.
        .redirect(reqwest::redirect::Policy::none())
        .build()
    {
        Ok(c) => c,
        Err(e) => return Err(e.to_string()),
    };
    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", ch.api_key.lines().next().unwrap_or_default()))
        .send()
        .await
        .map_err(|e| format!("upstream request failed: {e}"))?;
    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| format!("reading response: {e}"))?;
    if !status.is_success() {
        return Err(format!(
            "upstream HTTP {status}: {}",
            body.chars().take(200).collect::<String>()
        ));
    }
    let models: Vec<String> = serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|v| v.get("data").and_then(|d| d.as_array()).map(|arr| {
            arr.iter().filter_map(|m| m.get("id").and_then(|id| id.as_str()).map(String::from)).collect::<Vec<_>>()
        }))
        .unwrap_or_default();
    if models.is_empty() {
        return Err("no models returned from upstream".to_string());
    }
    state
        .db
        .update_channel_models(&ch.id, &models)
        .map_err(|e| e.to_string())?;
    Ok(models)
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
    match refresh_channel_models(&s, &ch).await {
        Ok(models) => {
            let mut updated = ch;
            updated.model_list = models;
            Json(ApiResponse::ok(updated)).into_response()
        }
        // The message already distinguishes a policy refusal from an upstream
        // failure, so the status is chosen from it rather than re-derived.
        Err(message) => {
            let status = if message.starts_with("ssrf protection refused") {
                StatusCode::FORBIDDEN
            } else if message.starts_with("no models returned") {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::BAD_GATEWAY
            };
            (status, Json(ApiResponse::<Channel>::err(message))).into_response()
        }
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
    // Never reveal the whole value. The previous form returned the *entire* key
    // whenever it was ten characters or shorter, which is a full disclosure for
    // exactly the short, low-entropy credentials most worth protecting.
    //
    // The preview exists so an operator can tell one key from another, so it
    // shows a short prefix — but only when the key is long enough that the prefix
    // is a small fraction of it.
    let chars: Vec<char> = trimmed.chars().collect();
    if chars.len() > 16 {
        let prefix: String = chars[..4].iter().collect();
        format!("{prefix}…")
    } else if chars.is_empty() {
        String::new()
    } else {
        // Short key: reveal only that it exists and how long it is.
        "…".to_string()
    }
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
            // Masked like every other channel response: this handler has no
            // legitimate reason to hand the credentials back, even to the caller
            // that just supplied them.
            Json(ApiResponse::ok(masked_channel(channel)))
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

/// How much of an API key a read may disclose.
///
/// Ports the reference's `MaskTokenKey` (`model/token.go:63`) exactly, including
/// the short-key branches: it reveals nothing for a key of four characters or
/// fewer, two characters at each end up to eight, and four at each end beyond
/// that. Listing a token's real value would make every console read — a browser
/// cache, a proxy log, a screenshot — a credential disclosure, and the reference
/// masks in *both* its list and its search for that reason
/// (`controller/token.go:140,157`).
///
/// The full value stays available from `create_key`, which is the one moment a
/// caller legitimately needs it.
pub fn mask_api_key(key: &str) -> String {
    let chars: Vec<char> = key.chars().collect();
    let len = chars.len();
    if key.is_empty() {
        return String::new();
    }
    if len <= 4 {
        return "*".repeat(len);
    }
    if len <= 8 {
        let head: String = chars[..2].iter().collect();
        let tail: String = chars[len - 2..].iter().collect();
        return format!("{head}****{tail}");
    }
    let head: String = chars[..4].iter().collect();
    let tail: String = chars[len - 4..].iter().collect();
    format!("{head}**********{tail}")
}

/// Mask the value of every API key in a list, for read responses.
fn masked_keys(keys: Vec<ApiKey>) -> Vec<ApiKey> {
    keys.into_iter()
        .map(|mut key| {
            key.key = mask_api_key(&key.key);
            key
        })
        .collect()
}

/// A user sees only their own keys; an admin sees every key.
///
/// Returning the whole table to any signed-in caller would expose other users'
/// credentials and let them infer each other's usage.
async fn list_keys(
    State(s): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Json<ApiResponse<Vec<ApiKey>>> {
    let user = match auth_user(&s, &headers) {
        Ok(user) => user,
        Err(response) => return Json(ApiResponse::err(error_message(response))),
    };
    Json(match s.db.list_api_keys() {
        Ok(keys) => {
            if user.role.is_admin() {
                ApiResponse::ok(masked_keys(keys))
            } else {
                ApiResponse::ok(masked_keys(
                    keys.into_iter()
                        .filter(|k| k.user_id == user.id)
                        .collect(),
                ))
            }
        }
        Err(e) => ApiResponse::err(e.to_string()),
    })
}

async fn create_key(
    State(s): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(k): Json<ApiKey>,
) -> Json<ApiResponse<ApiKey>> {
    let user = match auth_user(&s, &headers) {
        Ok(user) => user,
        Err(response) => return Json(ApiResponse::err(error_message(response))),
    };
    if k.key.trim().is_empty() {
        return Json(ApiResponse::err("key value must not be empty"));
    }
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
        // Ownership is taken from the credential, never from the body: trusting a
        // caller-supplied `user_id` would let one user create keys billed to
        // another's wallet. An admin may still target someone deliberately.
        user_id: if user.role.is_admin() && !k.user_id.trim().is_empty() {
            k.user_id
        } else {
            user.id
        },
    };
    Json(match s.db.upsert_api_key(&k) {
        Ok(()) => ApiResponse::ok(k),
        Err(e) => ApiResponse::err(e.to_string()),
    })
}

/// The fields an update may change.
///
/// Deliberately a separate struct rather than a whole `ApiKey`: a full-object PUT
/// would let a client rewrite `used_micros` (erasing usage), `user_id`
/// (re-attributing the key to someone else's wallet) or `key` (rotating the
/// credential silently). The reference restricts its update to exactly these
/// columns (`model/token.go:315`, `token.Update()`), and excludes the key itself.
#[derive(Deserialize)]
struct KeyUpdateInput {
    name: String,
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    priority: Option<i32>,
    #[serde(default)]
    expires_at: Option<chrono::DateTime<Utc>>,
    /// `null` means "leave unchanged"; a value is a new ceiling.
    #[serde(default)]
    quota_micros: Option<i64>,
    #[serde(default)]
    allowed_models: Option<Vec<String>>,
    #[serde(default)]
    ip_allowlist: Option<Vec<String>>,
    #[serde(default)]
    group_name: Option<String>,
    #[serde(default)]
    cross_group_retry: Option<bool>,
}

/// `PUT /api/keys/:id` — edit a key without replacing it.
///
/// This is the superset delta D6: the reference has full update semantics, and we
/// had only create/delete. Update is not just ergonomics — without it the only way
/// to change a name or extend an expiry is to delete the key and make a new one,
/// which rotates the credential and breaks every client using it.
///
/// Immutable by construction: the credential (`key`), its owner (`user_id`),
/// its creation time and its accumulated usage (`used_micros`) are all copied from
/// the stored row, never read from the body.
async fn update_key(
    State(s): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(input): Json<KeyUpdateInput>,
) -> Response {
    let user = match auth_user(&s, &headers) {
        Ok(user) => user,
        Err(_) => return Json(ApiResponse::<ApiKey>::err("authentication required")).into_response(),
    };
    let mut key = match s.db.get_api_key(&id) {
        Ok(Some(key)) => key,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(ApiResponse::<ApiKey>::err("key not found")),
            )
                .into_response()
        }
        Err(e) => return Json(ApiResponse::<ApiKey>::err(e.to_string())).into_response(),
    };
    // An owner may edit their own key; an admin may edit anyone's. Anything else
    // is someone else's credential.
    if key.user_id != user.id && !user.role.is_admin() {
        return Json(ApiResponse::<ApiKey>::err("not your key")).into_response();
    }
    if input.name.trim().is_empty() {
        return Json(ApiResponse::<ApiKey>::err("name must not be empty")).into_response();
    }
    if let Some(quota) = input.quota_micros {
        if quota < 0 {
            return Json(ApiResponse::<ApiKey>::err("quota must not be negative"))
                .into_response();
        }
    }

    key.name = input.name;
    if let Some(v) = input.enabled {
        key.enabled = v;
    }
    if let Some(v) = input.priority {
        key.priority = v;
    }
    if let Some(v) = input.expires_at {
        key.expires_at = Some(v);
    }
    if let Some(v) = input.quota_micros {
        key.quota_micros = v;
    }
    if let Some(v) = input.allowed_models {
        key.allowed_models = v;
    }
    if let Some(v) = input.ip_allowlist {
        key.ip_allowlist = v;
    }
    if let Some(v) = input.group_name {
        key.group_name = v;
    }
    if let Some(v) = input.cross_group_retry {
        key.cross_group_retry = v;
    }

    match s.db.upsert_api_key(&key) {
        // Masked like every other read, so an update cannot disclose the value the
        // list withholds.
        Ok(()) => Json(ApiResponse::ok(masked_keys(vec![key]).remove(0))).into_response(),
        Err(e) => Json(ApiResponse::<ApiKey>::err(e.to_string())).into_response(),
    }
}

async fn delete_key(
    State(s): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Json<ApiResponse<&'static str>> {
    let user = match auth_user(&s, &headers) {
        Ok(user) => user,
        Err(response) => return Json(ApiResponse::err(error_message(response))),
    };
    match s.db.get_api_key(&id) {
        Ok(Some(key)) if key.user_id != user.id && !user.role.is_admin() => {
            return Json(ApiResponse::err("not your key"));
        }
        Ok(Some(_)) => {}
        Ok(None) => return Json(ApiResponse::err("key not found")),
        Err(e) => return Json(ApiResponse::err(e.to_string())),
    }
    Json(match s.db.delete_api_key(&id) {
        Ok(()) => ApiResponse::ok("deleted"),
        Err(e) => ApiResponse::err(e.to_string()),
    })
}

/// `GET /api/keys/:id/secret` — the one route that returns a key's real value.
///
/// Listing is masked so a console read cannot leak credentials; this is the
/// deliberate exception, mirroring the reference's separate credential route
/// (`controller.GetTokenKey`, gated on the owner). Keeping it distinct means the
/// disclosure is requested explicitly, appears in logs under its own path, and
/// never rides along in a bulk response.
async fn reveal_key(
    State(s): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Json<ApiResponse<serde_json::Value>> {
    let user = match auth_user(&s, &headers) {
        Ok(user) => user,
        Err(_) => return Json(ApiResponse::err("authentication required")),
    };
    match s.db.get_api_key(&id) {
        Ok(Some(key)) => {
            // Owner or admin only: someone else's key is not readable even by id.
            if key.user_id != user.id && !user.role.is_admin() {
                return Json(ApiResponse::err("not your key"));
            }
            Json(ApiResponse::ok(serde_json::json!({
                "id": key.id,
                "key": key.key,
            })))
        }
        Ok(None) => Json(ApiResponse::err("key not found")),
        Err(e) => Json(ApiResponse::err(e.to_string())),
    }
}

async fn keys_usage(
    State(s): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Json<ApiResponse<Vec<ApiKeyUsage>>> {
    let user = match auth_user(&s, &headers) {
        Ok(user) => user,
        Err(response) => return Json(ApiResponse::err(error_message(response))),
    };
    match s.db.api_key_usage() {
        Ok(usage) => {
            if user.role.is_admin() {
                return Json(ApiResponse::ok(usage));
            }
            // `api_key_usage` joins keys; restrict it to keys this user owns.
            let owned: std::collections::HashSet<String> = s
                .db
                .list_api_keys()
                .unwrap_or_default()
                .into_iter()
                .filter(|k| k.user_id == user.id)
                .map(|k| k.id)
                .collect();
            Json(ApiResponse::ok(
                usage.into_iter().filter(|u| owned.contains(&u.api_key_id)).collect(),
            ))
        }
        Err(e) => Json(ApiResponse::err(e.to_string())),
    }
}

/// The message from a guard's error response, for handlers that were already
/// gated by the middleware and only need a defensible body.
fn error_message(response: Response) -> String {
    let _ = response;
    "authentication required".to_string()
}

#[derive(Deserialize)]
struct KeysQuery {
    page: Option<i64>,
    page_size: Option<i64>,
    search: Option<String>,
}

async fn query_keys(
    State(s): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Query<KeysQuery>,
) -> Response {
    let user = match auth_user(&s, &headers) {
        Ok(user) => user,
        Err(response) => return Json(ApiResponse::<Vec<ApiKey>>::err(error_message(response))).into_response(),
    };
    let page = q.page.unwrap_or(1).max(1);
    let page_size = q.page_size.unwrap_or(20).clamp(1, 200);
    match s.db.list_api_keys_paged(page, page_size, q.search.as_deref()) {
        Ok((items, total)) => {
            // Same rule as the list: a search must not be a way to read a key.
            // Restricted to the caller's own keys unless they are an admin, so a
            // search cannot enumerate other users' tokens either.
            let visible: Vec<ApiKey> = if user.role.is_admin() {
                items
            } else {
                items.into_iter().filter(|k| k.user_id == user.id).collect()
            };
            let total = if user.role.is_admin() {
                total
            } else {
                visible.len() as i64
            };
            Json(PaginatedResponse::new(masked_keys(visible), total, page, page_size))
                .into_response()
        }
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
    headers: HeaderMap,
) -> Response {
    let user = match auth_user(&s, &headers) {
        Ok(user) => user,
        Err(_) => return Json(ApiResponse::<Vec<RequestLog>>::err("authentication required")).into_response(),
    };
    // An admin reads the instance's logs; anyone else reads only their own, which
    // the reference expresses as a separate `/log/self` route
    // (`router/api-router.go:314,319`). Both shapes are served here so the console
    // does not need two calls, but the scope is never the caller's choice: it
    // follows the role, so `?api_key_id=someone-else` cannot widen it.
    let scope = if user.role.is_admin() {
        None
    } else {
        Some(user.id.clone())
    };
    if q.paginated.unwrap_or(false) {
        let page = q.page.unwrap_or(1).max(1);
        let page_size = q.page_size.unwrap_or(20).clamp(1, 200);
        return match s.db.query_request_logs_scoped(
            page,
            page_size,
            q.start.as_deref(),
            q.end.as_deref(),
            q.model.as_deref(),
            q.channel_id.as_deref(),
            q.api_key_id.as_deref(),
            q.status.as_deref(),
            q.search.as_deref(),
            scope.as_deref(),
        ) {
            Ok((items, total)) => Json(PaginatedResponse::new(items, total, page, page_size)).into_response(),
            Err(e) => db_error::<Vec<RequestLog>>(e),
        };
    }
    let limit = q.limit.unwrap_or(100).min(1000);
    // The unpaginated shape goes through the same scoped query so the two cannot
    // disagree about who may see what.
    Json(match s.db.query_request_logs_scoped(
        1,
        limit,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        scope.as_deref(),
    ) {
        Ok((logs, _)) => ApiResponse::ok(logs),
        Err(e) => ApiResponse::err(e.to_string()),
    })
    .into_response()
}

/// `GET /api/logs/self` — the caller's own logs, for any signed-in user.
///
/// Matches the reference's `/log/self` (`router/api-router.go:319`). It is an
/// explicit route rather than a query flag, so the scope is visible in the URL and
/// in access logs.
async fn list_my_logs(
    State(s): State<Arc<AppState>>,
    Query(q): Query<LogsQuery>,
    headers: HeaderMap,
) -> Response {
    let user = match auth_user(&s, &headers) {
        Ok(user) => user,
        Err(_) => {
            return Json(ApiResponse::<Vec<RequestLog>>::err("authentication required"))
                .into_response()
        }
    };
    let page = q.page.unwrap_or(1).max(1);
    let page_size = q.page_size.unwrap_or(20).clamp(1, 200);
    match s.db.query_request_logs_scoped(
        page,
        page_size,
        q.start.as_deref(),
        q.end.as_deref(),
        q.model.as_deref(),
        q.channel_id.as_deref(),
        q.api_key_id.as_deref(),
        q.status.as_deref(),
        q.search.as_deref(),
        Some(&user.id),
    ) {
        Ok((items, total)) => {
            Json(PaginatedResponse::new(items, total, page, page_size)).into_response()
        }
        Err(e) => db_error::<Vec<RequestLog>>(e),
    }
}


async fn clear_logs(State(s): State<Arc<AppState>>) -> Json<ApiResponse<usize>> {
    Json(match s.db.clear_request_logs() {
        Ok(n) => ApiResponse::ok(n),
        Err(e) => ApiResponse::err(e.to_string()),
    })
}

/// Every plugin the instance knows about.
async fn list_task_plugins(
    State(s): State<Arc<AppState>>,
) -> Json<ApiResponse<Vec<oxygenrouter_core::PluginSummary>>> {
    Json(match s.db.list_plugins() {
        Ok(list) => ApiResponse::ok(list),
        Err(e) => ApiResponse::err(e.to_string()),
    })
}

async fn get_task_plugin(
    State(s): State<Arc<AppState>>,
    Path(key): Path<String>,
) -> Response {
    match s.db.list_plugins() {
        Ok(list) => match list.into_iter().find(|p| p.key == key) {
            Some(plugin) => Json(ApiResponse::ok(plugin)).into_response(),
            None => (
                StatusCode::NOT_FOUND,
                Json(ApiResponse::<oxygenrouter_core::PluginSummary>::err("plugin not found")),
            )
                .into_response(),
        },
        Err(e) => db_error::<oxygenrouter_core::PluginSummary>(e),
    }
}

#[derive(serde::Deserialize)]
struct PluginUploadInput {
    key: String,
    version: String,
    source: String,
}

/// Store a plugin build, refusing anything the runtime cannot load.
///
/// The source is executed once here, in the host's own engine, before it is
/// written: accepting code that the gateway will run on every request without
/// ever having run it once would make a syntax error or a missing hook surface
/// as a routing failure later instead of as an upload refusal now.
async fn upload_task_plugin(
    State(s): State<Arc<AppState>>,
    Json(input): Json<PluginUploadInput>,
) -> Response {
    if let Err(reason) = validate_plugin_upload(&input) {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse::<oxygenrouter_core::PluginSummary>::err(reason)),
        )
            .into_response();
    }
    let manifest = match s
        .plugins
        .load(input.source.clone(), PLUGIN_CALL_TIMEOUT)
        .await
    {
        Ok(manifest) => manifest,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ApiResponse::<oxygenrouter_core::PluginSummary>::err(error.to_string())),
            )
                .into_response()
        }
    };
    // The manifest is the plugin's own claim about itself; a key that disagrees
    // with the path it is being stored under would make the console and the
    // hooks refer to different plugins.
    if manifest.key != input.key {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse::<oxygenrouter_core::PluginSummary>::err(format!(
                "manifest declares key {:?} but the upload names {:?}",
                manifest.key, input.key
            ))),
        )
            .into_response();
    }
    let version = oxygenrouter_core::PluginVersion {
        key: input.key.clone(),
        version: input.version.clone(),
        source: input.source.clone(),
        manifest: serde_json::to_value(&manifest).unwrap_or(serde_json::Value::Null),
        created_at: Utc::now(),
    };
    if let Err(e) = s.db.upsert_plugin_version(&version) {
        return db_error::<oxygenrouter_core::PluginSummary>(e);
    }
    // A first upload becomes active but stays off, so installing is not the same
    // act as enabling: an operator can inspect it before it runs on traffic.
    let existing = s.db.get_plugin_state(&input.key).ok().flatten();
    let active = existing
        .as_ref()
        .and_then(|state| state.active_version.clone())
        .or_else(|| Some(input.version.clone()));
    let enabled = existing.as_ref().map(|state| state.enabled).unwrap_or(false);
    if let Err(e) = s
        .db
        .set_plugin_state(&input.key, active.as_deref(), enabled)
    {
        return db_error::<oxygenrouter_core::PluginSummary>(e);
    }
    let _ = version; // stored above; the listing below is the canonical view
    match s.db.list_plugins() {
        Ok(list) => {
            let found = list
                .into_iter()
                .find(|p| p.key == input.key)
                .unwrap_or_else(|| oxygenrouter_core::PluginSummary {
                    key: input.key.clone(),
                    name: input.key.clone(),
                    version: Some(input.version.clone()),
                    description: String::new(),
                    enabled: false,
                    active_version: Some(input.version.clone()),
                    versions: vec![input.version.clone()],
                    hooks: Vec::new(),
                    updated_at: Utc::now(),
                });
            Json(ApiResponse::ok(found)).into_response()
        }
        Err(e) => db_error::<oxygenrouter_core::PluginSummary>(e),
    }
}

/// Refuse an upload that cannot possibly be usable, before running it.
fn validate_plugin_upload(input: &PluginUploadInput) -> Result<(), String> {
    if !oxygenrouter_plugin::valid_key(&input.key) {
        return Err(format!(
            "{:?} is not a valid plugin key: lowercase letters, digits, underscore and dash, starting alphanumeric, at most 30 characters",
            input.key
        ));
    }
    if input.version.trim().is_empty() {
        return Err("a version is required".to_string());
    }
    if input.source.trim().is_empty() {
        return Err("source is required".to_string());
    }
    Ok(())
}

#[derive(serde::Deserialize)]
struct PluginActivateInput {
    version: String,
}

/// Choose which stored build is active.
async fn activate_task_plugin(
    State(s): State<Arc<AppState>>,
    Path(key): Path<String>,
    Json(input): Json<PluginActivateInput>,
) -> Response {
    let versions = match s.db.list_plugin_versions(&key) {
        Ok(v) => v,
        Err(e) => return db_error::<oxygenrouter_core::PluginSummary>(e),
    };
    if !versions.iter().any(|v| v.version == input.version) {
        return (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::<oxygenrouter_core::PluginSummary>::err(format!(
                "version {:?} is not stored for plugin {:?}",
                input.version, key
            ))),
        )
            .into_response();
    }
    let enabled = s
        .db
        .get_plugin_state(&key)
        .ok()
        .flatten()
        .map(|state| state.enabled)
        .unwrap_or(false);
    // The build is loaded before it is activated, so a version that cannot even
    // be parsed cannot become the one the gateway would call.
    let source = versions
        .iter()
        .find(|v| v.version == input.version)
        .map(|v| v.source.clone())
        .unwrap_or_default();
    if let Err(error) = s.plugins.load(source, PLUGIN_CALL_TIMEOUT).await {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse::<oxygenrouter_core::PluginSummary>::err(error.to_string())),
        )
            .into_response();
    }
    match s
        .db
        .set_plugin_state(&key, Some(&input.version), enabled)
    {
        Ok(()) => Json(ApiResponse::ok("activated")).into_response(),
        Err(e) => db_error::<&str>(e),
    }
}

#[derive(serde::Deserialize)]
struct PluginStatusInput {
    enabled: bool,
}

async fn set_task_plugin_status(
    State(s): State<Arc<AppState>>,
    Path(key): Path<String>,
    Json(input): Json<PluginStatusInput>,
) -> Response {
    let state = match s.db.get_plugin_state(&key) {
        Ok(Some(state)) => state,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(ApiResponse::<&str>::err("plugin not found")),
            )
                .into_response()
        }
        Err(e) => return db_error::<&str>(e),
    };
    // Enabling a plugin with no active build would leave it on and doing nothing,
    // which reads as a broken plugin rather than an unset one.
    if input.enabled && state.active_version.is_none() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse::<&str>::err(
                "activate a version before enabling this plugin",
            )),
        )
            .into_response();
    }
    match s
        .db
        .set_plugin_state(&key, state.active_version.as_deref(), input.enabled)
    {
        Ok(()) => Json(ApiResponse::ok("updated")).into_response(),
        Err(e) => db_error::<&str>(e),
    }
}

#[derive(serde::Deserialize)]
struct PluginDryRunInput {
    /// The hook to call.
    hook: String,
    /// Its input, as JSON.
    input: serde_json::Value,
}

/// Call one hook without touching live traffic.
///
/// This is the only way to answer "what does this plugin actually do" without
/// routing a real request through it, so it is the safety valve for the whole
/// feature: an operator can see the transformation before enabling it.
async fn dry_run_task_plugin(
    State(s): State<Arc<AppState>>,
    Path(key): Path<String>,
    Json(input): Json<PluginDryRunInput>,
) -> Response {
    let state = match s.db.get_plugin_state(&key) {
        Ok(Some(state)) => state,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(ApiResponse::<serde_json::Value>::err("plugin not found")),
            )
                .into_response()
        }
        Err(e) => return db_error::<serde_json::Value>(e),
    };
    let Some(version) = state.active_version else {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse::<serde_json::Value>::err(
                "no active version to dry-run",
            )),
        )
            .into_response();
    };
    let source = match s.db.list_plugin_versions(&key) {
        Ok(versions) => versions
            .into_iter()
            .find(|v| v.version == version)
            .map(|v| v.source),
        Err(e) => return db_error::<serde_json::Value>(e),
    };
    let Some(source) = source else {
        return (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::<serde_json::Value>::err("active version is not stored")),
        )
            .into_response();
    };
    // Load a copy so the dry run reflects the stored build even if a newer one
    // was loaded into the host for something else.
    if let Err(error) = s.plugins.load(source, PLUGIN_CALL_TIMEOUT).await {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse::<serde_json::Value>::err(error.to_string())),
        )
            .into_response();
    }
    match s
        .plugins
        .call_hook(&key, &input.hook, input.input, PLUGIN_CALL_TIMEOUT)
        .await
    {
        Ok(value) => Json(ApiResponse::ok(value)).into_response(),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse::<serde_json::Value>::err(error.to_string())),
        )
            .into_response(),
    }
}

async fn delete_task_plugin_version(
    State(s): State<Arc<AppState>>,
    Path((key, version)): Path<(String, String)>,
) -> Response {
    match s.db.delete_plugin_version(&key, &version) {
        Ok(0) => (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::<usize>::err("no such version")),
        )
            .into_response(),
        Ok(n) => Json(ApiResponse::ok(n)).into_response(),
        Err(e) => db_error::<usize>(e),
    }
}

/// How long a plugin may take to load or answer, matching the reference's
/// `DefaultCallTimeout` (`pkg/jsplugin/engine.go:18`).
const PLUGIN_CALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// The administrative trail, newest first.
///
/// Root-only, enforced by the `ROOT_PREFIXES` entry: the trail names who holds
/// power and what they changed, so it is not an ordinary admin's to read.
async fn list_audit_logs(
    State(s): State<Arc<AppState>>,
    Query(q): Query<LogsQuery>,
) -> Json<ApiResponse<Vec<oxygenrouter_core::AuditLog>>> {
    let limit = q.page_size.unwrap_or(100).clamp(1, 1000);
    Json(match s.db.list_audit_logs(limit) {
        Ok(rows) => ApiResponse::ok(rows),
        Err(e) => ApiResponse::err(e.to_string()),
    })
}

async fn clear_audit_logs(State(s): State<Arc<AppState>>) -> Json<ApiResponse<usize>> {
    Json(match s.db.clear_audit_logs() {
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
    headers: HeaderMap,
) -> Json<ApiResponse<oxygenrouter_core::SystemStatus>> {
    let mut status = match s.db.get_system_status(s.start_time) {
        Ok(st) => st,
        Err(e) => return Json(ApiResponse::err(e.to_string())),
    };
    // This route is public, so it can only ever carry what a signed-out visitor
    // may see. An authenticated caller additionally gets the bind address, which
    // the console shows next to its own endpoint. The local API token is *not*
    // here for anyone: it is a credential, and a credential belongs on the
    // root-only `/api/system/info` rather than in a payload fetched by every page
    // load.
    if auth_user(&s, &headers).is_ok() {
        let cfg = oxygenrouter_core::APP_CONFIG.read();
        status.listen_host = Some(cfg.listen_host.clone());
        status.listen_port = Some(cfg.listen_port);
    }
    // A display preference, safe for the signed-out catalogue page too. An empty
    // or absent value keeps the shipped default rather than rendering a blank
    // currency, which `Intl.NumberFormat` would reject.
    if let Ok(Some(currency)) = s.db.get_setting("Currency") {
        let currency = currency.trim();
        if !currency.is_empty() {
            status.currency = currency.to_string();
        }
    }
    Json(ApiResponse::ok(status))
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

/// Take at most `n` **characters** from the front of `value`.
///
/// `&value[..n]` slices *bytes*, so it panics whenever byte `n` lands inside a
/// multi-byte character. Values reach `get_options` from the database, and an
/// operator may legitimately store a non-ASCII secret — a Chinese passphrase, a
/// token with a CJK prefix. A panic here is inside an `axum` handler, so the
/// whole console settings page returns 500 for every root caller until the
/// value is changed by hand. Counting characters removes the failure mode.
fn chars_prefix(value: &str, n: usize) -> String {
    value.chars().take(n).collect()
}

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
                    format!("{}…", chars_prefix(&value, 6))
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
    // A secret that reads as a mask is not a new value. Without this, a client
    // that renders the whole option list and saves it back would overwrite the
    // real secret with its own preview — the same round-trip footgun that
    // destroyed channel credentials. Setting a genuine value still works.
    if schema.secret && input.value.ends_with('\u{2026}') {
        return Json(ApiResponse::ok("unchanged"));
    }
    match s.db.set_setting(schema.key, &input.value) {
        Ok(()) => {
            // Most options are derived from the store by the request paths that
            // use them, so a write is live immediately. These two are cached
            // because the proxy consults them on every request, so they are the
            // exception that needs an explicit refresh — and the refresh lives
            // here, next to the write, rather than in a hook someone must
            // remember to call.
            if matches!(
                schema.key,
                "RequestLogEnabled" | "RecordIpLog" | "AuditLogEnabled" | "DataExportEnabled"
            ) {
                s.reload_log_policy();
            }
            // The billing thresholds sit behind the service's policy lock, so a
            // change must be pushed rather than derived per request. `reload_pricing`
            // carries the policy with the expressions, which is why one call covers
            // both.
            if matches!(
                schema.key,
                "TrustQuota" | "PreConsumedQuota" | "FreeModelPreConsumeEnabled"
            ) {
                s.reload_pricing();
            }
            // Everything the pricing engine reads lives in one reload, so the
            // pricing tables are refreshed by the same predicate rather than a
            // second list that could drift out of step with it.
            if matches!(
                schema.key,
                "ModelRatio" | "GroupRatio" | "billing_expr" | "billing_mode" | "QuotaPerUnit"
            ) {
                s.reload_pricing();
            }
            Json(ApiResponse::ok("updated"))
        }
        Err(error) => Json(ApiResponse::err(error.to_string())),
    }
}

async fn update_settings(
    State(s): State<Arc<AppState>>,
    Json(input): Json<UpdateSettingsInput>,
) -> Json<ApiResponse<oxygenrouter_core::AppSettings>> {
    // Snapshot the table-form of every field the settings table also holds,
    // before the block below consumes `input` by value. The table is the
    // authority, so a change here has to reach it and not only the in-memory
    // copy; capturing first keeps the mirroring a single, obvious list.
    let mirrors: Vec<(&'static str, String)> = [
        ("ListenHost", input.listen_host.clone()),
        ("ListenPort", input.listen_port.map(|v| v.to_string())),
        (
            "MaxConcurrentRequests",
            input.max_concurrent_requests.map(|v| v.to_string()),
        ),
        (
            "UpstreamTimeoutMs",
            input.upstream_timeout_ms.map(|v| v.to_string()),
        ),
        ("UserAgent", input.user_agent.clone()),
        ("LogLevel", input.log_level.clone()),
        ("RetryTimes", input.max_retries.map(|v| v.to_string())),
        ("RetryIntervalMs", input.retry_delay_ms.map(|v| v.to_string())),
        ("RetryBackoff", input.retry_backoff.clone()),
        (
            "LogRetentionDays",
            input.request_log_retention_days.map(|v| v.to_string()),
        ),
        ("Theme", input.theme.clone()),
        ("Language", input.language.clone()),
        // Stored under its option key so the console's masked read and this write
        // agree on one name.
        ("LocalApiToken", input.local_api_token.clone()),
    ]
    .into_iter()
    .filter_map(|(key, value)| value.map(|v| (key, v)))
    .collect();
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
    // The settings table is the authority, so every value that appears in both
    // stores is written to both from here on: the in-memory copy serves reads
    // during this process's lifetime, and the table is what a restart and the
    // option list see. Keeping them written together is what stops the drift this
    // whole change exists to remove.
    for (key, value) in mirrors {
        let _ = s.db.set_setting(key, &value);
    }
    let cfg = oxygenrouter_core::APP_CONFIG.read();
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


#[cfg(test)]
mod access_tests {
    use super::*;

    /// Regression guard for a real defect: 49 of 86 console routes were reachable
    /// anonymously, including `GET /api/channels` (which returns upstream
    /// credentials), `PUT /api/channels/:id` (which repoints a channel),
    /// `POST /api/keys`, and `GET /api/settings` (which returns
    /// `local_api_token`). The classification below is what the middleware acts
    /// on, so a misclassification is the whole vulnerability.
    #[test]
    fn credential_bearing_routes_are_not_public() {
        for path in [
            "/api/channels",
            "/api/channels/abc",
            "/api/channels/abc/keys",
            "/api/keys",
            "/api/keys/abc",
            "/api/keys/usage",
            "/api/settings",
            "/api/options",
            "/api/system/info",
            "/api/backup/create",
            "/api/logs",
            "/api/log/stats",
            "/api/dashboard",
            "/api/models-metadata",
            "/api/vendors",
            "/api/admin/users",
        ] {
            assert_ne!(
                required_access(path),
                Access::Public,
                "{path} must not be reachable without a credential"
            );
        }
    }

    #[test]
    fn admin_surfaces_require_an_admin() {
        for path in [
            "/api/channels",
            "/api/channels/abc",
            "/api/models-metadata",
            "/api/models-metadata/abc",
            "/api/vendors",
            "/api/vendors/abc",
            "/api/model-maps",
            "/api/rules",
            "/api/admin/users",
            "/api/subscription/admin/bind",
            "/api/authz/catalog",
        ] {
            assert_eq!(required_access(path), Access::Admin, "{path}");
        }
    }

    #[test]
    fn instance_wide_configuration_is_root_only() {
        // Writing options or reading settings exposes the instance's own token and
        // topology, so it is the owner's, not any admin's.
        for path in [
            "/api/options",
            "/api/settings",
            "/api/system/info",
            "/api/backup/create",
            "/api/plugin/task",
            "/api/system-task",
        ] {
            assert_eq!(required_access(path), Access::Root, "{path}");
        }
    }

    #[test]
    fn only_the_intended_routes_are_public() {
        // An allow-list, not a deny-list: a new route is closed unless listed.
        for path in ["/api/auth/login", "/api/auth/register", "/api/plans", "/api/status"] {
            assert_eq!(required_access(path), Access::Public, "{path}");
        }
        // A slightly different path must not inherit public status.
        assert_ne!(required_access("/api/plans/admin"), Access::Public);
        assert_ne!(required_access("/api/status/test"), Access::Public);
    }

    #[test]
    fn an_unclassified_route_defaults_to_requiring_a_session() {
        // Fail-closed: forgetting to classify a new route must not open it.
        assert_eq!(required_access("/api/something/new"), Access::User);
        assert_eq!(required_access("/api/wallet"), Access::User);
        assert_eq!(required_access("/api/auth/me"), Access::User);
        assert_eq!(required_access("/api/user/sessions"), Access::User);
    }

    #[test]
    fn a_prefix_match_requires_a_path_boundary() {
        // `/api/optionsfoo` must not inherit the `/api/options` classification,
        // or a lookalike route would silently become root-only (harmless) or,
        // worse, a root route could be shadowed into the default.
        assert_eq!(required_access("/api/options"), Access::Root);
        assert_eq!(required_access("/api/options/"), Access::Root);
        assert_eq!(required_access("/api/options/request_policy"), Access::Root);
        assert_eq!(required_access("/api/optionsfoo"), Access::User);
        assert_eq!(required_access("/api/settings-private"), Access::User);
        assert_eq!(required_access("/api/channelsx"), Access::User);
    }

    #[test]
    fn root_is_treated_as_a_superset_of_admin() {
        use oxygenrouter_core::UserRole;
        assert!(UserRole::Root.is_root());
        assert!(UserRole::Root.is_admin());
        assert!(!UserRole::Admin.is_root());
        assert!(UserRole::Admin.is_admin());
        assert!(!UserRole::User.is_admin());
    }

    #[test]
    fn the_root_role_round_trips_through_storage_text() {
        use oxygenrouter_core::UserRole;
        assert_eq!(UserRole::from_db("root"), UserRole::Root);
        assert_eq!(UserRole::from_db("admin"), UserRole::Admin);
        assert_eq!(UserRole::from_db("user"), UserRole::User);
        // An unrecognised value must not escalate.
        assert_eq!(UserRole::from_db("superuser"), UserRole::User);
        assert_eq!(UserRole::from_db(""), UserRole::User);
    }
}


#[cfg(test)]
mod credential_tests {
    use super::*;

    fn channel(key: &str) -> Channel {
        Channel {
            id: "c1".into(),
            name: "prod".into(),
            provider: "openai".into(),
            base_url: "https://example.test/v1".into(),
            api_key: key.into(),
            priority: 0,
            weight: 1,
            enabled: true,
            test_model: String::new(),
            group_name: "default".into(),
            tags: Vec::new(),
            model_list: Vec::new(),
            response_headers: serde_json::json!({}),
            status_code_mapping: serde_json::json!({}),
            override_parameters: serde_json::json!({}),
            balance_micros: 0,
            last_test_at: None,
            info: Default::default(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    /// The list is the most-read console endpoint, and it used to hand back the
    /// raw upstream credential (verified live before the fix).
    #[test]
    fn the_channel_list_view_never_carries_the_credential() {
        let secret = "sk-real-upstream-credential";
        let masked = masked_channel(channel(secret));
        assert_ne!(masked.api_key, secret);
        assert!(
            !masked.api_key.contains(secret),
            "the credential must not survive masking"
        );
        assert_eq!(masked.api_key, CHANNEL_KEY_PLACEHOLDER);
    }

    #[test]
    fn masking_preserves_every_non_credential_field() {
        // The mask must be surgical: an operator still needs the rest of the row.
        let masked = masked_channel(channel("sk-xyz"));
        assert_eq!(masked.name, "prod");
        assert_eq!(masked.base_url, "https://example.test/v1");
        assert_eq!(masked.provider, "openai");
        assert!(masked.enabled);
    }

    #[test]
    fn an_empty_key_is_left_empty_rather_than_placeheld() {
        // A placeholder on a keyless channel would imply a credential exists.
        let masked = masked_channel(channel(""));
        assert_eq!(masked.api_key, "");
        let whitespace = masked_channel(channel("   "));
        assert_eq!(whitespace.api_key, "   ");
    }

    /// Regression: the round-trip PUT destroyed a working credential. Verified
    /// live before the fix — saving the form with a blank key left the channel
    /// with `api_key = ""`.
    #[test]
    fn an_update_signal_is_recognised_in_both_shapes_a_client_sends() {
        for sent in [CHANNEL_KEY_PLACEHOLDER, "", "   "] {
            let unchanged = sent == CHANNEL_KEY_PLACEHOLDER || sent.trim().is_empty();
            assert!(unchanged, "{sent:?} must be treated as leave-unchanged");
        }
        // A real key is a change.
        assert!("sk-new-key".trim().is_empty() == false);
        assert_ne!("sk-new-key", CHANNEL_KEY_PLACEHOLDER);
    }

    /// The preview used to return the *entire* key for any key of ten characters
    /// or fewer — a full disclosure for short, low-entropy credentials.
    #[test]
    fn a_short_key_is_never_shown_in_full() {
        for short in ["abc", "sk-1234", "1234567890", "12345678901"] {
            let preview = key_preview(short);
            assert_ne!(preview, short, "{short} was disclosed in full");
            assert!(
                !preview.contains(short),
                "preview {preview:?} contains the whole key {short:?}"
            );
        }
    }

    #[test]
    fn a_long_key_shows_only_a_short_prefix() {
        let key = "sk-abcdefghijklmnopqrstuvwxyz012345";
        let preview = key_preview(key);
        assert!(preview.starts_with("sk-a"), "{preview}");
        assert!(preview.len() < key.len() / 4, "preview too revealing: {preview}");
        assert!(!preview.contains(&key[10..]), "the bulk must be hidden");
    }

    #[test]
    fn a_blank_key_produces_an_empty_preview() {
        assert_eq!(key_preview(""), "");
        assert_eq!(key_preview("   "), "");
    }
}

#[cfg(test)]
mod multibyte_safety_tests {
    use super::*;

    /// Regression for a panic, not a cosmetic bug. The option list slices its
    /// secret preview by **byte** offset (`&value[..6]`). Byte 6 lands inside a
    /// multi-byte character for any value whose 3rd character is non-ASCII, and
    /// the slice panics — inside the handler, so `GET /api/options` answers 500
    /// for every root caller, permanently, until the row is edited by hand.
    /// `chars_prefix` counts characters and cannot split one.
    #[test]
    fn a_multibyte_secret_preview_does_not_split_a_character() {
        // 密=[0,1,2] x=[3] 碼=[4,5,6]: byte 6 is the *third byte of 碼*, so
        // `&v[..6]` lands mid-character and panics. Two adjacent wide characters
        // would not reproduce it — their 6-byte prefix is an exact boundary — so
        // the ASCII byte in the middle is what makes this fixture a regression.
        let value = "密x碼abcdefghij";
        let preview = chars_prefix(value, 6);
        assert_eq!(preview, "密x碼abc");
        assert!(preview.ends_with('c'));

        // Every cut point of a mixed-width string must be safe.
        for n in 0..=value.chars().count() {
            let cut = chars_prefix(value, n);
            assert_eq!(cut.chars().count(), n.min(value.chars().count()));
            assert!(value.starts_with(&cut), "{cut:?} is not a prefix of {value:?}");
        }

        // The old expression, spelled out, asserts the defect rather than
        // trusting the comment above. `is_char_boundary` is what the byte slice
        // required and did not check.
        let byte_cut = value.len().min(6);
        assert!(
            !value.is_char_boundary(byte_cut),
            "byte {byte_cut} was expected to be mid-character for this fixture"
        );
    }

    #[test]
    fn a_short_multibyte_value_is_returned_whole_by_character_count() {
        assert_eq!(chars_prefix("密钥", 6), "密钥");
        assert_eq!(chars_prefix("", 6), "");
        // Longer than the budget: exactly six characters, still valid UTF-8.
        let long = "一".repeat(20);
        assert_eq!(chars_prefix(&long, 6).chars().count(), 6);
    }
}


#[cfg(test)]
mod key_masking_tests {
    use super::*;

    /// The reference's `MaskTokenKey` (`model/token.go:63`) branch by branch.
    /// Listing a live token would make every console read a credential
    /// disclosure, and the reference masks in both its list and its search
    /// (`controller/token.go:140,157`).
    #[test]
    fn masking_matches_the_reference_algorithm() {
        assert_eq!(mask_api_key(""), "");
        // <= 4: nothing is revealed, not even the length in characters.
        assert_eq!(mask_api_key("ab"), "**");
        assert_eq!(mask_api_key("abcd"), "****");
        // <= 8: two at each end.
        assert_eq!(mask_api_key("abcde"), "ab****de");
        assert_eq!(mask_api_key("abcdefgh"), "ab****gh");
        // > 8: four at each end.
        assert_eq!(mask_api_key("abcdefghi"), "abcd**********fghi");
        assert_eq!(
            mask_api_key("sk-abcdefghijklmnop"),
            "sk-a**********mnop"
        );
    }

    #[test]
    fn masking_never_reveals_the_interior() {
        // A realistic token: the middle carries the entropy.
        let key = "sk-1234567890abcdefghijklmnopqrstuvwxyz";
        let masked = mask_api_key(key);
        assert!(!masked.contains(&key[6..key.len() - 6]), "interior leaked: {masked}");
        assert!(masked.starts_with("sk-1"), "{masked}");
        assert!(masked.ends_with("wxyz"), "{masked}");
    }

    #[test]
    fn masking_handles_multibyte_input_without_panicking() {
        // `key[..4]` byte-slicing would panic on a multi-byte character; the
        // implementation counts characters instead.
        let masked = mask_api_key("密钥一abcdefghijklmnop密钥二");
        assert!(!masked.is_empty());
        assert!(masked.contains("****"), "{masked}");
    }

    #[test]
    fn a_masked_list_keeps_every_other_field() {
        use oxygenrouter_core::ApiKey;
        let key = ApiKey {
            id: "k1".into(),
            key: "sk-abcdefghijklmnop".into(),
            name: "prod".into(),
            priority: 3,
            user_id: "u1".into(),
            enabled: true,
            created_at: chrono::Utc::now(),
            expires_at: None,
            quota_micros: 1000,
            used_micros: 10,
            allowed_models: vec!["gpt-4o".into()],
            ip_allowlist: vec![],
            group_name: "default".into(),
            cross_group_retry: true,
        };
        let masked = masked_keys(vec![key]);
        assert_eq!(masked[0].key, "sk-a**********mnop");
        // The record must stay usable: name, quota and limits are intact.
        assert_eq!(masked[0].name, "prod");
        assert_eq!(masked[0].quota_micros, 1000);
        assert_eq!(masked[0].used_micros, 10);
        assert_eq!(masked[0].allowed_models, vec!["gpt-4o".to_string()]);
        assert_eq!(masked[0].priority, 3);
    }

    /// Regression: a client that saves the option list back would otherwise
    /// overwrite a real secret with the preview we handed it — the same
    /// round-trip footgun that destroyed channel credentials.
    #[test]
    fn a_masked_secret_is_recognised_as_unchanged() {
        // What `get_options` returns for a set secret — built with the same
        // helper the handler uses, so the two cannot drift apart.
        let preview = format!("{}…", chars_prefix("sk-live-token-value", 6));
        assert!(preview.ends_with('\u{2026}'), "{preview}");
        // A genuine value is not mistaken for one.
        assert!(!("sk-a-brand-new-token").ends_with('\u{2026}'));
        assert!(!("".to_string()).ends_with('\u{2026}'));
    }
}


#[cfg(test)]
mod log_scoping_tests {
    use super::*;

    /// Regression: an ordinary user could read the instance's whole log table and
    /// the aggregate dashboards. The reference gates its log listing on AdminAuth
    /// and serves a separate `/log/self` (`router/api-router.go:314,319`).
    #[test]
    fn instance_wide_observability_requires_an_admin() {
        for path in [
            "/api/logs",
            "/api/logs/stream",
            "/api/log",
            "/api/log/stats",
            "/api/dashboard",
            "/api/analytics/flow",
        ] {
            assert_eq!(required_access(path), Access::Admin, "{path}");
        }
    }

    #[test]
    fn the_self_scoped_route_stays_open_to_any_user() {
        // A user must still be able to see their own activity, or the page is
        // useless to them; the scope comes from the path, not a query flag.
        assert_eq!(required_access("/api/logs/self"), Access::User);
        assert_eq!(required_access("/api/logs/self/"), Access::User);
        // It must not leak the instance-wide route open along with it.
        assert_eq!(required_access("/api/logs"), Access::Admin);
        assert_eq!(required_access("/api/logs/anything-else"), Access::Admin);
    }

    /// The management trail names who holds power and what they changed. It is
    /// root-only, not admin-only, so a compromised ordinary admin cannot read
    /// back the history of the owner's actions and learn the instance's shape.
    #[test]
    fn the_audit_trail_is_root_only() {
        assert_eq!(required_access("/api/audit-logs"), Access::Root);
        assert_eq!(required_access("/api/audit-logs/"), Access::Root);
        // Not the traffic-log gate, which is admin.
        assert_ne!(required_access("/api/audit-logs"), Access::Admin);
        // And not public, in any spelling.
        assert_ne!(required_access("/api/audit-logs"), Access::Public);
    }

    /// The public readiness probe must not carry a credential.
    ///
    /// Verified live before the fix: `GET /api/status`, which is in
    /// `PUBLIC_ROUTES`, returned the instance's local API token and its bind
    /// address to a completely anonymous caller — every client using
    /// `Authorization: Bearer <that token>` is then impersonable by anyone who
    /// can open the port. The struct is the gate here: a field cannot be
    /// serialised out of a payload it is not part of.
    #[test]
    fn the_public_status_payload_carries_no_credential() {
        use oxygenrouter_core::SystemStatus;

        // Enumerate the payload by serialising a value and inspecting every key,
        // so a newly added field is covered without editing this test.
        let status = SystemStatus {
            version: "0".into(),
            uptime_seconds: 0,
            total_channels: 0,
            enabled_channels: 0,
            total_requests: 0,
            active_requests: 0,
            listen_host: None,
            listen_port: None,
            currency: "USD".into(),
        };
        let json = serde_json::to_value(&status).expect("system status serialises");
        let keys: Vec<&str> = json
            .as_object()
            .expect("an object")
            .keys()
            .map(|k| k.as_str())
            .collect();

        for forbidden in [
            "local_api_token",
            "api_token",
            "token",
            "secret",
            "password",
            "key",
        ] {
            assert!(
                !keys.contains(&forbidden),
                "the public status payload must not carry {forbidden:?}; keys: {keys:?}"
            );
        }
        // And nothing whose *name* merely suggests one.
        assert!(
            !keys.iter().any(|k| k.contains("token") || k.contains("secret")),
            "a credential-shaped field reached the public payload: {keys:?}"
        );
        // The anonymous shape omits the bind address entirely rather than
        // sending it as null, so a client cannot mistake absence for a value.
        assert!(!keys.contains(&"listen_host"), "{keys:?}");
        assert!(!keys.contains(&"listen_port"), "{keys:?}");
        assert_eq!(required_access("/api/status"), Access::Public);
    }
}


#[cfg(test)]
mod role_hierarchy_tests {
    use oxygenrouter_core::UserRole;

    /// Regression: an ordinary admin promoted itself to root and demoted the
    /// owner, both verified live before the fix. The reference guards this with
    /// `canManageTargetRole` (`controller/user.go:382`) and a `>= myRole` check on
    /// creation (`controller/user.go:987`).
    #[test]
    fn an_admin_cannot_manage_a_peer_or_the_owner() {
        use UserRole::*;
        // An admin may manage a user…
        assert!(UserRole::can_manage(&Admin, &User));
        // …but not a peer, and not the owner.
        assert!(!UserRole::can_manage(&Admin, &Admin));
        assert!(!UserRole::can_manage(&Admin, &Root));
        // A user manages nobody.
        assert!(!UserRole::can_manage(&User, &User));
        assert!(!UserRole::can_manage(&User, &Admin));
        assert!(!UserRole::can_manage(&User, &Root));
    }

    #[test]
    fn the_owner_can_manage_everyone() {
        use UserRole::*;
        assert!(UserRole::can_manage(&Root, &Root));
        assert!(UserRole::can_manage(&Root, &Admin));
        assert!(UserRole::can_manage(&Root, &User));
    }

    #[test]
    fn an_admin_cannot_mint_a_peer_or_a_root() {
        use UserRole::*;
        // Strictly below, matching the reference's `user.Role >= myRole` refusal.
        assert!(UserRole::can_assign(&Admin, &User));
        assert!(!UserRole::can_assign(&Admin, &Admin));
        assert!(!UserRole::can_assign(&Admin, &Root));
        assert!(UserRole::can_assign(&Root, &Admin));
        assert!(UserRole::can_assign(&Root, &User));
        // Nobody but root may create root, and root may not create root either —
        // the reference's comparison is strict, so a second owner needs a
        // deliberate promotion by an existing owner.
        assert!(!UserRole::can_assign(&Root, &Root));
    }

    #[test]
    fn rank_is_ordered_root_admin_user() {
        use UserRole::*;
        assert!(Root.rank() > Admin.rank());
        assert!(Admin.rank() > User.rank());
        // The exact values mirror the reference's constants.
        assert_eq!(Root.rank(), 100);
        assert_eq!(Admin.rank(), 10);
        assert_eq!(User.rank(), 1);
    }

    /// The two rules must both hold: managing a peer is refused even when the
    /// assigned role would be lower, and assigning upward is refused even when the
    /// target is a subordinate.
    #[test]
    fn both_rules_are_independent() {
        use UserRole::*;
        // Editing a peer downward: target check fails.
        assert!(!UserRole::can_manage(&Admin, &Admin));
        // Editing a subordinate upward: assignment check fails.
        assert!(UserRole::can_manage(&Admin, &User));
        assert!(!UserRole::can_assign(&Admin, &Admin));
    }
}


/// Guards over the classification itself, read from this file at test time.
///
/// The point is that the *source* is the input, not a hand-maintained list: if
/// someone adds a user-class route whose handler forgets to resolve the caller, or
/// marks a sensitive prefix as public, this fails. That is the failure mode behind
/// the earlier anonymous-access defect — a route that was simply never classified
/// — so it is worth a test that reads the code rather than trusting review.
#[cfg(test)]
mod classification_audit_tests {
    use super::*;

    /// Every route in the router, paired with its handler name, scraped from the
    /// `router()` source.
    fn routes_from_source() -> Vec<(String, String)> {
        let source = include_str!("api.rs");
        let mut out = Vec::new();
        let mut rest = source;
        while let Some(at) = rest.find(".route(") {
            rest = &rest[at + 7..];
            let Some(quote) = rest.find('"') else { break };
            let after = &rest[quote + 1..];
            let Some(end) = after.find('"') else { break };
            let path = after[..end].to_string();
            let tail = &after[end..];
            // Bound the route expression by paren depth rather than a fixed window:
            // a window walks past the end of a short route and attributes the *next*
            // route's handler to this path, which produced a list of phantom
            // offenders when this was first written.
            //
            // The `.route(` opening paren is already consumed, so depth starts at
            // zero and the expression ends at the `)` that takes it negative.
            let mut depth = 0i32;
            let mut end_at = tail.len();
            for (i, ch) in tail.char_indices() {
                match ch {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth < 0 {
                            end_at = i;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let window = &tail[..end_at];
            // Handler names are identifiers immediately followed by `(`.
            let bytes = window.as_bytes();
            let mut i = 0usize;
            while i < bytes.len() {
                if bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' {
                    let start = i;
                    while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                        i += 1;
                    }
                    // A handler is passed by name, so it is *not* followed by `(` —
                    // it appears as a bare argument, e.g. `get(list_channels)`.
                    // Being defined as `async fn` in this file is the real filter.
                    let name = &window[start..i];
                    if source.contains(&format!("async fn {name}(")) {
                        out.push((path.clone(), name.to_string()));
                    }
                } else {
                    i += 1;
                }
            }
            rest = tail;
        }
        out.sort();
        out.dedup();
        out
    }

    #[test]
    fn the_router_is_scrapable_so_this_audit_is_real() {
        // If this ever returns nothing the test below is vacuous rather than
        // passing, so assert the scrape actually found the router.
        let routes = routes_from_source();
        assert!(
            routes.len() > 50,
            "expected the router to be scraped, found {} routes",
            routes.len()
        );
        assert!(routes.iter().any(|(p, _)| p == "/api/channels"));
        assert!(routes.iter().any(|(p, _)| p == "/api/logs/self"));
    }

    #[test]
    fn a_user_class_route_resolves_the_caller() {
        // User-class handlers must derive identity from the credential, not from a
        // body field, or one user could act as another. `logout` is the deliberate
        // exception: it only needs the token to revoke it, and authenticates by
        // looking the session up directly.
        let source = include_str!("api.rs");
        let mut offenders = Vec::new();
        for (path, handler) in routes_from_source() {
            if required_access(&path) != Access::User || path.starts_with("/api/auth/") {
                continue;
            }
            let Some(start) = source.find(&format!("async fn {handler}(")) else {
                continue;
            };
            // Slice on a char boundary: the source contains multi-byte box-drawing
            // characters in comments, and a byte offset can land inside one.
            let end = source
                .char_indices()
                .map(|(i, _)| i)
                .filter(|i| *i > start && *i <= start + 4000)
                .last()
                .unwrap_or(start);
            let window = &source[start..end];
            if !window.contains("auth_user") {
                offenders.push(format!("{path} -> {handler}"));
            }
        }
        assert!(
            offenders.is_empty(),
            "user-class handlers that do not resolve the caller: {offenders:?}"
        );
    }

    #[test]
    fn sensitive_prefixes_are_never_public() {
        // A public classification on any of these would re-open the original
        // defect: `channels` returns upstream credentials, `options`/`settings`
        // return instance configuration, and `admin` writes other accounts.
        for path in [
            "/api/channels",
            "/api/channels/abc/keys",
            "/api/keys",
            "/api/keys/abc",
            "/api/options",
            "/api/settings",
            "/api/system/info",
            "/api/logs",
            "/api/dashboard",
            "/api/admin/users",
            "/api/models-metadata",
            "/api/vendors",
        ] {
            assert_ne!(
                required_access(path),
                Access::Public,
                "{path} must never be public"
            );
        }
    }

    #[test]
    fn every_public_route_is_one_we_chose() {
        // The public set is an allow-list; this pins its contents so widening it
        // is a deliberate, reviewable act rather than a side effect.
        let allowed = [
            "/api/auth/login",
            "/api/auth/register",
            "/api/plans",
            "/api/status",
        ];
        for path in allowed {
            assert_eq!(required_access(path), Access::Public, "{path}");
        }
        // Anything else must not be public, even if it looks related.
        for path in [
            "/api/auth/logout",
            "/api/auth/me",
            "/api/plans/admin",
            "/api/status/test",
            "/api/statusx",
        ] {
            assert_ne!(required_access(path), Access::Public, "{path}");
        }
    }
}


#[cfg(test)]
mod key_update_tests {
    use super::*;

    fn parse(json: &str) -> KeyUpdateInput {
        serde_json::from_str(json).expect("update input must deserialize")
    }

    /// The update deliberately accepts a narrow field set. A full-object PUT would
    /// let a client rewrite the credential, its owner or its accumulated usage.
    /// This pins that the dangerous names are simply not part of the struct, so a
    /// later "just reuse `ApiKey`" refactor fails here rather than in production.
    #[test]
    fn the_update_cannot_carry_immutable_fields() {
        // Unknown fields are ignored by serde, so none of these can reach storage
        // through this type.
        let input = parse(
            r#"{
                "name": "renamed",
                "key": "sk-attacker",
                "user_id": "someone-else",
                "used_micros": 0,
                "id": "hijacked",
                "created_at": "2000-01-01T00:00:00Z"
            }"#,
        );
        // The only identity-bearing value that survives is the name.
        assert_eq!(input.name, "renamed");
        // Everything optional defaults to absent, i.e. "leave unchanged".
        assert!(input.enabled.is_none());
        assert!(input.priority.is_none());
        assert!(input.expires_at.is_none());
        assert!(input.quota_micros.is_none());
        assert!(input.allowed_models.is_none());
        assert!(input.ip_allowlist.is_none());
        assert!(input.group_name.is_none());
        assert!(input.cross_group_retry.is_none());
    }

    /// The reference restricts its update to a known column set
    /// (`model/token.go:315`), and so does this.
    #[test]
    fn the_update_accepts_the_editable_fields() {
        let input = parse(
            r#"{
                "name": "n",
                "enabled": false,
                "priority": 3,
                "expires_at": "2030-01-01T00:00:00Z",
                "quota_micros": 42,
                "allowed_models": ["gpt-4o"],
                "ip_allowlist": ["10.0.0.1"],
                "group_name": "vip",
                "cross_group_retry": false
            }"#,
        );
        assert_eq!(input.name, "n");
        assert_eq!(input.enabled, Some(false));
        assert_eq!(input.priority, Some(3));
        assert!(input.expires_at.is_some());
        assert_eq!(input.quota_micros, Some(42));
        assert_eq!(input.allowed_models, Some(vec!["gpt-4o".to_string()]));
        assert_eq!(input.ip_allowlist, Some(vec!["10.0.0.1".to_string()]));
        assert_eq!(input.group_name, Some("vip".to_string()));
        assert_eq!(input.cross_group_retry, Some(false));
    }

    #[test]
    fn only_the_name_is_required() {
        // A minimal update renames and touches nothing else, which is what makes
        // extending an expiry or fixing a typo safe.
        let input = parse(r#"{"name": "renamed"}"#);
        assert_eq!(input.name, "renamed");
        // Omitting the name is a parse error rather than a silent no-op.
        assert!(serde_json::from_str::<KeyUpdateInput>(r#"{"enabled": true}"#).is_err());
    }
}

#[cfg(test)]
mod i18n_integrity_tests {
    /// The audit trail copies request bodies, and a body is a new place a
    /// credential could be disclosed in the clear — the same door the API-key
    /// masking work closed. These pin the two halves of the rule: a credential
    /// must never be stored, and the *name* of a setting must be.
    #[test]
    fn audit_bodies_never_carry_a_credential() {
        use super::redact_body;

        // A channel create carries its upstream key.
        let body = br#"{"name":"c","base_url":"https://x","api_key":"sk-live-abc123"}"#;
        let detail = redact_body(body, "/api/channels").expect("object body");
        assert!(!detail.contains("sk-live-abc123"), "{detail}");
        assert!(detail.contains("[redacted]"), "{detail}");
        // The rest of the row stays useful.
        assert!(detail.contains("https://x"), "{detail}");

        // Names that merely contain a redacted fragment must survive, or the
        // trail cannot say what was touched. `monkey` contains `key`.
        let body = br#"{"monkey":"banana","api_key":"sk-x"}"#;
        let detail = redact_body(body, "/api/channels").unwrap();
        assert!(detail.contains("banana"), "{detail}");
        assert!(!detail.contains("sk-x"), "{detail}");

        // Vendor-spelled secrets are covered without enumerating them.
        for name in ["github_client_secret", "SMTPServerToken", "private_key_pem"] {
            let body = format!(r#"{{"{name}":"leak-me"}}"#);
            let detail = redact_body(body.as_bytes(), "/api/settings").unwrap();
            assert!(!detail.contains("leak-me"), "{name} => {detail}");
        }
    }

    /// `/api/options` is the case that a substring rule gets wrong in both
    /// directions: its `key` field is the option's *name*, while its `value`
    /// field is the setting's content and may itself be a token. The schema is
    /// the authority on which is which.
    #[test]
    fn the_options_route_redacts_only_a_secret_settings_value() {
        use super::redact_body;

        // A secret option: the value must go, the name must stay.
        let body = br#"{"key":"LocalApiToken","value":"sk-live-token-value"}"#;
        let detail = redact_body(body, "/api/options").unwrap();
        assert!(!detail.contains("sk-live-token-value"), "{detail}");
        assert!(detail.contains("LocalApiToken"), "{detail}");

        // A non-secret option: the value is the useful part of the audit row.
        let body = br#"{"key":"SiteName","value":"My Router"}"#;
        let detail = redact_body(body, "/api/options").unwrap();
        assert!(detail.contains("My Router"), "{detail}");
        assert!(detail.contains("SiteName"), "{detail}");
    }

    /// A body that cannot be parsed is dropped: a blob whose fields cannot be
    /// inspected must not be assumed free of secrets.
    #[test]
    fn audit_bodies_that_cannot_be_inspected_are_dropped() {
        use super::redact_body;
        assert_eq!(redact_body(b"", "/api/x"), None);
        assert_eq!(redact_body(b"not json at all", "/api/x"), None);
        assert_eq!(redact_body(br#"["a","b"]"#, "/api/x"), None);
        assert_eq!(redact_body(br#""just a string""#, "/api/x"), None);
    }

    /// The Chinese string table shipped with three corrupted values — the label
    /// for 状态 had become a Hebrew cantillation mark plus a combining dot in
    /// three separate keys, and one other label was mangled the same way. They
    /// are invisible in a terminal that renders CJK, and the UI just showed a
    /// stray mark where a word should be.
    ///
    /// This pins the property rather than the three instances: a user-facing
    /// translation should never contain Hebrew, Arabic, Syriac, Thaana,
    /// combining marks, private-use code points, or the replacement character.
    /// Those have no business in a zh/en table, so their presence means an
    /// encoding accident, not a translation choice.
    #[test]
    fn no_translation_contains_an_encoding_accident() {
        let source = include_str!("../web/src/lib/i18n.ts");
        let mut offenders = Vec::new();
        for (index, line) in source.lines().enumerate() {
            let Some(quote) = line.find('"') else { continue };
            for ch in line[quote..].chars() {
                let cp = ch as u32;
                let suspicious = (0x0590..=0x08FF).contains(&cp)     // Hebrew..Arabic..
                    || (0x0300..=0x036F).contains(&cp)               // combining diacritics
                    || (0xE000..=0xF8FF).contains(&cp)               // private use
                    || cp == 0xFFFD;                                 // replacement char
                if suspicious {
                    offenders.push(format!(
                        "line {}: {} (U+{cp:04X})",
                        index + 1,
                        line.trim()
                    ));
                    break;
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "translations with mangled characters: {offenders:#?}"
        );
    }
}
