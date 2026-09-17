//! SSE stream translators and parsers.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

pub mod claude;
pub mod gemini;
pub mod openai;

pub use claude::parse_sse_frames;
