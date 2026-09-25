//! oxygenrouter-proxy: HTTP proxy, channel scheduler, router, upstream client
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

mod client;
mod dispatch;
pub mod limits;
mod router;
mod scheduler;
pub mod selection;
pub mod ssrf;
mod tester;
mod upstream;

pub use client::UpstreamClient;
pub use dispatch::{relay_format_for_path, RelayClient, RelayOutcome};
pub use router::{ModelRouter, RouteDecision};
pub use scheduler::ChannelScheduler;
pub use tester::test_channel;
pub use upstream::{ModelContextExceeded, ProxyError, ProxyRequest, ProxyResult};
/// Re-exported so downstream crates can name the usage type without depending on
/// `oxygenrouter-relay` directly.
pub use oxygenrouter_relay::Usage;
