//! SQLite database layer for OxygenRouter
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use argon2::password_hash::SaltString;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use chrono::{DateTime, Duration, Utc};
use parking_lot::Mutex;
use rand_core::OsRng;
use rusqlite::{params, Connection, OptionalExtension, Result as SqliteResult};
use std::path::Path;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiKeyResolutionError {
    Invalid,
    Disabled,
    Expired,
    ModelDisallowed,
    IpDisallowed,
    QuotaExceeded,
}

use super::{
    AnalyticsFilters, AnalyticsFlow, AnalyticsFlowLink, AnalyticsFlowNode, ApiKey, ApiKeyUsage,
    AuthSession, Channel, ChannelInfo, ChannelPerf, DashboardBreakdown, DashboardSnapshot,
    LedgerEntry, ModelMap, ModelMetadata, ModelTimeBucket, PaymentOrder, RedemptionCode, RequestLog,
    RouteRule, RouteType, Subscription, SubscriptionPlan, SystemStatus, TimeBucket, User, UserRole,
    Vendor,
};

fn json_string<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "{}".to_string())
}
fn json_value<T: serde::de::DeserializeOwned + Default>(value: &str) -> T {
    serde_json::from_str(value).unwrap_or_default()
}

fn map_api_key(row: &rusqlite::Row<'_>) -> SqliteResult<ApiKey> {
    Ok(ApiKey {
        id: row.get(0)?,
        key: row.get(1)?,
        name: row.get(2)?,
        priority: row.get(3)?,
        enabled: row.get::<_, i32>(4)? != 0,
        created_at: DateTime::parse_from_rfc3339(&row.get::<_, String>(5)?)
            .unwrap()
            .with_timezone(&Utc),
        expires_at: optional_datetime(row.get(6)?),
        quota_micros: row.get(7)?,
        used_micros: row.get(8)?,
        allowed_models: json_value(&row.get::<_, String>(9)?),
        ip_allowlist: json_value(&row.get::<_, String>(10)?),
        group_name: row.get(11)?,
        cross_group_retry: row.get::<_, i32>(12)? != 0,
        user_id: row.get(13)?,
    })
}

fn map_model_metadata(row: &rusqlite::Row<'_>) -> SqliteResult<ModelMetadata> {    Ok(ModelMetadata {
        id: row.get(0)?,
        model_name: row.get(1)?,
        description: row.get(2)?,
        icon: row.get(3)?,
        tags: row.get(4)?,
        vendor: row.get(5)?,
        endpoints: json_value(&row.get::<_, String>(6)?),
        name_rule: row.get(7)?,
        status: row.get(8)?,
        sync_official: row.get(9)?,
        created_at: DateTime::parse_from_rfc3339(&row.get::<_, String>(10)?)
            .unwrap()
            .with_timezone(&Utc),
        updated_at: DateTime::parse_from_rfc3339(&row.get::<_, String>(11)?)
            .unwrap()
            .with_timezone(&Utc),
    })
}
fn map_vendor(row: &rusqlite::Row<'_>) -> SqliteResult<Vendor> {
    Ok(Vendor {
        id: row.get(0)?,
        name: row.get(1)?,
        description: row.get(2)?,
        icon: row.get(3)?,
        status: row.get(4)?,
        // Filled in by the caller when it joins the model registry; the table
        // itself does not store a count, matching the reference.
        model_count: 0,
        created_at: DateTime::parse_from_rfc3339(&row.get::<_, String>(5)?)
            .unwrap()
            .with_timezone(&Utc),
        updated_at: DateTime::parse_from_rfc3339(&row.get::<_, String>(6)?)
            .unwrap()
            .with_timezone(&Utc),
    })
}

/// Columns every vendor query selects, in the order [`map_vendor`] reads them.
const VENDOR_COLUMNS: &str = "id,name,description,icon,status,created_at,updated_at";

fn json_object(value: &str) -> serde_json::Value {
    serde_json::from_str(value)
        .ok()
        .filter(serde_json::Value::is_object)
        .unwrap_or_else(|| serde_json::json!({}))
}
fn optional_datetime(value: Option<String>) -> Option<DateTime<Utc>> {
    value
        .and_then(|value| DateTime::parse_from_rfc3339(&value).ok())
        .map(|value| value.with_timezone(&Utc))
}
fn glob_match(pattern: &str, input: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == input;
    }
    let mut remainder = input;
    for (index, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        if index == 0 && !pattern.starts_with('*') {
            if !remainder.starts_with(part) {
                return false;
            }
            remainder = &remainder[part.len()..];
        } else if index == parts.len() - 1 && !pattern.ends_with('*') {
            return remainder.ends_with(part);
        } else if let Some(position) = remainder.find(part) {
            remainder = &remainder[position + part.len()..];
        } else {
            return false;
        }
    }
    true
}

pub struct Database {
    conn: Mutex<Connection>,
}

impl Database {
    pub fn new<P: AsRef<Path>>(path: P) -> SqliteResult<Self> {
        let conn = Connection::open(path)?;
        let db = Self {
            conn: Mutex::new(conn),
        };
        db.init_schema()?;
        Ok(db)
    }

