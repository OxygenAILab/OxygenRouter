//! The billing service: engine + storage, wired into one call.
//!
//! This is the layer the HTTP handlers drive. It owns the decision of *which*
//! pricing path applies (expression vs. legacy ratios), how much to reserve
//! before an upstream call, and how to settle afterwards.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use std::sync::Arc;

use crate::chat_quota::{compute_chat_quota, ChatQuotaRequest};
use crate::expr::{build_params, evaluate_with, quota_from_cost, used_vars, EvalContext};
use crate::pricing::Pricing;
use crate::quota_math::QuotaClamp;
use crate::session::{BillingError, BillingSession, QuotaStore};
use crate::usage::BillingUsage;

/// A resolved charge, with the audit trail needed to explain it in a log.
#[derive(Debug, Clone)]
pub struct Charge {
    /// Final quota in the single-request domain.
    pub quota: i64,
    /// Which pricing path produced it.
    pub path: BillingPath,
    /// The tier name, for expression pricing.
    pub matched_tier: Option<String>,
    /// True when the model is per-request priced.
    pub per_request: bool,
    /// Saturation event, if any.
    pub clamp: Option<QuotaClamp>,
    /// Estimated prompt tokens, recorded for logs and for `tokens_used`.
    pub estimated_prompt_tokens: i64,
}

/// Which pricing path produced a charge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BillingPath {
    /// A per-model `tiered_expr` expression.
    Expression,
    /// The legacy multiplier tables.
    Ratio,
    /// The model has no price configured, so the request is free.
    Unpriced,
}

impl BillingPath {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Expression => "tiered_expr",
            Self::Ratio => "ratio",
            Self::Unpriced => "unpriced",
        }
    }
}

/// Storage the billing service needs.
///
/// Implemented by the SQLite database. Kept as a trait so the service can be
/// tested without a database and so a future Postgres backend is a drop-in.
pub trait BillingStore: Send + Sync {
    /// Atomically reserve from an API key's quota; false when exhausted.
    fn try_reserve_key(&self, key_id: &str, amount: i64) -> Result<bool, BillingError>;
    /// Reserve from a wallet; false when the balance cannot cover it.
    fn try_reserve_wallet(&self, user_id: &str, amount: i64) -> Result<bool, BillingError>;
    /// Move quota out of a key, permitted to exceed the ceiling.
    fn debit_key(&self, key_id: &str, amount: i64) -> Result<(), BillingError>;
    /// Move quota back to a key.
    fn credit_key(&self, key_id: &str, amount: i64) -> Result<(), BillingError>;
    /// Move quota out of a wallet, permitted to go negative.
    fn debit_wallet(&self, user_id: &str, amount: i64) -> Result<(), BillingError>;
    /// Move quota into a wallet.
    fn credit_wallet(&self, user_id: &str, amount: i64) -> Result<(), BillingError>;
    /// Write the settlement ledger entry.
    ///
    /// **Audit only.** Balance movement belongs to the `BillingSession`; an
    /// implementation of this method must not move money, or every request
    /// would be charged twice.
    fn record_charge(
        &self,
        user_id: &str,
        amount: i64,
        description: &str,
        reference_id: Option<&str>,
    ) -> Result<(), BillingError>;
    /// Current wallet balance, for the trust-bypass check.
    fn wallet_balance(&self, user_id: &str) -> Result<i64, BillingError>;
}

/// Split a reservation between the key and the wallet.
///
/// A `QuotaStore` addresses one account, so the service presents two: the key
/// is charged first (it has the hard ceiling) and the wallet second.
struct DualStore<'a> {
    store: &'a dyn BillingStore,
    key_id: String,
}

impl QuotaStore for DualStore<'_> {
    /// Any account name other than the key is the wallet: a `BillingSession` is
    /// only ever constructed with exactly these two accounts.
    fn try_reserve(&self, account: &str, amount: i64) -> Result<bool, BillingError> {
        if account == self.key_id {
            self.store.try_reserve_key(account, amount)
        } else {
            self.store.try_reserve_wallet(account, amount)
        }
    }
    fn debit(&self, account: &str, amount: i64) -> Result<(), BillingError> {
        if account == self.key_id {
            self.store.debit_key(account, amount)
        } else {
            self.store.debit_wallet(account, amount)
        }
    }
    fn credit(&self, account: &str, amount: i64) -> Result<(), BillingError> {
        if account == self.key_id {
            self.store.credit_key(account, amount)
        } else {
            self.store.credit_wallet(account, amount)
        }
    }
}

