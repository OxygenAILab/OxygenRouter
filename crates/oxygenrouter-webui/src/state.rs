//! Shared AppState for the webui server
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use chrono::{DateTime, Utc};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{broadcast, RwLock};

use oxygenrouter_billing::{BillingPolicy, BillingService, Pricing};
use oxygenrouter_core::{Database, RequestLog};
use oxygenrouter_proxy::ChannelScheduler;

use crate::billing_store::SqliteBillingStore;

const LOG_BROADCAST_CAPACITY: usize = 100;

pub struct AppState {
    pub db: Arc<Database>,
    pub db_path: PathBuf,
    pub scheduler: Arc<RwLock<ChannelScheduler>>,
    pub local_token: String,
    pub start_time: Instant,
    pub started_at: DateTime<Utc>,
    pub config_path: PathBuf,
    pub log_broadcast: broadcast::Sender<RequestLog>,
    /// The quota engine, shared for cheap access on the hot path.
    pub billing: Arc<BillingService>,
    /// Billing's view of storage. Held as a concrete adapter so the trait object
    /// is constructed once rather than per request.
    pub billing_store: Arc<SqliteBillingStore>,
}

impl AppState {
    pub fn new(
        db: Arc<Database>,
        db_path: PathBuf,
        scheduler: ChannelScheduler,
        local_token: String,
        config_path: PathBuf,
    ) -> Self {
        let (log_broadcast, _) = broadcast::channel(LOG_BROADCAST_CAPACITY);

        // Pricing starts from the shipped pack; `reload_pricing` then applies the
        // instance's own option overrides, mirroring GetBillingExpr's precedence.
        let pricing = Pricing::from_embedded().unwrap_or_else(|e| {
            eprintln!("[OxygenRouter] pricing pack unusable ({e}); all models will bill as unpriced");
            Pricing::new(Default::default())
        });

        Self {
            db: Arc::clone(&db),
            db_path,
            scheduler: Arc::new(RwLock::new(scheduler)),
            local_token,
            start_time: Instant::now(),
            started_at: Utc::now(),
            config_path,
            log_broadcast,
            billing: Arc::new(BillingService::new(pricing, BillingPolicy::default())),
            billing_store: Arc::new(SqliteBillingStore::new(db)),
        }
    }

    pub async fn reload_scheduler_maps(&self) {
        let maps = self.db.list_model_maps().unwrap_or_default();
        self.scheduler.write().await.set_model_maps(maps);
    }

    /// Re-read pricing overrides from the option store.
    ///
    /// Called at startup and after an admin edits pricing, so a running instance
    /// picks up new expressions without a restart.
    pub fn reload_pricing(&self) {
        use oxygenrouter_billing::BillingMode;
        use std::collections::HashMap;

        // Bind the Arc first: `self.billing.pricing()` returns an owned Arc, and
        // taking a write guard from a temporary would drop it immediately.
        let pricing_handle = self.billing.pricing();
        let mut pricing = pricing_handle.write();

        if let Ok(Some(raw)) = self.db.get_setting("billing_expr") {
            match serde_json::from_str::<HashMap<String, String>>(&raw) {
                Ok(map) => pricing.set_expr_map(map),
                Err(e) => eprintln!("[OxygenRouter] billing_expr override ignored: {e}"),
            }
        }
        if let Ok(Some(raw)) = self.db.get_setting("billing_mode") {
            match serde_json::from_str::<HashMap<String, BillingMode>>(&raw) {
                Ok(map) => pricing.set_mode_map(map),
                Err(e) => eprintln!("[OxygenRouter] billing_mode override ignored: {e}"),
            }
        }
        if let Ok(Some(raw)) = self.db.get_setting("QuotaPerUnit") {
            if let Ok(value) = raw.trim().parse::<f64>() {
                pricing.set_quota_per_unit(value);
            }
        }
        if let Ok(Some(raw)) = self.db.get_setting("GroupRatio") {
            match serde_json::from_str::<HashMap<String, f64>>(&raw) {
                Ok(map) => {
                    for (group, ratio) in map {
                        pricing.set_group_ratio(&group, ratio);
                    }
                }
                Err(e) => eprintln!("[OxygenRouter] GroupRatio override ignored: {e}"),
            }
        }
    }
}