    fn init_schema(&self) -> SqliteResult<()> {
        let conn = self.conn.lock();
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS channels (
                id          TEXT PRIMARY KEY,
                name        TEXT NOT NULL,
                provider    TEXT NOT NULL DEFAULT 'openai',
                base_url    TEXT NOT NULL,
                api_key     TEXT NOT NULL,
                priority    INTEGER NOT NULL DEFAULT 0,
                weight      INTEGER NOT NULL DEFAULT 1,
                enabled     INTEGER NOT NULL DEFAULT 1,
                test_model  TEXT NOT NULL DEFAULT 'gpt-3.5-turbo',
                created_at  TEXT NOT NULL,
                updated_at  TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS api_keys (
                id         TEXT PRIMARY KEY,
                key        TEXT NOT NULL UNIQUE,
                name       TEXT NOT NULL,
                priority   INTEGER NOT NULL DEFAULT 0,
                enabled    INTEGER NOT NULL DEFAULT 1,
                created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS model_maps (
                id          TEXT PRIMARY KEY,
                channel_id  TEXT NOT NULL,
                pattern     TEXT NOT NULL,
                target_model TEXT NOT NULL,
                enabled     INTEGER NOT NULL DEFAULT 1,
                created_at  TEXT NOT NULL,
                FOREIGN KEY (channel_id) REFERENCES channels(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS model_metadata (
                id            TEXT PRIMARY KEY,
                model_name    TEXT NOT NULL UNIQUE,
                description   TEXT NOT NULL DEFAULT '',
                icon          TEXT NOT NULL DEFAULT '',
                tags          TEXT NOT NULL DEFAULT '',
                vendor        TEXT NOT NULL DEFAULT '',
                endpoints     TEXT NOT NULL DEFAULT '[]',
                name_rule     INTEGER NOT NULL DEFAULT 0,
                status        INTEGER NOT NULL DEFAULT 1,
                sync_official INTEGER NOT NULL DEFAULT 1,
                created_at    TEXT NOT NULL,
                updated_at    TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS route_rules (
                id          TEXT PRIMARY KEY,
                name        TEXT NOT NULL,
                rule_type   TEXT NOT NULL,
                priority    INTEGER NOT NULL DEFAULT 0,
                enabled     INTEGER NOT NULL DEFAULT 1,
                config      TEXT NOT NULL DEFAULT '{}',
                created_at  TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS request_logs (
                id           TEXT PRIMARY KEY,
                method       TEXT NOT NULL,
                path         TEXT NOT NULL,
                model        TEXT,
                channel_id   TEXT,
                api_key_id   TEXT,
                status_code  INTEGER,
                error        TEXT,
                tokens_used  INTEGER,
                duration_ms  INTEGER NOT NULL,
                created_at   TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS settings (
                key   TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS migrations (
                version TEXT PRIMARY KEY,
                applied_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS vendors (
                id          TEXT PRIMARY KEY,
                name        TEXT NOT NULL UNIQUE,
                description TEXT NOT NULL DEFAULT '',
                icon        TEXT NOT NULL DEFAULT '',
                status      INTEGER NOT NULL DEFAULT 1,
                created_at  TEXT NOT NULL,
                updated_at  TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_channels_priority ON channels(priority DESC);
            CREATE INDEX IF NOT EXISTS idx_model_maps_channel ON model_maps(channel_id);
            CREATE INDEX IF NOT EXISTS idx_request_logs_created ON request_logs(created_at DESC);

            CREATE TABLE IF NOT EXISTS users (
                id TEXT PRIMARY KEY, username TEXT NOT NULL UNIQUE, email TEXT NOT NULL UNIQUE,
                password_hash TEXT NOT NULL, role TEXT NOT NULL DEFAULT 'user', status TEXT NOT NULL DEFAULT 'active',
                balance_micros INTEGER NOT NULL DEFAULT 0, created_at TEXT NOT NULL, updated_at TEXT NOT NULL, last_login_at TEXT
            );
            CREATE TABLE IF NOT EXISTS auth_sessions (
                id TEXT PRIMARY KEY, user_id TEXT NOT NULL, token TEXT NOT NULL UNIQUE, expires_at TEXT NOT NULL, created_at TEXT NOT NULL,
                FOREIGN KEY(user_id) REFERENCES users(id) ON DELETE CASCADE
            );
            CREATE TABLE IF NOT EXISTS ledger_entries (
                id TEXT PRIMARY KEY, user_id TEXT NOT NULL, amount_micros INTEGER NOT NULL, balance_after_micros INTEGER NOT NULL,
                kind TEXT NOT NULL, description TEXT NOT NULL, reference_id TEXT, created_at TEXT NOT NULL,
                FOREIGN KEY(user_id) REFERENCES users(id) ON DELETE CASCADE
            );
            CREATE TABLE IF NOT EXISTS subscription_plans (
                id TEXT PRIMARY KEY, name TEXT NOT NULL, description TEXT NOT NULL, price_micros INTEGER NOT NULL,
                quota_micros INTEGER NOT NULL, duration_days INTEGER NOT NULL, enabled INTEGER NOT NULL DEFAULT 1,
                created_at TEXT NOT NULL, updated_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS subscriptions (
                id TEXT PRIMARY KEY, user_id TEXT NOT NULL, plan_id TEXT NOT NULL, status TEXT NOT NULL,
                started_at TEXT NOT NULL, expires_at TEXT NOT NULL, created_at TEXT NOT NULL,
                FOREIGN KEY(user_id) REFERENCES users(id) ON DELETE CASCADE, FOREIGN KEY(plan_id) REFERENCES subscription_plans(id)
            );
            CREATE TABLE IF NOT EXISTS redemption_codes (
                id TEXT PRIMARY KEY, code TEXT NOT NULL UNIQUE, amount_micros INTEGER NOT NULL, plan_id TEXT,
                enabled INTEGER NOT NULL DEFAULT 1, max_uses INTEGER NOT NULL, used_count INTEGER NOT NULL DEFAULT 0,
                expires_at TEXT, created_at TEXT NOT NULL, FOREIGN KEY(plan_id) REFERENCES subscription_plans(id)
            );
            CREATE TABLE IF NOT EXISTS redemption_uses (
                redemption_code_id TEXT NOT NULL, user_id TEXT NOT NULL, used_at TEXT NOT NULL,
                PRIMARY KEY(redemption_code_id, user_id), FOREIGN KEY(redemption_code_id) REFERENCES redemption_codes(id) ON DELETE CASCADE,
                FOREIGN KEY(user_id) REFERENCES users(id) ON DELETE CASCADE
            );
            CREATE TABLE IF NOT EXISTS payment_orders (
                id TEXT PRIMARY KEY, user_id TEXT NOT NULL, amount_micros INTEGER NOT NULL, provider TEXT NOT NULL,
                status TEXT NOT NULL, external_reference TEXT, created_at TEXT NOT NULL, updated_at TEXT NOT NULL,
                FOREIGN KEY(user_id) REFERENCES users(id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_sessions_token ON auth_sessions(token);
            CREATE INDEX IF NOT EXISTS idx_ledger_user_created ON ledger_entries(user_id, created_at DESC);
            CREATE INDEX IF NOT EXISTS idx_subscriptions_user ON subscriptions(user_id, expires_at DESC);
            CREATE INDEX IF NOT EXISTS idx_orders_user ON payment_orders(user_id, created_at DESC);
            "#,
        )?;
        // Migration: add provider column to existing channels table (idempotent)
        let _ = conn.execute(
            "ALTER TABLE channels ADD COLUMN provider TEXT NOT NULL DEFAULT 'openai'",
            [],
        );
        let _ = conn.execute("ALTER TABLE request_logs ADD COLUMN api_key_id TEXT", []);
        for sql in [
            "ALTER TABLE api_keys ADD COLUMN expires_at TEXT",
            "ALTER TABLE api_keys ADD COLUMN quota_micros INTEGER NOT NULL DEFAULT 0",
            "ALTER TABLE api_keys ADD COLUMN used_micros INTEGER NOT NULL DEFAULT 0",
            "ALTER TABLE api_keys ADD COLUMN allowed_models TEXT NOT NULL DEFAULT '[]'",
            "ALTER TABLE api_keys ADD COLUMN ip_allowlist TEXT NOT NULL DEFAULT '[]'",
            "ALTER TABLE api_keys ADD COLUMN group_name TEXT NOT NULL DEFAULT 'default'",
            "ALTER TABLE api_keys ADD COLUMN cross_group_retry INTEGER NOT NULL DEFAULT 1",
            // Billing needs to know whose wallet pays for a token. NewAPI binds
            // every token to a user row; an empty value means the key is not
            // wallet-backed (channel-only / pre-auth deployments).
            "ALTER TABLE api_keys ADD COLUMN user_id TEXT NOT NULL DEFAULT ''",
            "ALTER TABLE channels ADD COLUMN group_name TEXT NOT NULL DEFAULT 'default'",
            "ALTER TABLE channels ADD COLUMN tags TEXT NOT NULL DEFAULT '[]'",
            "ALTER TABLE channels ADD COLUMN model_list TEXT NOT NULL DEFAULT '[]'",
            "ALTER TABLE channels ADD COLUMN response_headers TEXT NOT NULL DEFAULT '{}'",
            "ALTER TABLE channels ADD COLUMN status_code_mapping TEXT NOT NULL DEFAULT '{}'",
            "ALTER TABLE channels ADD COLUMN override_parameters TEXT NOT NULL DEFAULT '{}'",
            "ALTER TABLE channels ADD COLUMN balance_micros INTEGER NOT NULL DEFAULT 0",
            "ALTER TABLE channels ADD COLUMN last_test_at TEXT",
            "ALTER TABLE channels ADD COLUMN info TEXT NOT NULL DEFAULT '{}'",
        ] {
            let _ = conn.execute(sql, []);
        }
        Ok(())
    }

    // ── Channels ──────────────────────────────────────────────────────────────

    pub fn upsert_channel(&self, ch: &Channel) -> SqliteResult<()> {
        let conn = self.conn.lock();
        conn.execute(
            r#"INSERT INTO channels (id,name,provider,base_url,api_key,priority,weight,enabled,test_model,group_name,tags,model_list,response_headers,status_code_mapping,override_parameters,balance_micros,last_test_at,info,created_at,updated_at)
               VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20)
               ON CONFLICT(id) DO UPDATE SET
                   name=?2, provider=?3, base_url=?4, api_key=?5, priority=?6, weight=?7,
                   enabled=?8, test_model=?9, group_name=?10, tags=?11, model_list=?12, response_headers=?13, status_code_mapping=?14, override_parameters=?15, balance_micros=?16, last_test_at=?17, info=?18, updated_at=?20"#,
            params![
                ch.id, ch.name, ch.provider, ch.base_url, ch.api_key, ch.priority, ch.weight,
                ch.enabled as i32, ch.test_model, ch.group_name, json_string(&ch.tags), json_string(&ch.model_list), json_string(&ch.response_headers), json_string(&ch.status_code_mapping), json_string(&ch.override_parameters), ch.balance_micros, ch.last_test_at.map(|v| v.to_rfc3339()), json_string(&ch.info), ch.created_at.to_rfc3339(), ch.updated_at.to_rfc3339()
            ],
        )?;
        Ok(())
    }

    pub fn list_channels(&self) -> SqliteResult<Vec<Channel>> {
        let conn = self.conn.lock();
        let mut s = conn.prepare(
            "SELECT id,name,provider,base_url,api_key,priority,weight,enabled,test_model,group_name,tags,model_list,response_headers,status_code_mapping,override_parameters,balance_micros,last_test_at,info,created_at,updated_at
             FROM channels ORDER BY priority DESC, weight DESC",
        )?;
        let rows = s.query_map([], |r| {
            Ok(Channel {
                id: r.get(0)?,
                name: r.get(1)?,
                provider: r.get(2)?,
                base_url: r.get(3)?,
                api_key: r.get(4)?,
                priority: r.get(5)?,
                weight: r.get(6)?,
                enabled: r.get::<_, i32>(7)? != 0,
                test_model: r.get(8)?,
                group_name: r.get(9)?,
                tags: json_value(&r.get::<_, String>(10)?),
                model_list: json_value(&r.get::<_, String>(11)?),
                response_headers: json_object(&r.get::<_, String>(12)?),
                status_code_mapping: json_object(&r.get::<_, String>(13)?),
                override_parameters: json_object(&r.get::<_, String>(14)?),
                balance_micros: r.get(15)?,
                last_test_at: optional_datetime(r.get(16)?),
                info: json_value(&r.get::<_, String>(17)?),
                created_at: DateTime::parse_from_rfc3339(&r.get::<_, String>(18)?)
                    .unwrap()
                    .with_timezone(&Utc),
                updated_at: DateTime::parse_from_rfc3339(&r.get::<_, String>(19)?)
                    .unwrap()
                    .with_timezone(&Utc),
            })
        })?;
        rows.collect()
    }

    pub fn get_channel(&self, id: &str) -> SqliteResult<Option<Channel>> {
        let conn = self.conn.lock();
        let mut s = conn.prepare(
            "SELECT id,name,provider,base_url,api_key,priority,weight,enabled,test_model,group_name,tags,model_list,response_headers,status_code_mapping,override_parameters,balance_micros,last_test_at,info,created_at,updated_at
             FROM channels WHERE id=?1",
        )?;
        let mut rows = s.query(params![id])?;
        if let Some(r) = rows.next()? {
            Ok(Some(Channel {
                id: r.get(0)?,
                name: r.get(1)?,
                provider: r.get(2)?,
                base_url: r.get(3)?,
                api_key: r.get(4)?,
                priority: r.get(5)?,
                weight: r.get(6)?,
                enabled: r.get::<_, i32>(7)? != 0,
                test_model: r.get(8)?,
                group_name: r.get(9)?,
                tags: json_value(&r.get::<_, String>(10)?),
                model_list: json_value(&r.get::<_, String>(11)?),
                response_headers: json_object(&r.get::<_, String>(12)?),
                status_code_mapping: json_object(&r.get::<_, String>(13)?),
                override_parameters: json_object(&r.get::<_, String>(14)?),
                balance_micros: r.get(15)?,
                last_test_at: optional_datetime(r.get(16)?),
                info: json_value(&r.get::<_, String>(17)?),
                created_at: DateTime::parse_from_rfc3339(&r.get::<_, String>(18)?)
                    .unwrap()
                    .with_timezone(&Utc),
                updated_at: DateTime::parse_from_rfc3339(&r.get::<_, String>(19)?)
                    .unwrap()
                    .with_timezone(&Utc),
            }))
        } else {
            Ok(None)
        }
    }

    pub fn delete_channel(&self, id: &str) -> SqliteResult<()> {
        self.conn
            .lock()
            .execute("DELETE FROM channels WHERE id=?1", params![id])?;
        Ok(())
    }

    pub fn update_channel_models(&self, id: &str, models: &[String]) -> SqliteResult<()> {
        self.conn.lock().execute(
            "UPDATE channels SET model_list=?1, updated_at=?2 WHERE id=?3",
            params![json_string(&models.to_vec()), Utc::now().to_rfc3339(), id],
        )?;
        Ok(())
    }

    /// Persist per-key status bookkeeping (multi-key health tracking).
    pub fn update_channel_info(&self, id: &str, info: &ChannelInfo) -> SqliteResult<()> {
        self.conn.lock().execute(
            "UPDATE channels SET info=?1, updated_at=?2 WHERE id=?3",
            params![json_string(info), Utc::now().to_rfc3339(), id],
        )?;
        Ok(())
    }

    /// Replace the whole key list (newline-joined) plus refresh multi-key bookkeeping.
    pub fn replace_channel_keys(&self, id: &str, keys: &[String], info: &ChannelInfo) -> SqliteResult<()> {
        self.conn.lock().execute(
            "UPDATE channels SET api_key=?1, info=?2, updated_at=?3 WHERE id=?4",
            params![keys.join("\n"), json_string(info), Utc::now().to_rfc3339(), id],
        )?;
        Ok(())
    }

    pub fn batch_update_channels(&self, ids: &[String], enabled: bool) -> SqliteResult<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let conn = self.conn.lock();
        let now = chrono::Utc::now().to_rfc3339();
        let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!(
            "UPDATE channels SET enabled=?1, updated_at=?2 WHERE id IN ({})",
            placeholders
        );
        let mut params_vec: Vec<&dyn rusqlite::ToSql> = Vec::with_capacity(2 + ids.len());
        params_vec.push(&enabled);
        params_vec.push(&now);
        for id in ids {
            params_vec.push(id);
        }
        conn.execute(&sql, params_vec.as_slice())?;
        Ok(())
    }

    pub fn batch_delete_channels(&self, ids: &[String]) -> SqliteResult<usize> {
        if ids.is_empty() {
            return Ok(0);
        }
        let conn = self.conn.lock();
        let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!("DELETE FROM channels WHERE id IN ({})", placeholders);
        let mut params_vec: Vec<&dyn rusqlite::ToSql> = Vec::with_capacity(ids.len());
        for id in ids {
            params_vec.push(id);
        }
        let n = conn.execute(&sql, params_vec.as_slice())?;
        Ok(n)
    }

    pub fn get_enabled_channels(&self) -> SqliteResult<Vec<Channel>> {
        let conn = self.conn.lock();
        let mut s = conn.prepare(
            "SELECT id,name,provider,base_url,api_key,priority,weight,enabled,test_model,group_name,tags,model_list,response_headers,status_code_mapping,override_parameters,balance_micros,last_test_at,info,created_at,updated_at
             FROM channels WHERE enabled=1 ORDER BY priority DESC, weight DESC",
        )?;
        let rows = s.query_map([], |r| {
            Ok(Channel {
                id: r.get(0)?,
                name: r.get(1)?,
                provider: r.get(2)?,
                base_url: r.get(3)?,
                api_key: r.get(4)?,
                priority: r.get(5)?,
                weight: r.get(6)?,
                enabled: r.get::<_, i32>(7)? != 0,
                test_model: r.get(8)?,
                group_name: r.get(9)?,
                tags: json_value(&r.get::<_, String>(10)?),
                model_list: json_value(&r.get::<_, String>(11)?),
                response_headers: json_object(&r.get::<_, String>(12)?),
                status_code_mapping: json_object(&r.get::<_, String>(13)?),
                override_parameters: json_object(&r.get::<_, String>(14)?),
                balance_micros: r.get(15)?,
                last_test_at: optional_datetime(r.get(16)?),
                info: json_value(&r.get::<_, String>(17)?),
                created_at: DateTime::parse_from_rfc3339(&r.get::<_, String>(18)?)
                    .unwrap()
                    .with_timezone(&Utc),
                updated_at: DateTime::parse_from_rfc3339(&r.get::<_, String>(19)?)
                    .unwrap()
                    .with_timezone(&Utc),
            })
        })?;
        rows.collect()
    }

    // ── Model Maps ───────────────────────────────────────────────────────────

    pub fn upsert_model_map(&self, m: &ModelMap) -> SqliteResult<()> {
        self.conn.lock().execute(
            r#"INSERT INTO model_maps (id,channel_id,pattern,target_model,enabled,created_at)
               VALUES (?1,?2,?3,?4,?5,?6)
               ON CONFLICT(id) DO UPDATE SET
                   channel_id=?2, pattern=?3, target_model=?4, enabled=?5"#,
            params![
                m.id,
                m.channel_id,
                m.pattern,
                m.target_model,
                m.enabled as i32,
                m.created_at.to_rfc3339()
            ],
        )?;
        Ok(())
    }

    pub fn list_model_maps(&self) -> SqliteResult<Vec<ModelMap>> {
        let conn = self.conn.lock();
        let mut s = conn.prepare(
            "SELECT id,channel_id,pattern,target_model,enabled,created_at FROM model_maps ORDER BY channel_id",
        )?;
        let rows = s.query_map([], |r| {
            Ok(ModelMap {
                id: r.get(0)?,
                channel_id: r.get(1)?,
                pattern: r.get(2)?,
                target_model: r.get(3)?,
                enabled: r.get::<_, i32>(4)? != 0,
                created_at: DateTime::parse_from_rfc3339(&r.get::<_, String>(5)?)
                    .unwrap()
                    .with_timezone(&Utc),
            })
        })?;
        rows.collect()
    }

    pub fn delete_model_map(&self, id: &str) -> SqliteResult<()> {
        self.conn
            .lock()
            .execute("DELETE FROM model_maps WHERE id=?1", params![id])?;
        Ok(())
    }

    // ── Route Rules ───────────────────────────────────────────────────────────

    pub fn upsert_route_rule(&self, r: &RouteRule) -> SqliteResult<()> {
        let config_str = serde_json::to_string(&r.config).unwrap_or_default();
        self.conn.lock().execute(
            r#"INSERT INTO route_rules (id,name,rule_type,priority,enabled,config,created_at)
               VALUES (?1,?2,?3,?4,?5,?6,?7)
               ON CONFLICT(id) DO UPDATE SET
                   name=?2, rule_type=?3, priority=?4, enabled=?5, config=?6"#,
            params![
                r.id,
                r.name,
                serde_json::to_string(&r.rule_type).unwrap(),
                r.priority,
                r.enabled as i32,
                config_str,
                r.created_at.to_rfc3339()
            ],
        )?;
        Ok(())
    }

    pub fn list_route_rules(&self) -> SqliteResult<Vec<RouteRule>> {
        let conn = self.conn.lock();
        let mut s = conn.prepare(
            "SELECT id,name,rule_type,priority,enabled,config,created_at FROM route_rules ORDER BY priority DESC",
        )?;
        let rows = s.query_map([], |r| {
            let rt: String = r.get(2)?;
            let rule_type =
                serde_json::from_str(&format!("\"{}\"", rt)).unwrap_or(RouteType::AutoHeuristic);
            let config_str: String = r.get(5)?;
            let config = serde_json::from_str(&config_str).unwrap_or_default();
            Ok(RouteRule {
                id: r.get(0)?,
                name: r.get(1)?,
                rule_type,
                priority: r.get(3)?,
                enabled: r.get::<_, i32>(4)? != 0,
                config,
                created_at: DateTime::parse_from_rfc3339(&r.get::<_, String>(6)?)
                    .unwrap()
                    .with_timezone(&Utc),
            })
        })?;
        rows.collect()
    }

    pub fn delete_route_rule(&self, id: &str) -> SqliteResult<()> {
        self.conn
            .lock()
            .execute("DELETE FROM route_rules WHERE id=?1", params![id])?;
        Ok(())
    }

    // ── API Keys ──────────────────────────────────────────────────────────────

    pub fn upsert_api_key(&self, k: &ApiKey) -> SqliteResult<()> {
        self.conn.lock().execute(
            r#"INSERT INTO api_keys (id,key,name,priority,enabled,created_at,expires_at,quota_micros,used_micros,allowed_models,ip_allowlist,group_name,cross_group_retry,user_id)
               VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)
               ON CONFLICT(id) DO UPDATE SET
                    key=?2, name=?3, priority=?4, enabled=?5, expires_at=?7, quota_micros=?8, allowed_models=?10, ip_allowlist=?11, group_name=?12, cross_group_retry=?13, user_id=?14"#,
            params![
                k.id,
                k.key,
                k.name,
                k.priority,
                k.enabled as i32,
                k.created_at.to_rfc3339(), k.expires_at.map(|v| v.to_rfc3339()), k.quota_micros, k.used_micros, json_string(&k.allowed_models), json_string(&k.ip_allowlist), k.group_name, k.cross_group_retry as i32, k.user_id
            ],
        )?;
        Ok(())
    }

    /// Server-side paginated + searchable API key list.
    pub fn list_api_keys_paged(
        &self,
        page: i64,
        page_size: i64,
        search: Option<&str>,
    ) -> SqliteResult<(Vec<ApiKey>, i64)> {
        let conn = self.conn.lock();
        let where_clause = "WHERE (?1 IS NULL OR name LIKE '%' || ?1 || '%' OR key LIKE '%' || ?1 || '%')";
        let total: i64 = conn.query_row(
            &format!("SELECT COUNT(*) FROM api_keys {where_clause}"),
            params![search],
            |r| r.get(0),
        )?;
        let offset = (page.max(1) - 1) * page_size.max(1);
        let mut s = conn.prepare(&format!(
            "SELECT id,key,name,priority,enabled,created_at,expires_at,quota_micros,used_micros,allowed_models,ip_allowlist,group_name,cross_group_retry,user_id
             FROM api_keys {where_clause} ORDER BY priority DESC, created_at DESC LIMIT ?2 OFFSET ?3"
        ))?;
        let rows = s.query_map(params![search, page_size.max(1), offset], map_api_key)?;
        let items = rows.collect::<SqliteResult<Vec<_>>>()?;
        Ok((items, total))
    }

    pub fn list_api_keys(&self) -> SqliteResult<Vec<ApiKey>> {
        let conn = self.conn.lock();
        let mut s = conn.prepare(
            "SELECT id,key,name,priority,enabled,created_at,expires_at,quota_micros,used_micros,allowed_models,ip_allowlist,group_name,cross_group_retry,user_id FROM api_keys ORDER BY priority DESC",
        )?;
        let rows = s.query_map([], map_api_key)?;
        rows.collect()
    }

    pub fn find_api_key_by_token(&self, token: &str) -> SqliteResult<Option<ApiKey>> {
        let conn = self.conn.lock();
        let mut s = conn.prepare(
            "SELECT id,key,name,priority,enabled,created_at,expires_at,quota_micros,used_micros,allowed_models,ip_allowlist,group_name,cross_group_retry,user_id FROM api_keys WHERE key=?1 AND enabled=1 LIMIT 1",
        )?;
        let mut rows = s.query(params![token])?;
        if let Some(r) = rows.next()? {
            Ok(Some(ApiKey {
                id: r.get(0)?,
                key: r.get(1)?,
                name: r.get(2)?,
                priority: r.get(3)?,
                enabled: r.get::<_, i32>(4)? != 0,
                created_at: DateTime::parse_from_rfc3339(&r.get::<_, String>(5)?)
                    .unwrap()
                    .with_timezone(&Utc),
                expires_at: optional_datetime(r.get(6)?),
                quota_micros: r.get(7)?,
                used_micros: r.get(8)?,
                allowed_models: json_value(&r.get::<_, String>(9)?),
                ip_allowlist: json_value(&r.get::<_, String>(10)?),
                group_name: r.get(11)?,
                cross_group_retry: r.get::<_, i32>(12)? != 0,
                user_id: r.get(13)?,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn has_active_api_keys(&self) -> SqliteResult<bool> {
        Ok(self.conn.lock().query_row(
            "SELECT EXISTS(SELECT 1 FROM api_keys WHERE enabled=1)",
            [],
            |r| r.get::<_, i32>(0),
        )? != 0)
    }

    pub fn resolve_api_key(
        &self,
        token: &str,
        model: &str,
        remote_ip: &str,
    ) -> Result<ApiKey, ApiKeyResolutionError> {
        let key = self
            .list_api_keys()
            .map_err(|_| ApiKeyResolutionError::Invalid)?
            .into_iter()
            .find(|k| k.key == token)
            .ok_or(ApiKeyResolutionError::Invalid)?;
        if !key.enabled {
            return Err(ApiKeyResolutionError::Disabled);
        }
        if key.expires_at.is_some_and(|expires| expires <= Utc::now()) {
            return Err(ApiKeyResolutionError::Expired);
        }
        if key.quota_micros > 0 && key.used_micros >= key.quota_micros {
            return Err(ApiKeyResolutionError::QuotaExceeded);
        }
        if !key.allowed_models.is_empty()
            && !key
                .allowed_models
                .iter()
                .any(|pattern| glob_match(pattern, model))
        {
            return Err(ApiKeyResolutionError::ModelDisallowed);
        }
        if !key.ip_allowlist.is_empty()
            && !key
                .ip_allowlist
                .iter()
                .any(|ip| ip == "*" || ip == remote_ip)
        {
            return Err(ApiKeyResolutionError::IpDisallowed);
        }
        Ok(key)
    }

    pub fn record_api_key_usage(
        &self,
        key_id: &str,
        amount_micros: i64,
    ) -> Result<(), ApiKeyResolutionError> {
        let changed = self.conn.lock().execute("UPDATE api_keys SET used_micros=used_micros+?2 WHERE id=?1 AND (quota_micros=0 OR used_micros+?2<=quota_micros)", params![key_id, amount_micros]).map_err(|_| ApiKeyResolutionError::Invalid)?;
        if changed == 0 {
            Err(ApiKeyResolutionError::QuotaExceeded)
        } else {
            Ok(())
        }
    }

    /// Get per-key usage statistics
    pub fn api_key_usage(&self) -> SqliteResult<Vec<ApiKeyUsage>> {
        let conn = self.conn.lock();
        let mut s = conn.prepare(
            "SELECT
                COALESCE(k.id, '') AS id,
                COALESCE(k.name, 'Anonymous') AS name,
                COALESCE(k.enabled, 1) AS enabled,
                COUNT(l.id) AS total_requests,
                SUM(CASE WHEN l.status_code >= 200 AND l.status_code < 300 THEN 1 ELSE 0 END) AS successful,
                COALESCE(SUM(l.tokens_used), 0) AS total_tokens,
                COALESCE(AVG(l.duration_ms), 0) AS avg_latency
             FROM api_keys k
             LEFT JOIN request_logs l ON l.api_key_id = k.id
             GROUP BY k.id
             ORDER BY total_requests DESC"
        )?;
        let rows = s.query_map([], |r| {
            Ok(ApiKeyUsage {
                api_key_id: r.get(0)?,
                api_key_name: r.get(1)?,
                enabled: r.get::<_, i32>(2)? != 0,
                total_requests: r.get(3)?,
                successful_requests: r.get(4)?,
                total_tokens: r.get(5)?,
                average_latency_ms: r.get(6)?,
            })
        })?;
        rows.collect()
    }

    pub fn delete_api_key(&self, id: &str) -> SqliteResult<()> {
        self.conn
            .lock()
            .execute("DELETE FROM api_keys WHERE id=?1", params![id])?;
        Ok(())
    }

    // ── Request Logs ──────────────────────────────────────────────────────────

    pub fn insert_request_log(&self, log: &RequestLog) -> SqliteResult<()> {
        self.conn.lock().execute(
            r#"INSERT INTO request_logs (id,method,path,model,channel_id,api_key_id,status_code,error,tokens_used,duration_ms,created_at)
               VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)"#,
            params![
                log.id, log.method, log.path, log.model, log.channel_id, log.api_key_id,
                log.status_code, log.error, log.tokens_used, log.duration_ms, log.created_at.to_rfc3339()
            ],
        )?;
        Ok(())
    }

    pub fn list_request_logs(&self, limit: i64) -> SqliteResult<Vec<RequestLog>> {
        let conn = self.conn.lock();
        let mut s = conn.prepare(
            "SELECT id,method,path,model,channel_id,api_key_id,status_code,error,tokens_used,duration_ms,created_at
             FROM request_logs ORDER BY created_at DESC LIMIT ?1",
        )?;
        let rows = s.query_map(params![limit], |r| {
            Ok(RequestLog {
                id: r.get(0)?,
                method: r.get(1)?,
                path: r.get(2)?,
                model: r.get(3)?,
                channel_id: r.get(4)?,
                api_key_id: r.get(5)?,
                status_code: r.get(6)?,
                error: r.get(7)?,
                tokens_used: r.get(8)?,
                duration_ms: r.get(9)?,
                created_at: DateTime::parse_from_rfc3339(&r.get::<_, String>(10)?)
                    .unwrap()
                    .with_timezone(&Utc),
            })
        })?;
        rows.collect()
    }

    /// Server-side paginated + filtered log query (NewAPI parity).
    pub fn query_request_logs(
        &self,
        page: i64,
        page_size: i64,
        start: Option<&str>,
        end: Option<&str>,
        model: Option<&str>,
        channel_id: Option<&str>,
        api_key_id: Option<&str>,
        status: Option<&str>,
        search: Option<&str>,
    ) -> SqliteResult<(Vec<RequestLog>, i64)> {
        let conn = self.conn.lock();
        let where_clause = "WHERE (?1 IS NULL OR created_at >= ?1) \
             AND (?2 IS NULL OR created_at < ?2) \
             AND (?3 IS NULL OR COALESCE(model, 'unknown') = ?3) \
             AND (?4 IS NULL OR COALESCE(channel_id, 'unassigned') = ?4) \
             AND (?5 IS NULL OR COALESCE(api_key_id, 'unassigned') = ?5) \
             AND (?6 IS NULL OR CASE ?6 \
                   WHEN 'success' THEN status_code >= 200 AND status_code < 300 \
                   WHEN 'error' THEN status_code < 200 OR status_code >= 400 \
                   ELSE CAST(status_code AS TEXT) LIKE ?6 || '%' END) \
             AND (?7 IS NULL OR path LIKE '%' || ?7 || '%' OR COALESCE(model,'') LIKE '%' || ?7 || '%' OR COALESCE(error,'') LIKE '%' || ?7 || '%')";
        let total: i64 = conn.query_row(
            &format!("SELECT COUNT(*) FROM request_logs {where_clause}"),
            params![start, end, model, channel_id, api_key_id, status, search],
            |r| r.get(0),
        )?;
        let offset = (page.max(1) - 1) * page_size.max(1);
        let mut stmt = conn.prepare(&format!(
            "SELECT id,method,path,model,channel_id,api_key_id,status_code,error,tokens_used,duration_ms,created_at \
             FROM request_logs {where_clause} ORDER BY created_at DESC LIMIT ?8 OFFSET ?9"
        ))?;
        let rows = stmt.query_map(
            params![start, end, model, channel_id, api_key_id, status, search, page_size.max(1), offset],
            |r| {
                Ok(RequestLog {
                    id: r.get(0)?,
                    method: r.get(1)?,
                    path: r.get(2)?,
                    model: r.get(3)?,
                    channel_id: r.get(4)?,
                    api_key_id: r.get(5)?,
                    status_code: r.get(6)?,
                    error: r.get(7)?,
                    tokens_used: r.get(8)?,
                    duration_ms: r.get(9)?,
                    created_at: DateTime::parse_from_rfc3339(&r.get::<_, String>(10)?)
                        .unwrap()
                        .with_timezone(&Utc),
                })
            },
        )?;
        let items = rows.collect::<SqliteResult<Vec<_>>>()?;
        Ok((items, total))
    }

    pub fn list_request_logs_since(
        &self,
        since: chrono::DateTime<Utc>,
        limit: i64,
    ) -> SqliteResult<Vec<RequestLog>> {
        let conn = self.conn.lock();
        let mut s = conn.prepare(
            "SELECT id,method,path,model,channel_id,api_key_id,status_code,error,tokens_used,duration_ms,created_at
             FROM request_logs WHERE created_at > ?1 ORDER BY created_at ASC LIMIT ?2",
        )?;
        let rows = s.query_map(params![since.to_rfc3339(), limit], |r| {
            Ok(RequestLog {
                id: r.get(0)?,
                method: r.get(1)?,
                path: r.get(2)?,
                model: r.get(3)?,
                channel_id: r.get(4)?,
                api_key_id: r.get(5)?,
                status_code: r.get(6)?,
                error: r.get(7)?,
                tokens_used: r.get(8)?,
                duration_ms: r.get(9)?,
                created_at: DateTime::parse_from_rfc3339(&r.get::<_, String>(10)?)
                    .unwrap()
                    .with_timezone(&Utc),
            })
        })?;
        rows.collect()
    }

    pub fn clear_request_logs(&self) -> SqliteResult<usize> {
        let conn = self.conn.lock();
        let n = conn.execute("DELETE FROM request_logs", [])?;
        Ok(n)
    }

    // ── Count helpers (for system info) ─────────────────────────────────────

    pub fn count_logs(&self) -> SqliteResult<i64> {
        let conn = self.conn.lock();
        conn.query_row("SELECT COUNT(*) FROM request_logs", [], |r| r.get(0))
    }

    pub fn count_channels(&self) -> SqliteResult<i64> {
        let conn = self.conn.lock();
        conn.query_row("SELECT COUNT(*) FROM channels", [], |r| r.get(0))
    }

    pub fn count_enabled_channels(&self) -> SqliteResult<i64> {
        let conn = self.conn.lock();
        conn.query_row("SELECT COUNT(*) FROM channels WHERE enabled=1", [], |r| {
            r.get(0)
        })
    }

    pub fn count_keys(&self) -> SqliteResult<i64> {
        let conn = self.conn.lock();
        conn.query_row("SELECT COUNT(*) FROM api_keys", [], |r| r.get(0))
    }

    // ── Model metadata registry ─────────────────────────────────────────────

    pub fn upsert_model_metadata(&self, m: &ModelMetadata) -> SqliteResult<()> {
        self.conn.lock().execute(
            r#"INSERT INTO model_metadata (id,model_name,description,icon,tags,vendor,endpoints,name_rule,status,sync_official,created_at,updated_at)
               VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)
               ON CONFLICT(id) DO UPDATE SET
                   model_name=?2, description=?3, icon=?4, tags=?5, vendor=?6, endpoints=?7,
                   name_rule=?8, status=?9, sync_official=?10, updated_at=?12"#,
            params![
                m.id, m.model_name, m.description, m.icon, m.tags, m.vendor,
                json_string(&m.endpoints), m.name_rule, m.status, m.sync_official,
                m.created_at.to_rfc3339(), m.updated_at.to_rfc3339()
            ],
        )?;
        Ok(())
    }

    pub fn list_model_metadata(&self) -> SqliteResult<Vec<ModelMetadata>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id,model_name,description,icon,tags,vendor,endpoints,name_rule,status,sync_official,created_at,updated_at
             FROM model_metadata ORDER BY model_name ASC",
        )?;
        let rows = stmt.query_map([], map_model_metadata)?;
        rows.collect()
    }

    pub fn list_model_metadata_paged(
        &self,
        page: i64,
        page_size: i64,
        search: Option<&str>,
    ) -> SqliteResult<(Vec<ModelMetadata>, i64)> {
        let conn = self.conn.lock();
        let where_clause = "WHERE (?1 IS NULL OR model_name LIKE '%' || ?1 || '%' OR vendor LIKE '%' || ?1 || '%' OR tags LIKE '%' || ?1 || '%')";
        let total: i64 = conn.query_row(
            &format!("SELECT COUNT(*) FROM model_metadata {where_clause}"),
            params![search],
            |r| r.get(0),
        )?;
        let offset = (page.max(1) - 1) * page_size.max(1);
        let mut stmt = conn.prepare(&format!(
            "SELECT id,model_name,description,icon,tags,vendor,endpoints,name_rule,status,sync_official,created_at,updated_at
             FROM model_metadata {where_clause} ORDER BY model_name ASC LIMIT ?2 OFFSET ?3"
        ))?;
        let rows = stmt.query_map(params![search, page_size.max(1), offset], map_model_metadata)?;
        let items = rows.collect::<SqliteResult<Vec<_>>>()?;
        Ok((items, total))
    }

    pub fn get_model_metadata(&self, id: &str) -> SqliteResult<Option<ModelMetadata>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id,model_name,description,icon,tags,vendor,endpoints,name_rule,status,sync_official,created_at,updated_at
             FROM model_metadata WHERE id=?1",
        )?;
        let mut rows = stmt.query(params![id])?;
        match rows.next()? {
            Some(row) => Ok(Some(map_model_metadata(row)?)),
            None => Ok(None),
        }
    }

    pub fn delete_model_metadata(&self, id: &str) -> SqliteResult<()> {
        self.conn.lock().execute("DELETE FROM model_metadata WHERE id=?1", params![id])?;
        Ok(())
    }

    // ── Vendors ───────────────────────────────────────────────────────────────

    /// Every vendor, each carrying its derived model count.
    ///
    /// The count is a `LEFT JOIN` against the model registry, not a stored
    /// column, so it cannot drift when a model is renamed or deleted — the same
    /// choice the reference makes (`ModelCount` is `gorm:"-"`).
    pub fn list_vendors(&self) -> SqliteResult<Vec<Vendor>> {
        let conn = self.conn.lock();
        let sql = format!(
            "SELECT {cols}, \
                    (SELECT COUNT(*) FROM model_metadata m WHERE m.vendor = v.name) \
             FROM vendors v ORDER BY v.name ASC",
            cols = VENDOR_COLUMNS
                .split(',')
                .map(|c| format!("v.{}", c))
                .collect::<Vec<_>>()
                .join(",")
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], |row| {
            let mut vendor = map_vendor(row)?;
            vendor.model_count = row.get(7)?;
            Ok(vendor)
        })?;
        rows.collect()
    }

    pub fn get_vendor(&self, id: &str) -> SqliteResult<Option<Vendor>> {
        let conn = self.conn.lock();
        let sql = format!(
            "SELECT {cols}, \
                    (SELECT COUNT(*) FROM model_metadata m WHERE m.vendor = v.name) \
             FROM vendors v WHERE v.id = ?1",
            cols = VENDOR_COLUMNS
                .split(',')
                .map(|c| format!("v.{}", c))
                .collect::<Vec<_>>()
                .join(",")
        );
        let mut stmt = conn.prepare(&sql)?;
        let mut rows = stmt.query(params![id])?;
        match rows.next()? {
            Some(row) => {
                let mut vendor = map_vendor(row)?;
                vendor.model_count = row.get(7)?;
                Ok(Some(vendor))
            }
            None => Ok(None),
        }
    }

    /// The vendor whose name matches exactly, if any.
    ///
    /// Vendor names are a uniqueness constraint, so a create can be refused
    /// rather than silently producing two rows with the same name.
    pub fn find_vendor_by_name(&self, name: &str) -> SqliteResult<Option<Vendor>> {
        let conn = self.conn.lock();
        let sql = format!("SELECT {VENDOR_COLUMNS} FROM vendors WHERE name = ?1");
        let mut stmt = conn.prepare(&sql)?;
        let mut rows = stmt.query(params![name])?;
        match rows.next()? {
            Some(row) => Ok(Some(map_vendor(row)?)),
            None => Ok(None),
        }
    }

    pub fn upsert_vendor(&self, v: &Vendor) -> SqliteResult<()> {
        let conn = self.conn.lock();
        conn.execute(
            r#"INSERT INTO vendors (id,name,description,icon,status,created_at,updated_at)
               VALUES (?1,?2,?3,?4,?5,?6,?7)
               ON CONFLICT(id) DO UPDATE SET
                   name=excluded.name,
                   description=excluded.description,
                   icon=excluded.icon,
                   status=excluded.status,
                   updated_at=excluded.updated_at"#,
            params![
                v.id,
                v.name,
                v.description,
                v.icon,
                v.status,
                v.created_at.to_rfc3339(),
                v.updated_at.to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    pub fn delete_vendor(&self, id: &str) -> SqliteResult<()> {
        self.conn.lock().execute("DELETE FROM vendors WHERE id=?1", params![id])?;
        Ok(())
    }

    /// Distinct model names seen across all channels (for the "missing metadata" view).
    pub fn distinct_channel_models(&self) -> SqliteResult<Vec<String>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare("SELECT model_list FROM channels")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        let mut names: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for row in rows {
            for name in json_value::<Vec<String>>(&row?) {
                let trimmed = name.trim();
                if !trimmed.is_empty() {
                    names.insert(trimmed.to_string());
                }
            }
        }
        Ok(names.into_iter().collect())
    }

    pub fn count_model_maps(&self) -> SqliteResult<i64> {        let conn = self.conn.lock();
        conn.query_row("SELECT COUNT(*) FROM model_maps", [], |r| r.get(0))
    }

    pub fn count_rules(&self) -> SqliteResult<i64> {
        let conn = self.conn.lock();
        conn.query_row("SELECT COUNT(*) FROM route_rules", [], |r| r.get(0))
    }

    // ── Backup (safe online backup using SQLite VACUUM INTO) ───────────────

    pub fn backup_to(&self, dest: &std::path::Path) -> SqliteResult<()> {
        let conn = self.conn.lock();
        let dest_str = dest.to_string_lossy().replace('\'', "''");
        conn.execute_batch(&format!("VACUUM INTO '{}'", dest_str))?;
        Ok(())
    }

    pub fn get_dashboard_snapshot(
        &self,
        time_range: &str,
        filters: &AnalyticsFilters,
    ) -> SqliteResult<DashboardSnapshot> {
        let conn = self.conn.lock();
        let (range_start, range_end, bucket_seconds, bucket_count) =
            Self::analytics_range(time_range);
        let start = range_start.to_rfc3339();
        let end = range_end.to_rfc3339();
        let status = filters.status.as_deref();
        let common = "request_logs.created_at >= ?1 AND request_logs.created_at < ?2 AND (?3 IS NULL OR COALESCE(request_logs.model, 'unknown') = ?3) AND (?4 IS NULL OR COALESCE(request_logs.channel_id, 'unassigned') = ?4) AND (?5 IS NULL OR COALESCE(request_logs.api_key_id, 'unassigned') = ?5) AND (?6 IS NULL OR CASE ?6 WHEN 'success' THEN request_logs.status_code >= 200 AND request_logs.status_code < 300 WHEN 'error' THEN request_logs.status_code < 200 OR request_logs.status_code >= 400 ELSE CAST(request_logs.status_code AS TEXT) LIKE ?6 || '%' END)";
        let totals_sql = format!("SELECT COUNT(*), COALESCE(SUM(CASE WHEN status_code >= 200 AND status_code < 300 THEN 1 ELSE 0 END), 0), COALESCE(AVG(duration_ms), 0), COALESCE(SUM(tokens_used), 0) FROM request_logs WHERE {common}");
        let (total_requests, successful_requests, average_latency_ms, total_tokens): (
            i64,
            i64,
            f64,
            i64,
        ) = conn.query_row(
            &totals_sql,
            params![
                start,
                end,
                filters.model.as_deref(),
                filters.channel_id.as_deref(),
                filters.api_key_id.as_deref(),
                status
            ],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )?;
        let failed_requests = total_requests - successful_requests;
        let today_requests = total_requests;
        let today_errors = failed_requests;
        let active_channels: i64 =
            conn.query_row("SELECT COUNT(*) FROM channels WHERE enabled = 1", [], |r| {
                r.get(0)
            })?;

        let breakdown = |field: &str, join: &str| -> SqliteResult<Vec<DashboardBreakdown>> {
            let sql = format!("SELECT COALESCE({field}, 'unassigned'), COUNT(*), COALESCE(SUM(CASE WHEN status_code < 200 OR status_code >= 400 THEN 1 ELSE 0 END), 0), COALESCE(AVG(duration_ms), 0), COALESCE(SUM(tokens_used), 0) FROM request_logs {join} WHERE {common} GROUP BY {field} ORDER BY COUNT(*) DESC");
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt
                .query_map(
                    params![
                        start,
                        end,
                        filters.model.as_deref(),
                        filters.channel_id.as_deref(),
                        filters.api_key_id.as_deref(),
                        status
                    ],
                    |r| {
                        Ok(DashboardBreakdown {
                            name: r.get(0)?,
                            requests: r.get(1)?,
                            errors: r.get(2)?,
                            average_latency_ms: r.get(3)?,
                            tokens: r.get(4)?,
                        })
                    },
                )?
                .collect::<SqliteResult<Vec<_>>>()?;
            Ok(rows)
        };
        let model_breakdown = breakdown("request_logs.model", "")?;
        let channel_breakdown = breakdown("request_logs.channel_id", "")?;
        let api_key_breakdown = breakdown(
            "api_keys.name",
            "LEFT JOIN api_keys ON api_keys.id = request_logs.api_key_id",
        )?;

        let recent_requests = {
            let sql = format!("SELECT id,method,path,model,channel_id,api_key_id,status_code,error,tokens_used,duration_ms,created_at FROM request_logs WHERE {common} ORDER BY created_at DESC LIMIT 8");
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(
                params![
                    start,
                    end,
                    filters.model.as_deref(),
                    filters.channel_id.as_deref(),
                    filters.api_key_id.as_deref(),
                    status
                ],
                |r| {
                    Ok(RequestLog {
                        id: r.get(0)?,
                        method: r.get(1)?,
                        path: r.get(2)?,
                        model: r.get(3)?,
                        channel_id: r.get(4)?,
                        api_key_id: r.get(5)?,
                        status_code: r.get(6)?,
                        error: r.get(7)?,
                        tokens_used: r.get(8)?,
                        duration_ms: r.get(9)?,
                        created_at: DateTime::parse_from_rfc3339(&r.get::<_, String>(10)?)
                            .unwrap()
                            .with_timezone(&Utc),
                    })
                },
            )?;
            rows.collect::<SqliteResult<Vec<_>>>()?
        };

        let time_series =
            Self::build_time_series(&conn, range_start, bucket_seconds, bucket_count, filters)?;
        let model_time_series = Self::build_dimension_time_series(
            &conn,
            range_start,
            bucket_seconds,
            bucket_count,
            filters,
            "COALESCE(model, 'unknown')",
            "",
        )?;
        let api_key_time_series = Self::build_dimension_time_series(
            &conn,
            range_start,
            bucket_seconds,
            bucket_count,
            filters,
            "COALESCE(api_keys.name, 'unassigned')",
            "LEFT JOIN api_keys ON api_keys.id = request_logs.api_key_id",
        )?;
        let channel_perf = Self::build_channel_perf(&conn, &start, &end, filters)?;

        Ok(DashboardSnapshot {
            range_start: start,
            range_end: end,
            bucket_seconds,
            total_requests,
            successful_requests,
            failed_requests,
            success_rate: if total_requests == 0 {
                0.0
            } else {
                successful_requests as f64 / total_requests as f64 * 100.0
            },
            average_latency_ms,
            today_requests,
            today_errors,
            total_tokens,
            active_channels,
            model_breakdown,
            channel_breakdown,
            api_key_breakdown,
            recent_requests,
            time_series,
            model_time_series,
            api_key_time_series,
            channel_perf,
            time_range: time_range.to_string(),
        })
    }

    fn analytics_range(time_range: &str) -> (DateTime<Utc>, DateTime<Utc>, i64, usize) {
        let (bucket_seconds, bucket_count) = match time_range {
            "1h" => (300, 12),
            "24h" => (3600, 24),
            "7d" => (86400, 7),
            "30d" => (86400, 30),
            _ => (3600, 24),
        };
        let end = Utc::now();
        let start = end - Duration::seconds(bucket_seconds * bucket_count as i64);
        (start, end, bucket_seconds, bucket_count)
    }

    fn build_time_series(
        conn: &Connection,
        start: DateTime<Utc>,
        seconds_per_bucket: i64,
        buckets: usize,
        filters: &AnalyticsFilters,
    ) -> SqliteResult<Vec<TimeBucket>> {
        let end = start + Duration::seconds(seconds_per_bucket * buckets as i64);
        let mut stmt = conn.prepare("SELECT CAST((strftime('%s', created_at) - strftime('%s', ?1)) / ?2 AS INTEGER), COUNT(*), COALESCE(SUM(CASE WHEN status_code < 200 OR status_code >= 400 THEN 1 ELSE 0 END), 0), COALESCE(AVG(duration_ms), 0) FROM request_logs WHERE created_at >= ?1 AND created_at < ?3 AND (?4 IS NULL OR COALESCE(model, 'unknown') = ?4) AND (?5 IS NULL OR COALESCE(channel_id, 'unassigned') = ?5) AND (?6 IS NULL OR COALESCE(api_key_id, 'unassigned') = ?6) AND (?7 IS NULL OR CASE ?7 WHEN 'success' THEN status_code >= 200 AND status_code < 300 WHEN 'error' THEN status_code < 200 OR status_code >= 400 ELSE CAST(status_code AS TEXT) LIKE ?7 || '%' END) GROUP BY 1")?;
        let grouped = stmt
            .query_map(
                params![
                    start.to_rfc3339(),
                    seconds_per_bucket,
                    end.to_rfc3339(),
                    filters.model.as_deref(),
                    filters.channel_id.as_deref(),
                    filters.api_key_id.as_deref(),
                    filters.status.as_deref()
                ],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, i64>(2)?,
                        r.get::<_, f64>(3)?,
                    ))
                },
            )?
            .collect::<SqliteResult<Vec<_>>>()?;
        let rows: std::collections::BTreeMap<i64, (i64, i64, f64)> = grouped
            .into_iter()
            .map(|(bucket, requests, errors, latency)| (bucket, (requests, errors, latency)))
            .collect();
        let buckets_vec = (0..buckets)
            .map(|index| {
                let bucket_start = start + Duration::seconds(index as i64 * seconds_per_bucket);
                let (requests, errors, latency_ms) =
                    rows.get(&(index as i64)).copied().unwrap_or((0, 0, 0.0));
                TimeBucket {
                    label: if seconds_per_bucket >= 86400 {
                        bucket_start.format("%b %-d").to_string()
                    } else if seconds_per_bucket >= 3600 {
                        bucket_start.format("%H:00").to_string()
                    } else {
                        bucket_start.format("%H:%M").to_string()
                    },
                    requests,
                    errors,
                    latency_ms,
                }
            })
            .collect();
        Ok(buckets_vec)
    }

    fn build_dimension_time_series(
        conn: &Connection,
        start: DateTime<Utc>,
        seconds: i64,
        buckets: usize,
        filters: &AnalyticsFilters,
        dimension: &str,
        join: &str,
    ) -> SqliteResult<Vec<ModelTimeBucket>> {
        let end = start + Duration::seconds(seconds * buckets as i64);
        let sql = format!("SELECT CAST((strftime('%s', request_logs.created_at) - strftime('%s', ?1)) / ?2 AS INTEGER), {dimension}, COUNT(*), COALESCE(SUM(CASE WHEN request_logs.status_code < 200 OR request_logs.status_code >= 400 THEN 1 ELSE 0 END), 0), COALESCE(SUM(request_logs.tokens_used), 0), COALESCE(AVG(request_logs.duration_ms), 0) FROM request_logs {join} WHERE request_logs.created_at >= ?1 AND request_logs.created_at < ?3 AND (?4 IS NULL OR COALESCE(request_logs.model, 'unknown') = ?4) AND (?5 IS NULL OR COALESCE(request_logs.channel_id, 'unassigned') = ?5) AND (?6 IS NULL OR COALESCE(request_logs.api_key_id, 'unassigned') = ?6) AND (?7 IS NULL OR CASE ?7 WHEN 'success' THEN request_logs.status_code >= 200 AND request_logs.status_code < 300 WHEN 'error' THEN request_logs.status_code < 200 OR request_logs.status_code >= 400 ELSE CAST(request_logs.status_code AS TEXT) LIKE ?7 || '%' END) GROUP BY 1, 2");
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt
            .query_map(
                params![
                    start.to_rfc3339(),
                    seconds,
                    end.to_rfc3339(),
                    filters.model.as_deref(),
                    filters.channel_id.as_deref(),
                    filters.api_key_id.as_deref(),
                    filters.status.as_deref()
                ],
                |r| {
                    let bucket: i64 = r.get(0)?;
                    let at = start + Duration::seconds(bucket * seconds);
                    Ok(ModelTimeBucket {
                        label: if seconds >= 86400 {
                            at.format("%b %-d").to_string()
                        } else if seconds >= 3600 {
                            at.format("%H:00").to_string()
                        } else {
                            at.format("%H:%M").to_string()
                        },
                        model: r.get(1)?,
                        requests: r.get(2)?,
                        errors: r.get(3)?,
                        tokens: r.get(4)?,
                        latency_ms: r.get(5)?,
                    })
                },
            )?
            .collect::<SqliteResult<Vec<_>>>()?;
        Ok(rows)
    }

    fn build_channel_perf(
        conn: &Connection,
        start: &str,
        end: &str,
        filters: &AnalyticsFilters,
    ) -> SqliteResult<Vec<ChannelPerf>> {
        let sql = "SELECT c.id, c.name, c.enabled, COUNT(l.id), COALESCE(SUM(CASE WHEN l.status_code < 200 OR l.status_code >= 400 THEN 1 ELSE 0 END), 0), COALESCE(AVG(l.duration_ms), 0), (SELECT error FROM request_logs e WHERE e.channel_id = c.id AND e.created_at >= ?1 AND e.created_at < ?2 AND e.error IS NOT NULL ORDER BY e.created_at DESC LIMIT 1), (SELECT strftime('%s', MAX(created_at)) FROM request_logs e WHERE e.channel_id = c.id AND e.created_at >= ?1 AND e.created_at < ?2 AND e.error IS NOT NULL) FROM channels c LEFT JOIN request_logs l ON l.channel_id = c.id AND l.created_at >= ?1 AND l.created_at < ?2 AND (?3 IS NULL OR COALESCE(l.model, 'unknown') = ?3) AND (?4 IS NULL OR COALESCE(l.channel_id, 'unassigned') = ?4) AND (?5 IS NULL OR COALESCE(l.api_key_id, 'unassigned') = ?5) AND (?6 IS NULL OR CASE ?6 WHEN 'success' THEN l.status_code >= 200 AND l.status_code < 300 WHEN 'error' THEN l.status_code < 200 OR l.status_code >= 400 ELSE CAST(l.status_code AS TEXT) LIKE ?6 || '%' END) GROUP BY c.id ORDER BY COUNT(l.id) DESC, c.name ASC";
        let mut stmt = conn.prepare(sql)?;
        let rows = stmt
            .query_map(
                params![
                    start,
                    end,
                    filters.model.as_deref(),
                    filters.channel_id.as_deref(),
                    filters.api_key_id.as_deref(),
                    filters.status.as_deref()
                ],
                |r| {
                    Ok(ChannelPerf {
                        channel_id: r.get(0)?,
                        channel_name: r.get(1)?,
                        enabled: r.get::<_, i32>(2)? != 0,
                        requests: r.get(3)?,
                        errors: r.get(4)?,
                        latency_ms: r.get(5)?,
                        last_error: r.get(6)?,
                        last_test_at: r.get(7)?,
                    })
                },
            )?
            .collect::<SqliteResult<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn get_analytics_flow(
        &self,
        time_range: &str,
        filters: &AnalyticsFilters,
    ) -> SqliteResult<AnalyticsFlow> {
        let conn = self.conn.lock();
        let (start, end, _, _) = Self::analytics_range(time_range);
        let sql = "SELECT COALESCE(k.name, 'unassigned'), COALESCE(l.model, 'unknown'), COALESCE(c.name, l.channel_id, 'unassigned'), COUNT(*), COALESCE(SUM(l.tokens_used), 0) FROM request_logs l LEFT JOIN api_keys k ON k.id = l.api_key_id LEFT JOIN channels c ON c.id = l.channel_id WHERE l.created_at >= ?1 AND l.created_at < ?2 AND (?3 IS NULL OR COALESCE(l.model, 'unknown') = ?3) AND (?4 IS NULL OR COALESCE(l.channel_id, 'unassigned') = ?4) AND (?5 IS NULL OR COALESCE(l.api_key_id, 'unassigned') = ?5) AND (?6 IS NULL OR CASE ?6 WHEN 'success' THEN l.status_code >= 200 AND l.status_code < 300 WHEN 'error' THEN l.status_code < 200 OR l.status_code >= 400 ELSE CAST(l.status_code AS TEXT) LIKE ?6 || '%' END) GROUP BY 1, 2, 3";
        let mut stmt = conn.prepare(sql)?;
        let rows = stmt
            .query_map(
                params![
                    start.to_rfc3339(),
                    end.to_rfc3339(),
                    filters.model.as_deref(),
                    filters.channel_id.as_deref(),
                    filters.api_key_id.as_deref(),
                    filters.status.as_deref()
                ],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, i64>(3)?,
                        r.get::<_, i64>(4)?,
                    ))
                },
            )?
            .collect::<SqliteResult<Vec<_>>>()?;
        let mut nodes = std::collections::BTreeMap::new();
        let mut links = std::collections::BTreeMap::<(String, String), (i64, i64)>::new();
        for (key, model, channel, requests, tokens) in rows {
            let key_id = format!("key:{key}");
            let model_id = format!("model:{model}");
            let channel_id = format!("channel:{channel}");
            nodes.insert(
                key_id.clone(),
                AnalyticsFlowNode {
                    id: key_id.clone(),
                    label: key,
                    kind: "api_key".into(),
                },
            );
            nodes.insert(
                model_id.clone(),
                AnalyticsFlowNode {
                    id: model_id.clone(),
                    label: model,
                    kind: "model".into(),
                },
            );
            nodes.insert(
                channel_id.clone(),
                AnalyticsFlowNode {
                    id: channel_id.clone(),
                    label: channel,
                    kind: "channel".into(),
                },
            );
            for edge in [(key_id, model_id.clone()), (model_id, channel_id)] {
                let entry = links.entry(edge).or_default();
                entry.0 += requests;
                entry.1 += tokens;
            }
        }
        Ok(AnalyticsFlow {
            range_start: start.to_rfc3339(),
            range_end: end.to_rfc3339(),
            nodes: nodes.into_values().collect(),
            links: links
                .into_iter()
                .map(
                    |((source, target), (request_count, tokens))| AnalyticsFlowLink {
                        source,
                        target,
                        request_count,
                        tokens,
                    },
                )
                .collect(),
        })
    }

    // ── Settings ────────────────────────────────────────────────────────────────

    pub fn get_setting(&self, key: &str) -> SqliteResult<Option<String>> {
        let conn = self.conn.lock();
        let mut s = conn.prepare("SELECT value FROM settings WHERE key=?1")?;
        let mut rows = s.query(params![key])?;
        if let Some(r) = rows.next()? {
            Ok(Some(r.get(0)?))
        } else {
            Ok(None)
        }
    }

    /// All persisted settings as key/value pairs.
    pub fn list_settings(&self) -> SqliteResult<std::collections::BTreeMap<String, String>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare("SELECT key, value FROM settings")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        let mut map = std::collections::BTreeMap::new();
        for row in rows {
            let (key, value) = row?;
            map.insert(key, value);
        }
        Ok(map)
    }

    pub fn set_setting(&self, key: &str, value: &str) -> SqliteResult<()> {        self.conn.lock().execute(
            "INSERT INTO settings (key,value) VALUES (?1,?2) ON CONFLICT(key) DO UPDATE SET value=?2",
            params![key, value],
        )?;
        Ok(())
    }

    // ── SaaS identity, wallet, and commerce ──────────────────────────────────

    fn parse_time(value: String) -> SqliteResult<DateTime<Utc>> {
        DateTime::parse_from_rfc3339(&value)
            .map(|time| time.with_timezone(&Utc))
            .map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })
    }

    fn user_from_row(row: &rusqlite::Row<'_>) -> SqliteResult<User> {
        Ok(User {
            id: row.get(0)?,
            username: row.get(1)?,
            email: row.get(2)?,
            role: UserRole::from_db(&row.get::<_, String>(3)?),
            status: row.get(4)?,
            balance_micros: row.get(5)?,
            created_at: Self::parse_time(row.get(6)?)?,
            updated_at: Self::parse_time(row.get(7)?)?,
            last_login_at: row
                .get::<_, Option<String>>(8)?
                .map(Self::parse_time)
                .transpose()?,
        })
    }

    fn hash_password(password: &str) -> SqliteResult<String> {
        let salt = SaltString::generate(&mut OsRng);
        Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map(|hash| hash.to_string())
            .map_err(|_| rusqlite::Error::InvalidQuery)
    }

    pub fn ensure_bootstrap_admin(&self) -> SqliteResult<Option<(String, String)>> {
        let count: i64 = self
            .conn
            .lock()
            .query_row("SELECT COUNT(*) FROM users", [], |row| row.get(0))?;
        if count != 0 {
            return Ok(None);
        }
        let password = std::env::var("OXYGENROUTER_ADMIN_PASSWORD")
            .unwrap_or_else(|_| Uuid::new_v4().simple().to_string());
        self.create_user("admin", "admin@localhost", &password, UserRole::Admin)?;
        Ok(Some(("admin".to_string(), password)))
    }

    pub fn create_user(
        &self,
        username: &str,
        email: &str,
        password: &str,
        role: UserRole,
    ) -> SqliteResult<User> {
        if username.trim().len() < 3 || password.len() < 8 || !email.contains('@') {
            return Err(rusqlite::Error::InvalidQuery);
        }
        let now = Utc::now();
        let user = User {
            id: Uuid::new_v4().to_string(),
            username: username.trim().to_string(),
            email: email.trim().to_string(),
            role,
            status: "active".into(),
            balance_micros: 0,
            created_at: now,
            updated_at: now,
            last_login_at: None,
        };
        let hash = Self::hash_password(password)?;
        self.conn.lock().execute("INSERT INTO users (id,username,email,password_hash,role,status,balance_micros,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)", params![user.id, user.username, user.email, hash, user.role.as_str(), user.status, user.balance_micros, now.to_rfc3339(), now.to_rfc3339()])?;
        Ok(user)
    }

    pub fn list_users(&self, search: Option<&str>) -> SqliteResult<Vec<User>> {
        let conn = self.conn.lock();
        let pattern = search.map(|value| format!("%{}%", value));
        let mut stmt = conn.prepare("SELECT id,username,email,role,status,balance_micros,created_at,updated_at,last_login_at FROM users WHERE (?1 IS NULL OR username LIKE ?1 OR email LIKE ?1) ORDER BY created_at DESC")?;
        let users = stmt
            .query_map(params![pattern], Self::user_from_row)?
            .collect();
        users
    }
    pub fn get_user(&self, id: &str) -> SqliteResult<Option<User>> {
        let conn = self.conn.lock();
        conn.query_row("SELECT id,username,email,role,status,balance_micros,created_at,updated_at,last_login_at FROM users WHERE id=?1", params![id], Self::user_from_row).optional()
    }
    pub fn update_user(
        &self,
        id: &str,
        username: &str,
        email: &str,
        role: UserRole,
        status: &str,
    ) -> SqliteResult<Option<User>> {
        let now = Utc::now().to_rfc3339();
        let conn = self.conn.lock();
        let changed = conn.execute(
            "UPDATE users SET username=?2,email=?3,role=?4,status=?5,updated_at=?6 WHERE id=?1",
            params![
                id,
                username.trim(),
                email.trim(),
                role.as_str(),
                status,
                now
            ],
        )?;
        if changed == 0 {
            Ok(None)
        } else {
            conn.query_row("SELECT id,username,email,role,status,balance_micros,created_at,updated_at,last_login_at FROM users WHERE id=?1", params![id], Self::user_from_row).map(Some)
        }
    }
    pub fn delete_user(&self, id: &str) -> SqliteResult<bool> {
        Ok(self
            .conn
            .lock()
            .execute("DELETE FROM users WHERE id=?1", params![id])?
            > 0)
    }

    pub fn authenticate(&self, username: &str, password: &str) -> SqliteResult<Option<User>> {
        let conn = self.conn.lock();
        let found: Option<(User, String)> = conn.query_row("SELECT id,username,email,role,status,balance_micros,created_at,updated_at,last_login_at,password_hash FROM users WHERE username=?1 AND status='active'", params![username], |row| Ok((Self::user_from_row(row)?, row.get(9)?))).optional()?;
        let Some((user, hash)) = found else {
            return Ok(None);
        };
        let parsed = PasswordHash::new(&hash).map_err(|_| rusqlite::Error::InvalidQuery)?;
        if Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_err()
        {
            return Ok(None);
        }
        conn.execute(
            "UPDATE users SET last_login_at=?2,updated_at=?2 WHERE id=?1",
            params![user.id, Utc::now().to_rfc3339()],
        )?;
        drop(conn);
        self.get_user(&user.id)
    }

    pub fn create_session(&self, user_id: &str, duration: Duration) -> SqliteResult<AuthSession> {
        let now = Utc::now();
        let session = AuthSession {
            id: Uuid::new_v4().to_string(),
            user_id: user_id.to_string(),
            token: Uuid::new_v4().to_string(),
            expires_at: now + duration,
            created_at: now,
        };
        self.conn.lock().execute("INSERT INTO auth_sessions (id,user_id,token,expires_at,created_at) VALUES (?1,?2,?3,?4,?5)", params![session.id, session.user_id, session.token, session.expires_at.to_rfc3339(), session.created_at.to_rfc3339()])?;
        Ok(session)
    }
    pub fn revoke_session(&self, token: &str) -> SqliteResult<bool> {
        Ok(self
            .conn
            .lock()
            .execute("DELETE FROM auth_sessions WHERE token=?1", params![token])?
            > 0)
    }
    pub fn session_user(&self, token: &str) -> SqliteResult<Option<(User, DateTime<Utc>)>> {
        let conn = self.conn.lock();
        conn.query_row("SELECT u.id,u.username,u.email,u.role,u.status,u.balance_micros,u.created_at,u.updated_at,u.last_login_at,s.expires_at FROM auth_sessions s JOIN users u ON u.id=s.user_id WHERE s.token=?1 AND s.expires_at>?2 AND u.status='active'", params![token, Utc::now().to_rfc3339()], |row| Ok((Self::user_from_row(row)?, Self::parse_time(row.get(9)?)?))).optional()
    }

    /// A user's live sessions, newest first.
    ///
    /// Expired rows are excluded rather than deleted: a purge would race with a
    /// concurrent request that is still holding the token, and the caller only
    /// ever wants the sessions that are actually usable.
    pub fn sessions_for_user(&self, user_id: &str) -> SqliteResult<Vec<AuthSession>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id,user_id,token,expires_at,created_at FROM auth_sessions
             WHERE user_id=?1 AND expires_at>?2
             ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map(params![user_id, Utc::now().to_rfc3339()], |row| {
            Ok(AuthSession {
                id: row.get(0)?,
                user_id: row.get(1)?,
                token: row.get(2)?,
                expires_at: Self::parse_time(row.get(3)?)?,
                created_at: Self::parse_time(row.get(4)?)?,
            })
        })?;
        rows.collect()
    }

