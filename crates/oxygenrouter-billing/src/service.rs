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
use crate::session::{BillingError, BillingSession, FundingSource, QuotaStore};
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

    /// Reserve up to `amount` from a subscription's pool, returning how much was
    /// taken. Partial by design: the caller covers the remainder from the wallet.
    fn try_reserve_subscription(&self, id: &str, amount: i64) -> Result<i64, BillingError>;
    /// Move quota out of a subscription's pool, permitted to exceed its ceiling
    /// (the overflow is the wallet's debt, recorded there).
    fn debit_subscription(&self, id: &str, amount: i64) -> Result<(), BillingError>;
    /// Move quota back into a subscription's pool.
    fn credit_subscription(&self, id: &str, amount: i64) -> Result<(), BillingError>;
    /// Return up to `amount` to a subscription's pool, reporting how much fitted.
    ///
    /// Separate from [`Self::credit_subscription`] because a refund must know how
    /// much the pool could absorb: anything it cannot take belongs to the wallet,
    /// which funded that part in the first place.
    fn restore_subscription_quota(&self, id: &str, amount: i64) -> Result<i64, BillingError>;
}

/// Which account a name addresses, for the store split.
///
/// A `QuotaStore` is given account *names*, but the three kinds of account need
/// different handling: a key has a hard ceiling, a wallet may go negative, and a
/// subscription pool is finite with the wallet behind it. Classifying by name is
/// what keeps `BillingSession` free of storage knowledge.
#[derive(Debug, Clone, PartialEq, Eq)]
enum AccountKind {
    Key,
    Subscription { overflow_user: String },
    Wallet,
}

/// Split a reservation between the key and whichever funding account pays.
struct DualStore<'a> {
    store: &'a dyn BillingStore,
    key_id: String,
    /// The funding account name, when it is a subscription pool.
    subscription_id: Option<String>,
    /// The wallet that absorbs a subscription's overflow.
    overflow_user: Option<String>,
}

impl<'a> DualStore<'a> {
    fn new(store: &'a dyn BillingStore, key_id: &str) -> Self {
        Self {
            store,
            key_id: key_id.to_string(),
            subscription_id: None,
            overflow_user: None,
        }
    }

    /// Attach a subscription funding source, so its account name resolves to the
    /// pool with `user_id` behind it.
    fn with_subscription(mut self, subscription_id: &str, user_id: &str) -> Self {
        self.subscription_id = Some(subscription_id.to_string());
        self.overflow_user = Some(user_id.to_string());
        self
    }

    fn classify(&self, account: &str) -> AccountKind {
        if account == self.key_id {
            return AccountKind::Key;
        }
        if self.subscription_id.as_deref() == Some(account) {
            return AccountKind::Subscription {
                overflow_user: self.overflow_user.clone().unwrap_or_default(),
            };
        }
        AccountKind::Wallet
    }
}