/// Thresholds that change billing behaviour.
#[derive(Debug, Clone)]
pub struct BillingPolicy {
    /// Wallet balance above which reservation is skipped (trust bypass).
    pub trust_quota: i64,
    /// Multiplier applied to the prompt estimate when reserving. `1.0` reserves
    /// exactly the estimate; higher values guard against under-estimation.
    pub pre_consume_multiplier: f64,
    /// Reserve even when the model's price resolves to zero.
    pub pre_consume_free_models: bool,
    /// Assumed completion length when the request does not cap it.
    pub assumed_completion_tokens: i64,
}

impl Default for BillingPolicy {
    fn default() -> Self {
        Self {
            trust_quota: 10_000_000,
            pre_consume_multiplier: 1.0,
            pre_consume_free_models: false,
            assumed_completion_tokens: 500,
        }
    }
}

/// The billing service.
pub struct BillingService {
    pricing: Arc<parking_lot::RwLock<Pricing>>,
    policy: BillingPolicy,
}

impl BillingService {
    pub fn new(pricing: Pricing, policy: BillingPolicy) -> Self {
        Self {
            pricing: Arc::new(parking_lot::RwLock::new(pricing)),
            policy,
        }
    }

    pub fn pricing(&self) -> Arc<parking_lot::RwLock<Pricing>> {
        Arc::clone(&self.pricing)
    }

    pub fn policy(&self) -> &BillingPolicy {
        &self.policy
    }

    /// Compute the charge for a completed request.
    ///
    /// `usage` is what the upstream actually reported (or our estimate when it
    /// reported nothing).
    pub fn charge(
        &self,
        model: &str,
        group: &str,
        usage: &BillingUsage,
        ctx: &EvalContext,
    ) -> Charge {
        let pricing = self.pricing.read();
        let quota_per_unit = pricing.quota_per_unit();
        let group_ratio = pricing.group_ratio(group);

        if let Some(expression) = pricing.expression(model) {
            let used = used_vars(expression);
            let params = build_params(usage, &used);
            match evaluate_with(expression, &params, ctx) {
                Ok(outcome) => {
                    let (quota, clamp) =
                        quota_from_cost(outcome.cost_usd, quota_per_unit, group_ratio);
                    return Charge {
                        quota,
                        path: BillingPath::Expression,
                        matched_tier: outcome.matched_tier,
                        per_request: outcome.billing_unit_request,
                        clamp,
                        estimated_prompt_tokens: usage.prompt_tokens,
                    };
                }
                Err(error) => {
                    // A broken expression must not silently bill zero: the
                    // request was served, so fall through to the ratio tables as
                    // the safest estimation, and the error surfaces in the log.
                    let fallback = format!("expression error: {}", error);
                    let price = pricing.price_data(model, group);
                    let result = compute_chat_quota(&ChatQuotaRequest {
                        usage: usage.clone(),
                        price,
                        is_stream: false,
                    });
                    return Charge {
                        quota: result.quota,
                        path: BillingPath::Ratio,
                        matched_tier: Some(fallback),
                        per_request: false,
                        clamp: result.clamp,
                        estimated_prompt_tokens: usage.prompt_tokens,
                    };
                }
            }
        }

        if pricing.is_unpriced(model) {
            return Charge {
                quota: 0,
                path: BillingPath::Unpriced,
                matched_tier: None,
                per_request: false,
                clamp: None,
                estimated_prompt_tokens: usage.prompt_tokens,
            };
        }

        let price = pricing.price_data(model, group);
        let result = compute_chat_quota(&ChatQuotaRequest {
            usage: usage.clone(),
            price: price.clone(),
            is_stream: false,
        });
        Charge {
            quota: result.quota,
            path: if price.use_price {
                BillingPath::Ratio
            } else {
                BillingPath::Ratio
            },
            matched_tier: None,
            per_request: price.use_price,
            clamp: result.clamp,
            estimated_prompt_tokens: usage.prompt_tokens,
        }
    }