    /// The session row behind a token, if it is still live.
    ///
    /// Used to identify *which* session is making a call, so the API can mark it
    /// as the current one and refuse to revoke it out from under the caller.
    pub fn session_by_token(&self, token: &str) -> SqliteResult<Option<AuthSession>> {
        let conn = self.conn.lock();
        conn.query_row(
            "SELECT id,user_id,token,expires_at,created_at FROM auth_sessions
             WHERE token=?1 AND expires_at>?2",
            params![token, Utc::now().to_rfc3339()],
            |row| {
                Ok(AuthSession {
                    id: row.get(0)?,
                    user_id: row.get(1)?,
                    token: row.get(2)?,
                    expires_at: Self::parse_time(row.get(3)?)?,
                    created_at: Self::parse_time(row.get(4)?)?,
                })
            },
        )
        .optional()
    }

    /// Revoke one of `user_id`'s sessions, identified by its session id.
    ///
    /// Scoped by `user_id` rather than leaving it to the caller: a delete keyed on
    /// the id alone would let any authenticated user revoke any other user's
    /// session by guessing an id.
    pub fn revoke_session_by_id(&self, user_id: &str, session_id: &str) -> SqliteResult<bool> {
        Ok(self
            .conn
            .lock()
            .execute(
                "DELETE FROM auth_sessions WHERE id=?1 AND user_id=?2",
                params![session_id, user_id],
            )?
            > 0)
    }