impl QuotaStore for DualStore<'_> {
    fn try_reserve(&self, account: &str, amount: i64) -> Result<bool, BillingError> {
        match self.classify(account) {
            AccountKind::Key => self.store.try_reserve_key(account, amount),
            AccountKind::Wallet => self.store.try_reserve_wallet(account, amount),
            AccountKind::Subscription { overflow_user } => {
                // Take what the pool has, then the remainder from the wallet.
                let taken = self.store.try_reserve_subscription(account, amount)?;
                let remainder = amount - taken;
                if remainder == 0 {
                    return Ok(true);
                }
                match self.store.try_reserve_wallet(&overflow_user, remainder) {
                    Ok(true) => Ok(true),
                    // The wallet cannot cover the rest: give the pool's part back
                    // so a refused request leaves no trace on either account.
                    Ok(false) => {
                        self.store.credit_subscription(account, taken)?;
                        Ok(false)
                    }
                    Err(error) => {
                        self.store.credit_subscription(account, taken)?;
                        Err(error)
                    }
                }
            }
        }
    }
    fn debit(&self, account: &str, amount: i64) -> Result<(), BillingError> {
        match self.classify(account) {
            AccountKind::Key => self.store.debit_key(account, amount),
            AccountKind::Wallet => self.store.debit_wallet(account, amount),
            AccountKind::Subscription { overflow_user } => {
                // A settlement beyond the pool is the wallet's debt.
                let taken = self.store.try_reserve_subscription(account, amount)?;
                let remainder = amount - taken;
                if remainder > 0 {
                    self.store.debit_wallet(&overflow_user, remainder)?;
                }
                Ok(())
            }
        }
    }
    fn credit(&self, account: &str, amount: i64) -> Result<(), BillingError> {
        match self.classify(account) {
            AccountKind::Key => self.store.credit_key(account, amount),
            AccountKind::Wallet => self.store.credit_wallet(account, amount),
            AccountKind::Subscription { overflow_user } => {
                // Refund into the pool first (so an expired-soon plan is made
                // whole), and only push into the wallet what the pool refuses.
                let restored = self.store.restore_subscription_quota(account, amount)?;
                let remainder = amount - restored;
                if remainder > 0 {
                    self.store.credit_wallet(&overflow_user, remainder)?;
                }
                Ok(())
            }
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
    /// Smallest reservation this instance will hold, in quota micros.
    ///
    /// `0` disables the floor. The console calls this `PreConsumedQuota`; the
    /// reference's constant of the same name is the amount held for a request
    /// whose price is not yet known, so taking it as a minimum keeps the field
    /// meaningful without over-reserving a genuinely cheap model.
    pub min_pre_consume: i64,
    /// Assumed completion length when the request does not cap it.
    pub assumed_completion_tokens: i64,
}

impl Default for BillingPolicy {
    fn default() -> Self {
        Self {
            trust_quota: 10_000_000,
            pre_consume_multiplier: 1.0,
            pre_consume_free_models: false,
            // Off by default: a floor changes what every request holds back, and
            // the shipped defaults must keep behaving exactly as before.
            min_pre_consume: 0,
            assumed_completion_tokens: 500,
        }
    }
}

/// The billing service.
pub struct BillingService {
    pricing: Arc<parking_lot::RwLock<Pricing>>,
    /// Read on every charge and every reservation, so it lives behind the same
    /// kind of lock `pricing` does rather than being copied into the struct at
    /// construction. A plain field would make the thresholds frozen at startup:
    /// the instance's configured reserve and trust values could not be applied to
    /// a running process at all.
    policy: Arc<parking_lot::RwLock<BillingPolicy>>,
}

impl BillingService {
    pub fn new(pricing: Pricing, policy: BillingPolicy) -> Self {
        Self {
            pricing: Arc::new(parking_lot::RwLock::new(pricing)),
            policy: Arc::new(parking_lot::RwLock::new(policy)),
        }
    }

    pub fn pricing(&self) -> Arc<parking_lot::RwLock<Pricing>> {
        Arc::clone(&self.pricing)
    }

    /// Replace the policy in place, so a live instance picks up new thresholds.
    pub fn set_policy(&self, policy: BillingPolicy) {
        *self.policy.write() = policy;
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
            .unwrap_or(self.policy.read().assumed_completion_tokens);

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
        // One read of the policy for the whole decision, so the multiplier and
        // the free-model rule cannot be observed half-updated.
        let policy = self.policy.read().clone();
        let scaled = (charge.quota as f64 * policy.pre_consume_multiplier).ceil();
        if charge.path == BillingPath::Unpriced {
            // A model with no price has nothing to hold back, so the free-model
            // switch is the only thing that can make it reserve — and its whole
            // purpose is to hold the standard amount anyway. This branch used to
            // be dead: an unpriced charge has `quota == 0`, which tripped an
            // earlier `scaled <= 0.0` return, so the option could never do
            // anything at all.
            if !policy.pre_consume_free_models || policy.min_pre_consume <= 0 {
                return 0;
            }
            return policy.min_pre_consume;
        }
        if scaled <= 0.0 {
            return 0;
        }
        let amount = (scaled.min(crate::quota_math::MAX_QUOTA as f64) as i64)
            .max(policy.min_pre_consume.max(0));
        amount
    }

    /// Begin a billing session for one request.
    ///
    /// Returns the session and the amount actually reserved.
    pub fn begin(
        &self,
        store: &dyn BillingStore,
        key_id: &str,
        funding: &FundingSource,
        is_playground: bool,
        reservation: i64,
    ) -> Result<(BillingSession, i64), BillingError> {
        // The wallet behind this request, for the trust check and for reporting a
        // real balance when a reservation is refused.
        let user_id = match funding {
            FundingSource::Wallet { user_id } => user_id.as_str(),
            FundingSource::Subscription { user_id, .. } => user_id.as_str(),
        };
        // Trust bypass: a well-funded account skips reservation entirely, which
        // keeps the hot path free of write contention.
        let trusted = if is_playground {
            false
        } else {
            store
                .wallet_balance(user_id)
                .map(|b| b > self.policy.read().trust_quota)
                .unwrap_or(false)
        };

        let session = BillingSession::new(
            key_id.to_string(),
            funding.account().to_string(),
            is_playground,
        )
        .trusted(trusted);

        let dual = match funding {
            FundingSource::Wallet { .. } => DualStore::new(store, key_id),
            FundingSource::Subscription {
                user_id,
                subscription_id,
            } => DualStore::new(store, key_id).with_subscription(subscription_id, user_id),
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
        funding: &FundingSource,
    ) -> Result<(), BillingError> {
        let dual = match funding {
            FundingSource::Wallet { .. } => DualStore::new(store, key_id),
            FundingSource::Subscription {
                user_id: payer,
                subscription_id,
            } => DualStore::new(store, key_id).with_subscription(subscription_id, payer),
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
        funding: &FundingSource,
    ) -> Result<(), BillingError> {
        let dual = match funding {
            FundingSource::Wallet { .. } => DualStore::new(store, key_id),
            FundingSource::Subscription {
                user_id: payer,
                subscription_id,
            } => DualStore::new(store, key_id).with_subscription(subscription_id, payer),
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
        /// Subscription pools: `(total, used)`. Absent means unlimited.
        subscriptions: Mutex<HashMap<String, Pool>>,
    }

    #[derive(Debug, Clone, Copy)]
    struct Pool {
        total: i64,
        used: i64,
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
        /// Give `id` a finite pool, so the subscription path has something to
        /// draw from.
        fn with_subscription(&self, id: &str, total: i64) -> &Self {
            self.subscriptions
                .lock()
                .insert(id.to_string(), Pool { total, used: 0 });
            self
        }
        fn pool_used(&self, id: &str) -> i64 {
            self.subscriptions.lock().get(id).map(|p| p.used).unwrap_or(0)
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

        // The in-memory store predates subscriptions; these exercise the
        // subscription path without a database. `0` total means unlimited, so a
        // reservation takes whatever is asked and records nothing, matching the
        // SQLite store's semantics.
        fn try_reserve_subscription(&self, id: &str, amount: i64) -> Result<i64, BillingError> {
            // Must compute *and* mark the reservation, like the SQLite store:
            // a read-only simulation would let every request think the pool is
            // untouched.
            let mut pools = self.subscriptions.lock();
            match pools.get_mut(id) {
                None => Ok(amount),
                Some(pool) => {
                    if pool.total <= 0 {
                        // Unlimited: nothing recorded.
                        return Ok(amount);
                    }
                    let taken = ((pool.total - pool.used).max(0)).min(amount);
                    pool.used += taken;
                    Ok(taken)
                }
            }
        }
        fn debit_subscription(&self, id: &str, amount: i64) -> Result<(), BillingError> {
            let mut pools = self.subscriptions.lock();
            if let Some(pool) = pools.get_mut(id) {
                pool.used += amount;
            }
            Ok(())
        }
        fn credit_subscription(&self, id: &str, amount: i64) -> Result<(), BillingError> {
            let mut pools = self.subscriptions.lock();
            if let Some(pool) = pools.get_mut(id) {
                pool.used = (pool.used - amount).max(0);
            }
            Ok(())
        }
        fn restore_subscription_quota(&self, id: &str, amount: i64) -> Result<i64, BillingError> {
            let mut pools = self.subscriptions.lock();
            match pools.get_mut(id) {
                None => Ok(0),
                Some(pool) => {
                    // Bounded by what was used, matching the SQLite store: a
                    // restore reduces `used`, so the pool's *room* is the wrong
                    // bound (it would restore nothing from a drained pool).
                    let restored = pool.used.max(0).min(amount);
                    pool.used -= restored;
                    Ok(restored)
                }
            }
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

    /// The same baseline `service()` uses, for tests that replace the policy.
    fn service_policy() -> BillingPolicy {
        BillingPolicy {
            trust_quota: i64::MAX,
            ..Default::default()
        }
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

    /// `PreConsumedQuota`, `FreeModelPreConsumeEnabled` and `TrustQuota` were
    /// advertised and read by nothing, so the engine always ran on its shipped
    /// defaults. These pin the three behaviours the options are supposed to buy.
    #[test]
    fn the_billing_policy_options_actually_move_the_thresholds() {
        let body = serde_json::json!({
            "messages": [{"role": "user", "content": "hello world"}],
            "max_tokens": 100
        });

        // The floor raises a small reservation to the configured minimum...
        let with_floor = service();
        with_floor.set_policy(BillingPolicy {
            min_pre_consume: 5_000_000,
            ..service_policy()
        });
        assert_eq!(
            with_floor.reservation("Ling-1T", "default", &body, "/v1/chat/completions"),
            5_000_000
        );
        // ...and is off by default, so nothing changes for an instance that never
        // touched the option.
        let plain = service();
        plain.set_policy(service_policy());
        assert!(
            plain.reservation("Ling-1T", "default", &body, "/v1/chat/completions") < 5_000_000,
            "the floor must not apply unless configured"
        );

        // The floor must not resurrect a free model: an unpriced model reserves
        // nothing while the free-model switch is off, floor or no floor.
        with_floor.set_policy(BillingPolicy {
            min_pre_consume: 5_000_000,
            pre_consume_free_models: false,
            ..service_policy()
        });
        assert_eq!(
            with_floor.reservation("definitely-not-a-real-model", "default", &body, "/v1/chat/completions"),
            0
        );
        // And the switch is what makes it reserve.
        with_floor.set_policy(BillingPolicy {
            min_pre_consume: 5_000_000,
            pre_consume_free_models: true,
            ..service_policy()
        });
        assert_eq!(
            with_floor.reservation("definitely-not-a-real-model", "default", &body, "/v1/chat/completions"),
            5_000_000
        );

        // A policy change is visible immediately, which is what makes the option
        // live rather than a restart-time setting: the same call returns a
        // different amount once the floor moves.
        with_floor.set_policy(BillingPolicy {
            min_pre_consume: 42,
            pre_consume_free_models: true,
            ..service_policy()
        });
        assert_eq!(
            with_floor.reservation("definitely-not-a-real-model", "default", &body, "/v1/chat/completions"),
            42
        );
        with_floor.set_policy(BillingPolicy {
            min_pre_consume: 7,
            pre_consume_free_models: true,
            ..service_policy()
        });
        assert_eq!(
            with_floor.reservation("definitely-not-a-real-model", "default", &body, "/v1/chat/completions"),
            7
        );
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
        let wallet = FundingSource::Wallet { user_id: "user".into() };
        let body = serde_json::json!({"messages":[{"role":"user","content":"hello"}],"max_tokens":100});

        let reservation = service.reservation("Ling-1T", "default", &body, "/v1/chat/completions");
        let (session, reserved) = service
            .begin(&store, "key", &wallet, false, reservation)
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
            .settle(&store, &session, "key", "user", &charge, "test", None, &wallet)
            .expect("settle");

        assert_eq!(store.wallet(), 1_000_000 - charge.quota);
    }

    #[test]
    fn failed_request_is_refunded_in_full() {
        let service = service();
        let store = MemStore::new(1_000_000, 1_000_000);
        let wallet = FundingSource::Wallet { user_id: "user".into() };
        let body = serde_json::json!({"messages":[{"role":"user","content":"hello"}],"max_tokens":100});
        let reservation = service.reservation("Ling-1T", "default", &body, "/v1/chat/completions");
        let (session, _) = service.begin(&store, "key", &wallet, false, reservation).unwrap();

        service.refund(&store, &session, "key", "user", &wallet).unwrap();
        assert_eq!(store.wallet(), 1_000_000);
    }

    #[test]
    fn insufficient_wallet_rejects_the_request() {
        let service = service();
        let store = MemStore::new(1_000_000, 10);
        let wallet = FundingSource::Wallet { user_id: "user".into() };
        let body = serde_json::json!({"messages":[{"role":"user","content":"hello"}],"max_tokens":1000});
        let reservation = service.reservation("Ling-1T", "default", &body, "/v1/chat/completions");
        assert!(reservation > 10);
        assert!(service.begin(&store, "key", &wallet, false, reservation).is_err());
        // The key quota must not have been consumed by the failed attempt.
        assert_eq!(store.key_used(), 1_000_000);
        assert_eq!(store.wallet(), 10);
    }

    #[test]
    fn settlement_overrunning_the_reservation_overdraws_the_wallet() {
        let service = service();
        let store = MemStore::new(1_000_000, 1_000);
        let wallet = FundingSource::Wallet { user_id: "user".into() };
        let reservation = 100;
        let (session, _) = service.begin(&store, "key", &wallet, false, reservation).unwrap();

        let usage = BillingUsage {
            prompt_tokens: 1_000_000,
            completion_tokens: 100_000,
            ..Default::default()
        };
        let charge = service.charge("Ling-1T", "default", &usage, &EvalContext::default());
        assert!(charge.quota > reservation);
        service
            .settle(&store, &session, "key", "user", &charge, "big", None, &wallet)
            .unwrap();
        assert_eq!(store.wallet(), 1_000 - charge.quota);
        assert!(store.wallet() < 0, "wallet should absorb the debt");
    }

    #[test]
    fn a_subscription_pool_pays_before_the_wallet() {
        // The pool is spent first, so a user's subscription is used up before
        // their top-up balance.
        let service = service();
        let store = MemStore::new(1_000_000, 1_000_000);
        store.with_subscription("sub-1", 10_000);
        let funding = FundingSource::Subscription {
            user_id: "user".into(),
            subscription_id: "sub-1".into(),
        };

        let (session, reserved) = service.begin(&store, "key", &funding, false, 1_000).unwrap();
        assert_eq!(reserved, 1_000);
        assert_eq!(store.pool_used("sub-1"), 1_000, "the pool should be drawn on");
        assert_eq!(store.wallet(), 1_000_000, "the wallet must be untouched");

        let charge = Charge {
            quota: 1_000,
            ..service.charge("Ling-1T", "default", &BillingUsage::default(), &EvalContext::default())
        };
        service
            .settle(&store, &session, "key", "user", &charge, "test", None, &funding)
            .unwrap();
        assert_eq!(store.pool_used("sub-1"), 1_000);
        assert_eq!(store.wallet(), 1_000_000);
    }

    #[test]
    fn a_pool_too_small_for_the_reservation_overflows_to_the_wallet() {
        // Partial by design: what the pool cannot cover the wallet does, rather
        // than the request being refused or the pool being skipped entirely.
        let service = service();
        let store = MemStore::new(1_000_000, 1_000_000);
        store.with_subscription("sub-1", 100);
        let funding = FundingSource::Subscription {
            user_id: "user".into(),
            subscription_id: "sub-1".into(),
        };

        let (_, reserved) = service.begin(&store, "key", &funding, false, 500).unwrap();
        assert_eq!(reserved, 500);
        assert_eq!(store.pool_used("sub-1"), 100, "the pool gives all it has");
        assert_eq!(
            store.wallet(),
            1_000_000 - 400,
            "the wallet covers the remainder"
        );
    }

    #[test]
    fn refusing_a_reservation_leaves_the_pool_untouched() {
        // The rollback path: the pool is charged first, so a wallet that cannot
        // cover the remainder must give the pool's part back. Otherwise a refused
        // request would still cost the user quota.
        let service = service();
        let store = MemStore::new(1_000_000, 50);
        store.with_subscription("sub-1", 100);
        let funding = FundingSource::Subscription {
            user_id: "user".into(),
            subscription_id: "sub-1".into(),
        };

        let result = service.begin(&store, "key", &funding, false, 500);
        assert!(result.is_err(), "500 cannot be funded by 100 + 50");
        assert_eq!(store.pool_used("sub-1"), 0, "the pool must be rolled back");
        assert_eq!(store.wallet(), 50, "the wallet must be untouched");
        assert_eq!(store.key_used(), 1_000_000, "the key must be untouched");
    }

    #[test]
    fn a_failed_subscription_request_restores_pool_and_wallet() {
        let service = service();
        let store = MemStore::new(1_000_000, 1_000_000);
        store.with_subscription("sub-1", 100);
        let funding = FundingSource::Subscription {
            user_id: "user".into(),
            subscription_id: "sub-1".into(),
        };

        let (session, _) = service.begin(&store, "key", &funding, false, 500).unwrap();
        assert_eq!(store.pool_used("sub-1"), 100);
        assert_eq!(store.wallet(), 1_000_000 - 400);

        service.refund(&store, &session, "key", "user", &funding).unwrap();

        assert_eq!(store.pool_used("sub-1"), 0, "the pool must be made whole");
        assert_eq!(store.wallet(), 1_000_000, "and so must the wallet");
    }

    #[test]
    fn settlement_beyond_the_pool_debts_the_wallet() {
        // A stream that overruns its reservation must land somewhere real. The
        // pool is finite, so the excess becomes wallet debt -- the same rule the
        // wallet already follows for a reservation overrun.
        let service = service();
        // A small wallet, so the overrun genuinely has to go negative rather than
        // merely shrinking.
        let store = MemStore::new(1_000_000, 100);
        store.with_subscription("sub-1", 200);
        let funding = FundingSource::Subscription {
            user_id: "user".into(),
            subscription_id: "sub-1".into(),
        };

        let (session, _) = service.begin(&store, "key", &funding, false, 200).unwrap();
        let charge = Charge {
            quota: 900,
            ..service.charge("Ling-1T", "default", &BillingUsage::default(), &EvalContext::default())
        };
        service
            .settle(&store, &session, "key", "user", &charge, "big", None, &funding)
            .unwrap();

        // The pool is already drained by the reservation, so the entire excess is
        // the wallet's.
        assert_eq!(store.pool_used("sub-1"), 200, "the pool gave its whole ceiling");
        assert_eq!(store.wallet(), 100 - 700, "the wallet pays the excess");
        assert!(
            store.wallet() < 0,
            "the excess is wallet debt, got {}",
            store.wallet()
        );
    }

    #[test]
    fn an_unlimited_pool_never_touches_the_wallet() {
        // `total = 0` means unlimited, matching the reference.
        let service = service();
        let store = MemStore::new(1_000_000, 500);
        store.with_subscription("sub-1", 0);
        let funding = FundingSource::Subscription {
            user_id: "user".into(),
            subscription_id: "sub-1".into(),
        };

        let (_, reserved) = service.begin(&store, "key", &funding, false, 100_000).unwrap();
        assert_eq!(reserved, 100_000);
        assert_eq!(store.pool_used("sub-1"), 0, "unlimited records no usage");
        assert_eq!(store.wallet(), 500, "the wallet must be untouched");
    }

    #[test]
    fn a_wallet_source_behaves_exactly_as_before() {
        // Regression guard for the funding split: the pre-existing path must be
        // unchanged, or every non-subscriber's billing silently shifts.
        let service = service();
        let store = MemStore::new(1_000_000, 1_000_000);
        let wallet = FundingSource::Wallet { user_id: "user".into() };

        let (session, reserved) = service.begin(&store, "key", &wallet, false, 1_000).unwrap();
        assert_eq!(reserved, 1_000);
        assert_eq!(store.wallet(), 999_000, "the wallet pays directly");

        service.refund(&store, &session, "key", "user", &wallet).unwrap();
        assert_eq!(store.wallet(), 1_000_000);
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