    /// Quota to reserve before calling upstream.
    ///
    /// The prompt side is estimated and the completion side assumed, because
    /// neither is known yet. Reserving too little only means the shortfall is
    /// debited at settlement; reserving too much would reject a request the
    /// caller could afford, so the estimate is deliberately conservative.
    pub fn reservation(
        &self,
        model: &str,
        group: &str,
        body: &serde_json::Value,
        request_path: &str,
    ) -> i64 {
        let estimated_prompt = crate::estimator::estimate_prompt_tokens(model, body);
        let requested_max = body
            .get("max_tokens")
            .or_else(|| body.get("max_completion_tokens"))
            .or_else(|| body.get("max_output_tokens"))
            .and_then(|v| v.as_i64())
            .filter(|v| *v > 0)
            .unwrap_or(self.policy.assumed_completion_tokens);

        let usage = BillingUsage {
            prompt_tokens: estimated_prompt,
            completion_tokens: requested_max,
            semantic: match request_path {
                p if p.starts_with("/v1/messages") => crate::usage::UsageSemantic::Anthropic,
                _ => crate::usage::UsageSemantic::OpenAi,
            },
            ..Default::default()
        };

        let charge = self.charge(model, group, &usage, &EvalContext::default());
        let scaled = (charge.quota as f64 * self.policy.pre_consume_multiplier).ceil();
        if scaled <= 0.0 {
            return 0;
        }
        if !self.policy.pre_consume_free_models && charge.path == BillingPath::Unpriced {
            return 0;
        }
        scaled.min(crate::quota_math::MAX_QUOTA as f64) as i64
    }

    /// Begin a billing session for one request.
    ///
    /// Returns the session and the amount actually reserved.
    pub fn begin(
        &self,
        store: &dyn BillingStore,
        key_id: &str,
        user_id: &str,
        is_playground: bool,
        reservation: i64,
    ) -> Result<(BillingSession, i64), BillingError> {
        // Trust bypass: a well-funded account skips reservation entirely, which
        // keeps the hot path free of write contention.
        let trusted = if is_playground {
            false
        } else {
            store
                .wallet_balance(user_id)
                .map(|b| b > self.policy.trust_quota)
                .unwrap_or(false)
        };

        let session =
            BillingSession::new(key_id.to_string(), user_id.to_string(), is_playground)
                .trusted(trusted);

        let dual = DualStore {
            store,
            key_id: key_id.to_string(),
        };
        let reserved = session.pre_consume(&dual, reservation).map_err(|error| {
            // Turn the unset placeholder into the real balance so the client
            // message is actionable instead of "available unknown".
            match error {
                BillingError::InsufficientBalance { needed, available }
                    if available < 0 =>
                {
                    let actual = store.wallet_balance(user_id).unwrap_or(0);
                    BillingError::InsufficientBalance {
                        needed,
                        available: actual,
                    }
                }
                other => other,
            }
        })?;
        Ok((session, reserved))
    }

    /// Settle a completed request.
    pub fn settle(
        &self,
        store: &dyn BillingStore,
        session: &BillingSession,
        key_id: &str,
        user_id: &str,
        charge: &Charge,
        description: &str,
        reference_id: Option<&str>,
    ) -> Result<(), BillingError> {
        let dual = DualStore {
            store,
            key_id: key_id.to_string(),
        };
        session.note_clamp(charge.clamp.clone());
        // The session owns every balance movement (it holds the reservation and
        // applies the delta), so the ledger write must be audit-only. Writing
        // the amount through a balance-mutating path here would charge twice.
        session.settle(&dual, charge.quota)?;
        store.record_charge(user_id, -charge.quota, description, reference_id)?;
        Ok(())
    }