    /// Revoke every session of `user_id` except `keep_session_id`.
    ///
    /// Returns how many rows were removed. A `None` keep-id revokes them all,
    /// which is the "sign out everywhere" case.
    pub fn revoke_other_sessions(
        &self,
        user_id: &str,
        keep_session_id: Option<&str>,
    ) -> SqliteResult<usize> {
        let conn = self.conn.lock();
        match keep_session_id {
            Some(keep) => Ok(conn.execute(
                "DELETE FROM auth_sessions WHERE user_id=?1 AND id<>?2",
                params![user_id, keep],
            )?),
            None => Ok(
                conn.execute("DELETE FROM auth_sessions WHERE user_id=?1", params![user_id])?
            ),
        }
    }

    fn credit_tx(
        tx: &rusqlite::Transaction<'_>,
        user_id: &str,
        amount: i64,
        kind: &str,
        description: &str,
        reference_id: Option<&str>,
    ) -> SqliteResult<LedgerEntry> {
        let balance: i64 = tx.query_row(
            "SELECT balance_micros FROM users WHERE id=?1",
            params![user_id],
            |row| row.get(0),
        )?;
        let after = balance
            .checked_add(amount)
            .ok_or(rusqlite::Error::InvalidQuery)?;
        if after < 0 {
            return Err(rusqlite::Error::InvalidParameterName(
                "insufficient balance".into(),
            ));
        }
        let now = Utc::now();
        let entry = LedgerEntry {
            id: Uuid::new_v4().to_string(),
            user_id: user_id.to_string(),
            amount_micros: amount,
            balance_after_micros: after,
            kind: kind.into(),
            description: description.into(),
            reference_id: reference_id.map(str::to_string),
            created_at: now,
        };
        tx.execute(
            "UPDATE users SET balance_micros=?2,updated_at=?3 WHERE id=?1",
            params![user_id, after, now.to_rfc3339()],
        )?;
        tx.execute("INSERT INTO ledger_entries (id,user_id,amount_micros,balance_after_micros,kind,description,reference_id,created_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)", params![entry.id,entry.user_id,entry.amount_micros,entry.balance_after_micros,entry.kind,entry.description,entry.reference_id,entry.created_at.to_rfc3339()])?;
        Ok(entry)
    }
    pub fn admin_adjust_balance(
        &self,
        user_id: &str,
        amount: i64,
        description: &str,
    ) -> SqliteResult<LedgerEntry> {
        if amount == 0 {
            return Err(rusqlite::Error::InvalidQuery);
        }
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        let entry = Self::credit_tx(&tx, user_id, amount, "admin_adjustment", description, None)?;
        tx.commit()?;
        Ok(entry)
    }

