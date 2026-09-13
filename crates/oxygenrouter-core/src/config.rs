//! App config (serde + once_cell)
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use once_cell::sync::Lazy;
use parking_lot::RwLock;
use std::path::Path;

use super::AppSettings;

pub static APP_CONFIG: Lazy<RwLock<AppSettings>> =
    Lazy::new(|| RwLock::new(AppSettings::default()));

pub fn load_config<P: AsRef<Path>>(path: P) -> anyhow::Result<()> {
    let path = path.as_ref();
    if path.exists() {
        let raw = std::fs::read_to_string(path)?;
        let cfg: AppSettings = serde_json::from_str(&raw)?;
        *APP_CONFIG.write() = cfg;
    }
    Ok(())
}

pub fn save_config<P: AsRef<Path>>(path: P) -> anyhow::Result<()> {
    let cfg = APP_CONFIG.read().clone();
    let raw = serde_json::to_string_pretty(&cfg)?;
    std::fs::write(path, raw)?;
    Ok(())
}