    /// Refund a failed request.
    /// Return a failed request's reservation.
    ///
    /// `user_id` is accepted for symmetry with `settle` and for callers that log
    /// the refund; the session already holds both account names, so the refund
    /// itself does not need it.
    pub fn refund(
        &self,
        store: &dyn BillingStore,
        session: &BillingSession,
        key_id: &str,
        _user_id: &str,
    ) -> Result<(), BillingError> {
        let dual = DualStore {
            store,
            key_id: key_id.to_string(),
        };
        session.refund(&dual)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pricing::Pricing;
    use parking_lot::Mutex;
    use std::collections::HashMap;

    #[derive(Default)]
    struct MemStore {
        keys: Mutex<HashMap<String, i64>>,
        wallets: Mutex<HashMap<String, i64>>,
        ledger: Mutex<Vec<(String, i64)>>,
    }

    impl MemStore {
        fn new(key_quota: i64, wallet: i64) -> Self {
            let s = Self::default();
            s.keys.lock().insert("key".into(), key_quota);
            s.wallets.lock().insert("user".into(), wallet);
            s
        }
        fn wallet(&self) -> i64 {
            *self.wallets.lock().get("user").unwrap_or(&0)
        }
        fn key_used(&self) -> i64 {
            *self.keys.lock().get("key").unwrap_or(&0)
        }
    }

    impl BillingStore for MemStore {
        fn try_reserve_key(&self, key: &str, amount: i64) -> Result<bool, BillingError> {
            let mut k = self.keys.lock();
            let v = k.entry(key.to_string()).or_insert(0);
            if *v < amount {
                return Ok(false);
            }
            *v -= amount;
            Ok(true)
        }
        fn try_reserve_wallet(&self, user: &str, amount: i64) -> Result<bool, BillingError> {
            let mut w = self.wallets.lock();
            let v = w.entry(user.to_string()).or_insert(0);
            if *v < amount {
                return Ok(false);
            }
            *v -= amount;
            Ok(true)
        }
        fn debit_key(&self, key: &str, amount: i64) -> Result<(), BillingError> {
            *self.keys.lock().entry(key.to_string()).or_insert(0) -= amount;
            Ok(())
        }
        fn credit_key(&self, key: &str, amount: i64) -> Result<(), BillingError> {
            *self.keys.lock().entry(key.to_string()).or_insert(0) += amount;
            Ok(())
        }
        fn debit_wallet(&self, user: &str, amount: i64) -> Result<(), BillingError> {
            *self.wallets.lock().entry(user.to_string()).or_insert(0) -= amount;
            Ok(())
        }
        fn credit_wallet(&self, user: &str, amount: i64) -> Result<(), BillingError> {
            *self.wallets.lock().entry(user.to_string()).or_insert(0) += amount;
            Ok(())
        }
        fn record_charge(
            &self,
            _user: &str,
            amount: i64,
            _desc: &str,
            _reference: Option<&str>,
        ) -> Result<(), BillingError> {
            self.ledger.lock().push(("consume".into(), amount));
            Ok(())
        }
        fn wallet_balance(&self, user: &str) -> Result<i64, BillingError> {
            Ok(*self.wallets.lock().get(user).unwrap_or(&0))
        }
    }

    fn service() -> BillingService {
        BillingService::new(
            Pricing::from_embedded().expect("embedded pack"),
            BillingPolicy {
                trust_quota: i64::MAX, // disable trust bypass for deterministic tests
                ..Default::default()
            },
        )
    }

    #[test]
    fn expression_charge_matches_the_engine() {
        let service = service();
        let usage = BillingUsage {
            prompt_tokens: 1000,
            completion_tokens: 100,
            ..Default::default()
        };
        let charge = service.charge("Ling-1T", "default", &usage, &EvalContext::default());
        assert_eq!(charge.path, BillingPath::Expression);
        assert!(charge.quota > 0);
        assert!(charge.matched_tier.is_some());
    }

    #[test]
    fn unpriced_model_is_free() {
        let service = service();
        let usage = BillingUsage {
            prompt_tokens: 1000,
            completion_tokens: 100,
            ..Default::default()
        };
        let charge = service.charge(
            "definitely-not-a-real-model",
            "default",
            &usage,
            &EvalContext::default(),
        );
        assert_eq!(charge.quota, 0);
        assert_eq!(charge.path, BillingPath::Unpriced);
    }

    #[test]
    fn reservation_is_positive_for_a_priced_model() {
        let service = service();
        let body = serde_json::json!({
            "messages": [{"role": "user", "content": "hello world"}],
            "max_tokens": 100
        });
        let r = service.reservation("Ling-1T", "default", &body, "/v1/chat/completions");
        assert!(r > 0, "reservation was {r}");
    }

    #[test]
    fn reservation_accounts_for_max_tokens() {
        let service = service();
        let small = serde_json::json!({
            "messages": [{"role":"user","content":"hi"}], "max_tokens": 1
        });
        let large = serde_json::json!({
            "messages": [{"role":"user","content":"hi"}], "max_tokens": 100000
        });
        let a = service.reservation("Ling-1T", "default", &small, "/v1/chat/completions");
        let b = service.reservation("Ling-1T", "default", &large, "/v1/chat/completions");
        assert!(b > a, "larger max_tokens must reserve more: {a} vs {b}");
    }

    #[test]
    fn reservation_is_zero_for_an_unpriced_model() {
        let service = service();
        let body = serde_json::json!({"messages":[{"role":"user","content":"hi"}],"max_tokens":100});
        assert_eq!(
            service.reservation("not-a-model-xyz", "default", &body, "/v1/chat/completions"),
            0
        );
    }

    #[test]
    fn full_lifecycle_debits_the_wallet() {
        let service = service();
        let store = MemStore::new(1_000_000, 1_000_000);
        let body = serde_json::json!({"messages":[{"role":"user","content":"hello"}],"max_tokens":100});

        let reservation = service.reservation("Ling-1T", "default", &body, "/v1/chat/completions");
        let (session, reserved) = service
            .begin(&store, "key", "user", false, reservation)
            .expect("begin");
        assert_eq!(reserved, reservation);
        assert!(store.wallet() < 1_000_000, "wallet must be reserved against");

        let usage = BillingUsage {
            prompt_tokens: 10,
            completion_tokens: 5,
            ..Default::default()
        };
        let charge = service.charge("Ling-1T", "default", &usage, &EvalContext::default());
        service
            .settle(&store, &session, "key", "user", &charge, "test", None)
            .expect("settle");

        assert_eq!(store.wallet(), 1_000_000 - charge.quota);
    }

    #[test]
    fn failed_request_is_refunded_in_full() {
        let service = service();
        let store = MemStore::new(1_000_000, 1_000_000);
        let body = serde_json::json!({"messages":[{"role":"user","content":"hello"}],"max_tokens":100});
        let reservation = service.reservation("Ling-1T", "default", &body, "/v1/chat/completions");
        let (session, _) = service.begin(&store, "key", "user", false, reservation).unwrap();

        service.refund(&store, &session, "key", "user").unwrap();
        assert_eq!(store.wallet(), 1_000_000);
    }

    #[test]
    fn insufficient_wallet_rejects_the_request() {
        let service = service();
        let store = MemStore::new(1_000_000, 10);
        let body = serde_json::json!({"messages":[{"role":"user","content":"hello"}],"max_tokens":1000});
        let reservation = service.reservation("Ling-1T", "default", &body, "/v1/chat/completions");
        assert!(reservation > 10);
        assert!(service.begin(&store, "key", "user", false, reservation).is_err());
        // The key quota must not have been consumed by the failed attempt.
        assert_eq!(store.key_used(), 1_000_000);
        assert_eq!(store.wallet(), 10);
    }

    #[test]
    fn settlement_overrunning_the_reservation_overdraws_the_wallet() {
        let service = service();
        let store = MemStore::new(1_000_000, 1_000);
        let reservation = 100;
        let (session, _) = service.begin(&store, "key", "user", false, reservation).unwrap();

        let usage = BillingUsage {
            prompt_tokens: 1_000_000,
            completion_tokens: 100_000,
            ..Default::default()
        };
        let charge = service.charge("Ling-1T", "default", &usage, &EvalContext::default());
        assert!(charge.quota > reservation);
        service
            .settle(&store, &session, "key", "user", &charge, "big", None)
            .unwrap();
        assert_eq!(store.wallet(), 1_000 - charge.quota);
        assert!(store.wallet() < 0, "wallet should absorb the debt");
    }

    #[test]
    fn group_ratio_scales_the_charge() {
        let mut pricing = Pricing::from_embedded().unwrap();
        pricing.set_group_ratio("vip", 0.5);
        let service = BillingService::new(
            pricing,
            BillingPolicy {
                trust_quota: i64::MAX,
                ..Default::default()
            },
        );
        let usage = BillingUsage {
            prompt_tokens: 1000,
            completion_tokens: 100,
            ..Default::default()
        };
        let base = service.charge("Ling-1T", "default", &usage, &EvalContext::default());
        let discounted = service.charge("Ling-1T", "vip", &usage, &EvalContext::default());
        assert!(
            discounted.quota < base.quota,
            "discounted {} should be under base {}",
            discounted.quota,
            base.quota
        );
    }

    #[test]
    fn per_request_models_set_the_flag() {
        let service = service();
        let usage = BillingUsage::default();
        let charge = service.charge("dall-e-3", "default", &usage, &EvalContext::default());
        assert!(charge.per_request, "dall-e-3 is per-request priced");
        assert!(charge.quota > 0, "a per-request price must charge with zero tokens");
    }
}
