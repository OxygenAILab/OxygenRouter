//! oxygenrouter-billing: the quota engine.
//!
//! Turns upstream token usage into a charge, and manages the reserve → settle →
//! refund lifecycle that keeps a client's balance honest under concurrency.
//!
//! Layout:
//! * [`quota_math`] — conversion, rounding and saturation policy
//! * [`price`] — the per-request pricing snapshot
//! * [`usage`] — dialect-preserving token details
//! * [`chat_quota`] — the chat/text charge formula
//! * [`estimator`] — offline token estimation for pre-consume, for providers
//!   whose responses carry no usage
//! * [`session`] — pre-consume / settle / refund
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

pub mod chat_quota;
pub mod estimator;
pub mod expr;
pub mod price;
pub mod pricing;
pub mod quota_math;
pub mod session;
pub mod service;
pub mod usage;

pub use chat_quota::{compute_chat_quota, ChatQuotaRequest, ChatQuotaResult};
pub use estimator::{estimate_prompt_tokens, estimate_tokens, TokenEstimator};
pub use expr::{
    build_params, evaluate, evaluate_task_usage, evaluate_with, quota_from_cost, used_usage_keys,
    used_vars, EvalContext, ExprError, ExprOutcome, TokenParams,
};
pub use price::{PriceData, QUOTA_PER_UNIT};
pub use pricing::{
    BillingMode, Pricing, PricingDocument, PricingLoadError, PricingMeta, PricingTables,
    DEFAULT_GROUP_RATIO,
};
pub use quota_math::{
    quota_from_decimal, quota_from_decimal_checked, quota_from_decimal_strict, quota_from_f64,
    quota_round, quota_round_checked, wallet_quota_from_decimal, QuotaClamp, QuotaClampKind,
    MAX_QUOTA, MAX_WALLET_QUOTA, MIN_QUOTA,
};
pub use session::{BillingError, BillingSession, BillingState, FundingSource};
pub use service::{
    BillingPath, BillingPolicy, BillingService, BillingStore, Charge,
};
pub use usage::{BillingUsage, UsageDetails, UsageSemantic};