    // -----------------------------------------------------------------------
    // Billing: atomic quota movement
    //
    // These are the storage primitives the billing engine drives. Each guard
    // and the mutation it authorises happen in one statement so two concurrent
    // requests cannot both pass a stale check and over-spend.
    // -----------------------------------------------------------------------

    /// Atomically move `amount` out of an API key's quota.
    ///
    /// Returns `Ok(true)` when the key had room. A key with no quota ceiling
    /// (`quota_micros == 0`) is unlimited, matching NewAPI's semantics.
    pub fn try_reserve_key_quota(
        &self,
        key_id: &str,
        amount: i64,
    ) -> SqliteResult<bool> {
        if amount <= 0 {
            return Ok(true);
        }
        let changed = self.conn.lock().execute(
            "UPDATE api_keys SET used_micros = used_micros + ?2 \
             WHERE id = ?1 AND (quota_micros = 0 OR used_micros + ?2 <= quota_micros)",
            params![key_id, amount],
        )?;
        Ok(changed == 1)
    }

    /// Unconditionally move `amount` out of an API key's used-quota counter.
    ///
    /// Used at settlement, where a key may legitimately exceed its ceiling
    /// (the request is already served; the overage must be recorded).
    pub fn debit_key_quota(&self, key_id: &str, amount: i64) -> SqliteResult<()> {
        if amount == 0 {
            return Ok(());
        }
        self.conn.lock().execute(
            "UPDATE api_keys SET used_micros = used_micros + ?2 WHERE id = ?1",
            params![key_id, amount],
        )?;
        Ok(())
    }

