//! oxygenrouter-core: data models, SQLite schema, config
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

pub mod authz;
mod config;
mod db;
mod models;
pub mod net;
mod options;
pub mod totp;

pub use config::{APP_CONFIG, load_config, save_config};
pub use db::{ApiKeyResolutionError, ChannelSelector, Database, DEFAULT_MIN_PASSWORD_LEN};
pub use models::*;
pub use options::{
    find_schema, validate as validate_option, OptionKind, OptionSchema, OptionSection, OPTION_SCHEMA,
};
