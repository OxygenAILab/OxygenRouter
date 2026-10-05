//! oxygenrouter-core: data models, SQLite schema, config
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

//! oxygenrouter-core: data models, SQLite schema, config
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover
//!
//! # Where configuration lives
//!
//! The `settings` table is the single authority. `config.json` is a bootstrap
//! file: it supplies the bind address and the data directory, which are needed
//! before the database is open, and it is the one place a launch-time value can
//! be given on a machine where the console is not reachable yet. Everything else
//! is read from the table through [`Database::typed_setting`], so there is one
//! value per setting rather than two that disagree.

pub mod authz;
pub mod artifact_access;
mod config;
mod db;
mod models;
pub mod net;
pub mod options;
pub mod totp;

pub use config::{APP_CONFIG, load_config, save_config};
pub use db::{ApiKeyResolutionError, ChannelSelector, Database, DEFAULT_MIN_PASSWORD_LEN};
pub use models::*;

/// The task lifecycle literals, shared by the store, the poller and the plugin
/// contract, so all three compare against the same strings.
pub use oxygenrouter_plugin::{
    STATUS_FAILURE, STATUS_IN_PROGRESS, STATUS_NOT_START, STATUS_QUEUED, STATUS_SUBMITTED,
    STATUS_SUCCESS, STATUS_UNKNOWN,
};

pub use options::{
    find_schema, validate as validate_option, OptionKind, OptionSchema, OptionSection, OPTION_SCHEMA,
};
