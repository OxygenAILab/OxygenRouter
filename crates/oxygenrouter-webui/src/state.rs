//! Shared AppState for the webui server
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use chrono::{DateTime, Utc};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{broadcast, RwLock};

use oxygenrouter_core::{Database, RequestLog};
use oxygenrouter_proxy::ChannelScheduler;

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
        Self {
            db,
            db_path,
            scheduler: Arc::new(RwLock::new(scheduler)),
            local_token,
            start_time: Instant::now(),
            started_at: Utc::now(),
            config_path,
            log_broadcast,
        }
    }

    pub async fn reload_scheduler_maps(&self) {
        let maps = self.db.list_model_maps().unwrap_or_default();
        self.scheduler.write().await.set_model_maps(maps);
    }
}