    /// Move `amount` back into an API key's counter, never below zero.
    pub fn credit_key_quota(&self, key_id: &str, amount: i64) -> SqliteResult<()> {
        if amount == 0 {
            return Ok(());
        }
        self.conn.lock().execute(
            "UPDATE api_keys SET used_micros = MAX(0, used_micros - ?2) WHERE id = ?1",
            params![key_id, amount],
        )?;
        Ok(())
    }

    /// Atomically move `amount` out of a user's wallet.
    ///
    /// Returns `Ok(false)` when the wallet cannot cover it.
    pub fn try_reserve_wallet(
        &self,
        user_id: &str,
        amount: i64,
    ) -> SqliteResult<bool> {
        if amount <= 0 {
            return Ok(true);
        }
        let changed = self.conn.lock().execute(
            "UPDATE users SET balance_micros = balance_micros - ?2, updated_at = ?3 \
             WHERE id = ?1 AND balance_micros >= ?2",
            params![user_id, amount, Utc::now().to_rfc3339()],
        )?;
        Ok(changed == 1)
    }

    /// Move `amount` out of a wallet, allowing it to go negative (debt).
    ///
    /// Settlement may exceed the reservation; the wallet absorbs the difference
    /// rather than the request becoming free.
    pub fn debit_wallet(&self, user_id: &str, amount: i64) -> SqliteResult<()> {
        if amount == 0 {
            return Ok(());
        }
        self.conn.lock().execute(
            "UPDATE users SET balance_micros = balance_micros - ?2, updated_at = ?3 WHERE id = ?1",
            params![user_id, amount, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    /// Move `amount` into a wallet.
    pub fn credit_wallet(&self, user_id: &str, amount: i64) -> SqliteResult<()> {
        if amount == 0 {
            return Ok(());
        }
        self.conn.lock().execute(
            "UPDATE users SET balance_micros = balance_micros + ?2, updated_at = ?3 WHERE id = ?1",
            params![user_id, amount, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    /// Append a ledger entry **without** moving the balance.
    ///
    /// Billing settlements move the balance through `debit_wallet`/`credit_wallet`
    /// (which the billing session owns); this records the reason alongside the
    /// current balance so the wallet history stays explainable. Doing both in
    /// one call would double-charge.
    pub fn append_ledger_entry(
        &self,
        user_id: &str,
        amount_micros: i64,
        kind: &str,
        description: &str,
        reference_id: Option<&str>,
    ) -> SqliteResult<LedgerEntry> {
        let conn = self.conn.lock();
        let balance: i64 = conn.query_row(
            "SELECT COALESCE(balance_micros, 0) FROM users WHERE id=?1",
            params![user_id],
            |row| row.get(0),
        )?;
        let now = Utc::now();
        let entry = LedgerEntry {
            id: Uuid::new_v4().to_string(),
            user_id: user_id.to_string(),
            amount_micros,
            balance_after_micros: balance,
            kind: kind.to_string(),
            description: description.to_string(),
            reference_id: reference_id.map(str::to_string),
            created_at: now,
        };
        conn.execute("INSERT INTO ledger_entries (id,user_id,amount_micros,balance_after_micros,kind,description,reference_id,created_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)", params![entry.id,entry.user_id,entry.amount_micros,entry.balance_after_micros,entry.kind,entry.description,entry.reference_id,entry.created_at.to_rfc3339()])?;
        Ok(entry)
    }

    /// Total quota consumed by one API key, in micros.
    pub fn key_used_micros(&self, key_id: &str) -> SqliteResult<i64> {
        let conn = self.conn.lock();
        let value: i64 = conn.query_row(
            "SELECT COALESCE(used_micros, 0) FROM api_keys WHERE id = ?1",
            params![key_id],
            |r| r.get(0),
        )?;
        Ok(value)
    }

    /// A user's current wallet balance, in micros.
    pub fn wallet_balance(&self, user_id: &str) -> SqliteResult<i64> {
        let conn = self.conn.lock();
        let value: i64 = conn.query_row(
            "SELECT COALESCE(balance_micros, 0) FROM users WHERE id = ?1",
            params![user_id],
            |r| r.get(0),
        )?;
        Ok(value)
    }
    pub fn list_ledger(&self, user_id: &str) -> SqliteResult<Vec<LedgerEntry>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare("SELECT id,user_id,amount_micros,balance_after_micros,kind,description,reference_id,created_at FROM ledger_entries WHERE user_id=?1 ORDER BY created_at DESC")?;
        let entries = stmt
            .query_map(params![user_id], |row| {
                Ok(LedgerEntry {
                    id: row.get(0)?,
                    user_id: row.get(1)?,
                    amount_micros: row.get(2)?,
                    balance_after_micros: row.get(3)?,
                    kind: row.get(4)?,
                    description: row.get(5)?,
                    reference_id: row.get(6)?,
                    created_at: Self::parse_time(row.get(7)?)?,
                })
            })?
            .collect();
        entries
    }

    fn plan_from_row(row: &rusqlite::Row<'_>) -> SqliteResult<SubscriptionPlan> {
        Ok(SubscriptionPlan {
            id: row.get(0)?,
            name: row.get(1)?,
            description: row.get(2)?,
            price_micros: row.get(3)?,
            quota_micros: row.get(4)?,
            duration_days: row.get(5)?,
            enabled: row.get::<_, i32>(6)? != 0,
            created_at: Self::parse_time(row.get(7)?)?,
            updated_at: Self::parse_time(row.get(8)?)?,
        })
    }
    pub fn upsert_plan(&self, plan: &SubscriptionPlan) -> SqliteResult<()> {
        self.conn.lock().execute("INSERT INTO subscription_plans (id,name,description,price_micros,quota_micros,duration_days,enabled,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9) ON CONFLICT(id) DO UPDATE SET name=?2,description=?3,price_micros=?4,quota_micros=?5,duration_days=?6,enabled=?7,updated_at=?9", params![plan.id,plan.name,plan.description,plan.price_micros,plan.quota_micros,plan.duration_days,plan.enabled as i32,plan.created_at.to_rfc3339(),plan.updated_at.to_rfc3339()])?;
        Ok(())
    }
    pub fn list_plans(&self, enabled_only: bool) -> SqliteResult<Vec<SubscriptionPlan>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare("SELECT id,name,description,price_micros,quota_micros,duration_days,enabled,created_at,updated_at FROM subscription_plans WHERE (?1=0 OR enabled=1) ORDER BY price_micros")?;
        let plans = stmt
            .query_map(params![enabled_only as i32], Self::plan_from_row)?
            .collect();
        plans
    }
    pub fn delete_plan(&self, id: &str) -> SqliteResult<bool> {
        Ok(self
            .conn
            .lock()
            .execute("DELETE FROM subscription_plans WHERE id=?1", params![id])?
            > 0)
    }
    pub fn subscribe(&self, user_id: &str, plan_id: &str) -> SqliteResult<Subscription> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        let plan = tx.query_row("SELECT id,name,description,price_micros,quota_micros,duration_days,enabled,created_at,updated_at FROM subscription_plans WHERE id=?1 AND enabled=1", params![plan_id], Self::plan_from_row).optional()?.ok_or(rusqlite::Error::QueryReturnedNoRows)?;
        let now = Utc::now();
        tx.execute(
            "UPDATE subscriptions SET status='expired' WHERE user_id=?1 AND status='active' AND expires_at<=?2",
            params![user_id, now.to_rfc3339()],
        )?;
        let same_plan_active: Option<String> = tx
            .query_row(
                "SELECT id FROM subscriptions WHERE user_id=?1 AND plan_id=?2 AND status='active' LIMIT 1",
                params![user_id, plan.id],
                |row| row.get(0),
            )
            .optional()?;
        if same_plan_active.is_some() {
            return Err(rusqlite::Error::InvalidParameterName(
                "subscription already active".into(),
            ));
        }
        tx.execute(
            "UPDATE subscriptions SET status='cancelled' WHERE user_id=?1 AND status='active'",
            params![user_id],
        )?;
        let subscription = Subscription {
            id: Uuid::new_v4().to_string(),
            user_id: user_id.into(),
            plan_id: plan.id,
            status: "active".into(),
            started_at: now,
            expires_at: now + Duration::days(plan.duration_days),
            created_at: now,
        };
        if plan.price_micros > 0 {
            Self::credit_tx(
                &tx,
                user_id,
                -plan.price_micros,
                "subscription_purchase",
                &format!("Subscription: {}", plan.name),
                Some(&subscription.id),
            )?;
        }
        tx.execute("INSERT INTO subscriptions (id,user_id,plan_id,status,started_at,expires_at,created_at) VALUES (?1,?2,?3,?4,?5,?6,?7)",params![subscription.id,subscription.user_id,subscription.plan_id,subscription.status,subscription.started_at.to_rfc3339(),subscription.expires_at.to_rfc3339(),subscription.created_at.to_rfc3339()])?;
        tx.commit()?;
        Ok(subscription)
    }
    pub fn list_subscriptions(&self, user_id: &str) -> SqliteResult<Vec<Subscription>> {
        let conn = self.conn.lock();
        let mut stmt=conn.prepare("SELECT id,user_id,plan_id,status,started_at,expires_at,created_at FROM subscriptions WHERE user_id=?1 ORDER BY created_at DESC")?;
        let subscriptions = stmt
            .query_map(params![user_id], |row| {
                Ok(Subscription {
                    id: row.get(0)?,
                    user_id: row.get(1)?,
                    plan_id: row.get(2)?,
                    status: row.get(3)?,
                    started_at: Self::parse_time(row.get(4)?)?,
                    expires_at: Self::parse_time(row.get(5)?)?,
                    created_at: Self::parse_time(row.get(6)?)?,
                })
            })?
            .collect();
        subscriptions
    }

    fn redemption_from_row(row: &rusqlite::Row<'_>) -> SqliteResult<RedemptionCode> {
        Ok(RedemptionCode {
            id: row.get(0)?,
            code: row.get(1)?,
            amount_micros: row.get(2)?,
            plan_id: row.get(3)?,
            enabled: row.get::<_, i32>(4)? != 0,
            max_uses: row.get(5)?,
            used_count: row.get(6)?,
            expires_at: row
                .get::<_, Option<String>>(7)?
                .map(Self::parse_time)
                .transpose()?,
            created_at: Self::parse_time(row.get(8)?)?,
        })
    }
    pub fn upsert_redemption_code(&self, code: &RedemptionCode) -> SqliteResult<()> {
        self.conn.lock().execute("INSERT INTO redemption_codes (id,code,amount_micros,plan_id,enabled,max_uses,used_count,expires_at,created_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9) ON CONFLICT(id) DO UPDATE SET code=?2,amount_micros=?3,plan_id=?4,enabled=?5,max_uses=?6,expires_at=?8",params![code.id,code.code,code.amount_micros,code.plan_id,code.enabled as i32,code.max_uses,code.used_count,code.expires_at.map(|v|v.to_rfc3339()),code.created_at.to_rfc3339()])?;
        Ok(())
    }
    pub fn list_redemption_codes(&self) -> SqliteResult<Vec<RedemptionCode>> {
        let conn = self.conn.lock();
        let mut stmt=conn.prepare("SELECT id,code,amount_micros,plan_id,enabled,max_uses,used_count,expires_at,created_at FROM redemption_codes ORDER BY created_at DESC")?;
        let codes = stmt.query_map([], Self::redemption_from_row)?.collect();
        codes
    }
    pub fn delete_redemption_code(&self, id: &str) -> SqliteResult<bool> {
        Ok(self
            .conn
            .lock()
            .execute("DELETE FROM redemption_codes WHERE id=?1", params![id])?
            > 0)
    }
    pub fn redeem(&self, user_id: &str, code_value: &str) -> SqliteResult<()> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        let code=tx.query_row("SELECT id,code,amount_micros,plan_id,enabled,max_uses,used_count,expires_at,created_at FROM redemption_codes WHERE code=?1",params![code_value],Self::redemption_from_row).optional()?.ok_or(rusqlite::Error::QueryReturnedNoRows)?;
        if !code.enabled
            || code.used_count >= code.max_uses
            || code.expires_at.is_some_and(|expiry| expiry <= Utc::now())
        {
            return Err(rusqlite::Error::InvalidQuery);
        }
        let already_used: Option<i32> = tx
            .query_row(
                "SELECT 1 FROM redemption_uses WHERE redemption_code_id=?1 AND user_id=?2",
                params![code.id, user_id],
                |row| row.get(0),
            )
            .optional()?;
        if already_used.is_some() {
            return Err(rusqlite::Error::InvalidQuery);
        }
        tx.execute(
            "INSERT INTO redemption_uses (redemption_code_id,user_id,used_at) VALUES (?1,?2,?3)",
            params![code.id, user_id, Utc::now().to_rfc3339()],
        )?;
        tx.execute(
            "UPDATE redemption_codes SET used_count=used_count+1 WHERE id=?1",
            params![code.id],
        )?;
        if code.amount_micros != 0 {
            Self::credit_tx(
                &tx,
                user_id,
                code.amount_micros,
                "redemption",
                "Redemption code",
                Some(&code.id),
            )?;
        }
        if let Some(plan_id) = code.plan_id {
            let plan = tx.query_row(
                "SELECT duration_days FROM subscription_plans WHERE id=?1",
                params![plan_id],
                |row| row.get::<_, i64>(0),
            )?;
            let now = Utc::now();
            tx.execute("INSERT INTO subscriptions (id,user_id,plan_id,status,started_at,expires_at,created_at) VALUES (?1,?2,?3,'active',?4,?5,?4)",params![Uuid::new_v4().to_string(),user_id,plan_id,now.to_rfc3339(),(now+Duration::days(plan)).to_rfc3339()])?;
        }
        tx.commit()
    }

    fn order_from_row(row: &rusqlite::Row<'_>) -> SqliteResult<PaymentOrder> {
        Ok(PaymentOrder {
            id: row.get(0)?,
            user_id: row.get(1)?,
            amount_micros: row.get(2)?,
            provider: row.get(3)?,
            status: row.get(4)?,
            external_reference: row.get(5)?,
            created_at: Self::parse_time(row.get(6)?)?,
            updated_at: Self::parse_time(row.get(7)?)?,
        })
    }
    pub fn create_manual_order(&self, user_id: &str, amount: i64) -> SqliteResult<PaymentOrder> {
        if amount <= 0 {
            return Err(rusqlite::Error::InvalidQuery);
        }
        let now = Utc::now();
        let order = PaymentOrder {
            id: Uuid::new_v4().to_string(),
            user_id: user_id.into(),
            amount_micros: amount,
            provider: "manual".into(),
            status: "pending".into(),
            external_reference: None,
            created_at: now,
            updated_at: now,
        };
        self.conn.lock().execute("INSERT INTO payment_orders (id,user_id,amount_micros,provider,status,external_reference,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",params![order.id,order.user_id,order.amount_micros,order.provider,order.status,order.external_reference,now.to_rfc3339(),now.to_rfc3339()])?;
        Ok(order)
    }
    pub fn list_orders(&self) -> SqliteResult<Vec<PaymentOrder>> {
        let conn = self.conn.lock();
        let mut stmt=conn.prepare("SELECT id,user_id,amount_micros,provider,status,external_reference,created_at,updated_at FROM payment_orders ORDER BY created_at DESC")?;
        let orders = stmt.query_map([], Self::order_from_row)?.collect();
        orders
    }
    pub fn complete_manual_order(&self, id: &str) -> SqliteResult<Option<PaymentOrder>> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        let order=tx.query_row("SELECT id,user_id,amount_micros,provider,status,external_reference,created_at,updated_at FROM payment_orders WHERE id=?1",params![id],Self::order_from_row).optional()?;
        let Some(mut order) = order else {
            return Ok(None);
        };
        if order.provider != "manual" || order.status != "pending" {
            return Err(rusqlite::Error::InvalidQuery);
        };
        Self::credit_tx(
            &tx,
            &order.user_id,
            order.amount_micros,
            "manual_payment",
            "Manual payment completed",
            Some(&order.id),
        )?;
        order.status = "completed".into();
        order.updated_at = Utc::now();
        tx.execute(
            "UPDATE payment_orders SET status='completed',updated_at=?2 WHERE id=?1",
            params![order.id, order.updated_at.to_rfc3339()],
        )?;
        tx.commit()?;
        Ok(Some(order))
    }

    // ── Stats ─────────────────────────────────────────────────────────────────

    pub fn get_system_status(&self, start_time: std::time::Instant) -> SqliteResult<SystemStatus> {
        let total_channels: i64 =
            self.conn
                .lock()
                .query_row("SELECT COUNT(*) FROM channels", [], |r| r.get(0))?;
        let enabled_channels: i64 = self.conn.lock().query_row(
            "SELECT COUNT(*) FROM channels WHERE enabled=1",
            [],
            |r| r.get(0),
        )?;
        let total_requests: i64 =
            self.conn
                .lock()
                .query_row("SELECT COUNT(*) FROM request_logs", [], |r| r.get(0))?;

        let cfg = super::APP_CONFIG.read();
        Ok(SystemStatus {
            version: env!("CARGO_PKG_VERSION").to_string(),
            uptime_seconds: start_time.elapsed().as_secs(),
            total_channels,
            enabled_channels,
            total_requests,
            active_requests: 0,
            local_api_token: cfg.local_api_token.clone(),
            listen_host: cfg.listen_host.clone(),
            listen_port: cfg.listen_port,
        })
    }
}

pub struct ChannelSelector {
    db: std::sync::Arc<Database>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn database() -> Database {
        Database::new(":memory:").expect("in-memory database")
    }

