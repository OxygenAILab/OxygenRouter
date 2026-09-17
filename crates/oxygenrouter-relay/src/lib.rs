//! oxygenrouter-relay: provider adapter layer.
//!
//! Wire-format conversion (OpenAI <-> Anthropic <-> Gemini), SSE stream
//! translation, per-provider auth conventions, and SigV4 signing — the engine
//! that makes `Channel.provider` real.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

pub mod adaptor;
pub mod adapters;
pub mod channel_type;
pub mod convert;
pub mod error;
pub mod registry;
pub mod retry;
pub mod sigv4;
pub mod sse;
pub mod usage;
pub mod value;

pub use adaptor::{AdaptedResponse, Adaptor, UpstreamResponse};
pub use channel_type::{channel_type_to_api_type, provider_str_to_channel_type, ApiType, ChannelType};
pub use error::RelayError;
pub use registry::{get_adaptor, supported_names};
pub use value::{RelayFormat, RelayInfo, RelayValue, Usage};
