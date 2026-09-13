//! oxygenrouter-core: data models, SQLite schema, config
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

mod config;
mod db;
mod models;
mod options;

pub use config::{APP_CONFIG, load_config, save_config};
pub use db::{ApiKeyResolutionError, ChannelSelector, Database};
pub use models::*;
pub use options::{
    find_schema, validate as validate_option, OptionKind, OptionSchema, OptionSection, OPTION_SCHEMA,
};