    #[test]
    fn password_authentication_and_session_lookup() {
        let db = database();
        let user = db
            .create_user(
                "alice",
                "alice@example.test",
                "secure-password",
                UserRole::User,
            )
            .unwrap();
        assert!(db
            .authenticate("alice", "wrong-password")
            .unwrap()
            .is_none());
        assert_eq!(
            db.authenticate("alice", "secure-password")
                .unwrap()
                .unwrap()
                .id,
            user.id
        );
        let session = db.create_session(&user.id, Duration::hours(1)).unwrap();
        assert_eq!(
            db.session_user(&session.token).unwrap().unwrap().0.username,
            "alice"
        );
        assert!(db.revoke_session(&session.token).unwrap());
        assert!(db.session_user(&session.token).unwrap().is_none());
    }

    #[test]
    fn adjustments_cannot_make_balance_negative() {
        let db = database();
        let user = db
            .create_user("bob", "bob@example.test", "secure-password", UserRole::User)
            .unwrap();
        assert_eq!(
            db.admin_adjust_balance(&user.id, 250, "credit")
                .unwrap()
                .balance_after_micros,
            250
        );
        assert!(db
            .admin_adjust_balance(&user.id, -251, "overdraft")
            .is_err());
        assert_eq!(db.get_user(&user.id).unwrap().unwrap().balance_micros, 250);
    }

    #[test]
    fn subscription_purchase_requires_balance_and_is_atomic() {
        let db = database();
        let user = db
            .create_user(
                "buyer",
                "buyer@example.test",
                "secure-password",
                UserRole::User,
            )
            .unwrap();
        let plan = SubscriptionPlan {
            id: "starter".into(),
            name: "Starter".into(),
            description: "Quota".into(),
            price_micros: 100,
            quota_micros: 1000,
            duration_days: 30,
            enabled: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        db.upsert_plan(&plan).unwrap();
        assert!(db.subscribe(&user.id, &plan.id).is_err());
        assert_eq!(db.get_user(&user.id).unwrap().unwrap().balance_micros, 0);
        assert!(db.list_subscriptions(&user.id).unwrap().is_empty());
    }

    #[test]
    fn subscription_purchase_debits_and_replaces_prior_subscription() {
        let db = database();
        let user = db
            .create_user(
                "buyer2",
                "buyer2@example.test",
                "secure-password",
                UserRole::User,
            )
            .unwrap();
        db.admin_adjust_balance(&user.id, 250, "credit").unwrap();
        let now = Utc::now();
        let first = SubscriptionPlan {
            id: "first".into(),
            name: "First".into(),
            description: "Quota".into(),
            price_micros: 100,
            quota_micros: 1000,
            duration_days: 30,
            enabled: true,
            created_at: now,
            updated_at: now,
        };
        let second = SubscriptionPlan {
            id: "second".into(),
            name: "Second".into(),
            description: "Quota".into(),
            price_micros: 50,
            quota_micros: 2000,
            duration_days: 60,
            enabled: true,
            created_at: now,
            updated_at: now,
        };
        db.upsert_plan(&first).unwrap();
        db.upsert_plan(&second).unwrap();
        let first_subscription = db.subscribe(&user.id, &first.id).unwrap();
        let second_subscription = db.subscribe(&user.id, &second.id).unwrap();
        let subscriptions = db.list_subscriptions(&user.id).unwrap();
        assert_eq!(db.get_user(&user.id).unwrap().unwrap().balance_micros, 100);
        assert_eq!(
            subscriptions
                .iter()
                .find(|s| s.id == first_subscription.id)
                .unwrap()
                .status,
            "cancelled"
        );
        assert_eq!(
            subscriptions
                .iter()
                .find(|s| s.id == second_subscription.id)
                .unwrap()
                .status,
            "active"
        );
        assert_eq!(
            db.list_ledger(&user.id)
                .unwrap()
                .iter()
                .filter(|e| e.kind == "subscription_purchase")
                .count(),
            2
        );
    }

    #[test]
    fn redemption_enforces_single_user_use_and_maximum() {
        let db = database();
        let first = db
            .create_user(
                "carol",
                "carol@example.test",
                "secure-password",
                UserRole::User,
            )
            .unwrap();
        let second = db
            .create_user(
                "dave",
                "dave@example.test",
                "secure-password",
                UserRole::User,
            )
            .unwrap();
        let code = RedemptionCode {
            id: Uuid::new_v4().to_string(),
            code: "ONEUSE".into(),
            amount_micros: 100,
            plan_id: None,
            enabled: true,
            max_uses: 1,
            used_count: 0,
            expires_at: None,
            created_at: Utc::now(),
        };
        db.upsert_redemption_code(&code).unwrap();
        db.redeem(&first.id, "ONEUSE").unwrap();
        assert_eq!(db.get_user(&first.id).unwrap().unwrap().balance_micros, 100);
        assert!(db.redeem(&first.id, "ONEUSE").is_err());
        assert!(db.redeem(&second.id, "ONEUSE").is_err());
    }
}

impl ChannelSelector {
    pub fn new(db: std::sync::Arc<Database>) -> Self {
        Self { db }
    }

    pub fn list_enabled(&self) -> SqliteResult<Vec<Channel>> {
        self.db.get_enabled_channels()
    }

    pub fn get(&self, id: &str) -> SqliteResult<Option<Channel>> {
        self.db.get_channel(id)
    }

    /// Long-context fallback: if current channel has a model_map that maps the failing
    /// model to a longer-context target on the same channel, return that target.
    pub fn db_fallback(&self, channel: &Channel, model: &str) -> Option<String> {
        let maps = self.db.list_model_maps().ok()?;
        for m in maps {
            if m.channel_id == channel.id && m.enabled {
                if m.pattern == model || m.pattern == "*" {
                    return Some(m.target_model);
                }
            }
        }
        None
    }

    pub fn select_channel(&self, _model: &str) -> SqliteResult<Option<Channel>> {
        let channels = self.db.get_enabled_channels()?;
        if channels.is_empty() {
            return Ok(None);
        }
        let best = channels
            .into_iter()
            .max_by_key(|c| (c.priority, c.weight))
            .unwrap();
        Ok(Some(best))
    }
}
