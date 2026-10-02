//! The reserve -> settle -> refund lifecycle for one request.
//!
//! Ported from NewAPI's `BillingSession` (`service/billing_session.go`).
//!
//! The state machine exists to make three things true:
//!
//! * **No over-spend under concurrency.** The caller must reserve atomically
//!   (a conditional `UPDATE ... WHERE quota >= ?`) before the upstream call, so
//!   two in-flight requests cannot both pass a stale balance check.
//! * **Failed requests cost nothing.** `refund` restores exactly what was
//!   reserved, and is idempotent.
//! * **A settled session never refunds.** Once funding is committed, a later
//!   `refund` must not hand money back — that is how a request would become free
//!   by failing *after* success.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};

use crate::quota_math::QuotaClamp;

#[derive(Debug, thiserror::Error)]
pub enum BillingError {
    /// The account cannot cover the charge.
    ///
    /// `available` is `-1` when the refusing layer could not read the balance;
    /// callers should substitute the real figure rather than printing it.
    #[error("insufficient balance: need {needed}, available {}", if *.available < 0 { "unknown".to_string() } else { available.to_string() })]
    InsufficientBalance { needed: i64, available: i64 },
    #[error("quota must not be negative: {0}")]
    NegativeQuota(i64),
    #[error("storage error: {0}")]
    Storage(String),
}

/// Observable lifecycle state, for tests and diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BillingState {
    /// Nothing reserved yet.
    Fresh,
    /// A reservation is held; a settle or refund is still expected.
    Reserved,
    /// Committed at the actual charge.
    Settled,
    /// Reserved quota returned.
    Refunded,
}

/// Applies quota movements to the caller's storage.
///
/// Implementations must make `try_reserve` atomic with respect to the balance
/// check (a single conditional UPDATE), otherwise the concurrency guarantee is
/// lost.
pub trait QuotaStore: Send + Sync {
    /// Attempt to move `amount` out of `account`. Returns `Ok(false)` when the
    /// account cannot cover it; `Err` only on a storage failure.
    fn try_reserve(&self, account: &str, amount: i64) -> Result<bool, BillingError>;
    /// Move `amount` out, allowed to overdraw (wallet debt semantics).
    fn debit(&self, account: &str, amount: i64) -> Result<(), BillingError>;
    /// Move `amount` in.
    fn credit(&self, account: &str, amount: i64) -> Result<(), BillingError>;
}

/// Aggregate of an account's spendable quota and its lifetime usage.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct QuotaSnapshot {
    pub balance: i64,
    pub used: i64,
}

/// One account a request may be charged against, with the bookkeeping the
/// session needs to adjust it later.
///
/// The reference routes this through a `FundingSource` trait
/// (`service/funding_source.go`) that draws from a subscription or the wallet by
/// billing preference. A subscription is not a wallet with a different balance:
/// its pool is finite and pool-scoped, so a request that outruns it leaves a debt
/// the wallet must absorb rather than a negative subscription balance. Carrying
/// the two identities explicitly is what lets [`crate::session::BillingSession`]
/// settle that overage somewhere real.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FundingSource {
    /// The user's wallet. May go negative on settlement, as money owed.
    Wallet { user_id: String },
    /// A subscription's own pool, with the wallet as its overflow.
    ///
    /// `subscription_id` is the pool to charge; `user_id` pays anything the pool
    /// cannot cover.
    Subscription {
        user_id: String,
        subscription_id: String,
    },
}

impl FundingSource {
    /// The account name a `QuotaStore` addresses for the primary charge.
    pub fn account(&self) -> &str {
        match self {
            FundingSource::Wallet { user_id } => user_id,
            // The subscription id names the pool; the store recognises it because
            // it is neither the key account nor a known user id.
            FundingSource::Subscription {
                subscription_id, ..
            } => subscription_id,
        }
    }

    /// The wallet that absorbs overflow, if this source has one.
    pub fn overflow_user(&self) -> Option<&str> {
        match self {
            FundingSource::Wallet { .. } => None,
            FundingSource::Subscription { user_id, .. } => Some(user_id),
        }
    }
}

/// One request's billing lifecycle.
pub struct BillingSession {
    token_account: String,
    funding_account: String,
    is_playground: bool,
    trusted: bool,
    pre_consumed: AtomicI64,
    token_consumed: AtomicI64,
    settled: AtomicBool,
    refunded: AtomicBool,
    funding_settled: AtomicBool,
    clamp: parking_lot::Mutex<Option<QuotaClamp>>,
}

impl BillingSession {
    pub fn new(token_account: String, funding_account: String, is_playground: bool) -> Self {
        Self {
            token_account,
            funding_account,
            is_playground,
            trusted: false,
            pre_consumed: AtomicI64::new(0),
            token_consumed: AtomicI64::new(0),
            settled: AtomicBool::new(false),
            refunded: AtomicBool::new(false),
            funding_settled: AtomicBool::new(false),
            clamp: parking_lot::Mutex::new(None),
        }
    }

    /// Mark the account trusted: skip reservation, allow settlement to overdraw.
    /// Mirrors NewAPI's trust bypass for well-funded accounts.
    pub fn trusted(mut self, trusted: bool) -> Self {
        self.trusted = trusted;
        self
    }

    /// Rebuild a session around a reservation made by an earlier one.
    ///
    /// A task outlives the session that reserved for it, so the settlement that
    /// happens later has to reconstruct the reservation before it can move a
    /// delta. Without this, a fresh session's `refund` hands back zero and a
    /// failed task silently keeps the caller's money (`refund` reads the
    /// session's own `pre_consumed`, not the database).
    ///
    /// The stored amount is the whole truth the session needs: `refund` returns
    /// exactly this much, and `settle(actual)` moves only `actual - reserved`.
    pub fn restored(
        token_account: String,
        funding_account: String,
        is_playground: bool,
        reserved: i64,
    ) -> Self {
        let session = Self::new(token_account, funding_account, is_playground);
        let reserved = reserved.max(0);
        session.pre_consumed.store(reserved, Ordering::SeqCst);
        if !is_playground {
            session.token_consumed.store(reserved, Ordering::SeqCst);
        }
        session
    }

    pub fn is_trusted(&self) -> bool {
        self.trusted
    }

    pub fn pre_consumed(&self) -> i64 {
        self.pre_consumed.load(Ordering::SeqCst)
    }

    pub fn state(&self) -> BillingState {
        if self.settled.load(Ordering::SeqCst) {
            BillingState::Settled
        } else if self.refunded.load(Ordering::SeqCst) {
            BillingState::Refunded
        } else if self.pre_consumed.load(Ordering::SeqCst) > 0 {
            BillingState::Reserved
        } else {
            BillingState::Fresh
        }
    }

    /// Record the first saturation event, for log auditing.
    pub fn note_clamp(&self, clamp: Option<QuotaClamp>) {
        if clamp.is_none() {
            return;
        }
        let mut slot = self.clamp.lock();
        if slot.is_none() {
            *slot = clamp;
        }
    }

    pub fn clamp(&self) -> Option<QuotaClamp> {
        self.clamp.lock().clone()
    }

    /// Reserve `quota` before the upstream call. Returns the amount reserved.
    ///
    /// Order matters: the token account is charged first and rolled back if the
    /// funding account cannot cover the charge, so a partial failure never leaves
    /// the token account short without the wallet being debited too.
    pub fn pre_consume(&self, store: &dyn QuotaStore, quota: i64) -> Result<i64, BillingError> {
        if quota < 0 {
            return Err(BillingError::NegativeQuota(quota));
        }
        if self.settled.load(Ordering::SeqCst) || self.refunded.load(Ordering::SeqCst) {
            return Ok(self.pre_consumed.load(Ordering::SeqCst));
        }

        let effective = if self.trusted { 0 } else { quota };
        if effective == 0 {
            return Ok(0);
        }

        // 1) token account (hard floor)
        if !self.is_playground {
            if !store.try_reserve(&self.token_account, effective)? {
                // The balance is not visible at this layer (`try_reserve` is a
                // boolean), so `available` is left unset (-1) and the caller
                // fills in the real figure from the store.
                return Err(BillingError::InsufficientBalance {
                    needed: effective,
                    available: -1,
                });
            }
            self.token_consumed.store(effective, Ordering::SeqCst);
        }

        // 2) funding account; roll the token charge back on failure
        let funded = store.try_reserve(&self.funding_account, effective)?;
        if !funded {
            let rollback = self.token_consumed.swap(0, Ordering::SeqCst);
            if rollback > 0 && !self.is_playground {
                store.credit(&self.token_account, rollback)?;
            }
            return Err(BillingError::InsufficientBalance {
                needed: effective,
                available: -1,
            });
        }

        self.pre_consumed.store(effective, Ordering::SeqCst);
        Ok(effective)
    }

    /// Settle at the actual charge. Idempotent.
    pub fn settle(&self, store: &dyn QuotaStore, actual_quota: i64) -> Result<(), BillingError> {
        if self.settled.swap(true, Ordering::SeqCst) {
            return Ok(());
        }

        let delta = actual_quota - self.pre_consumed.load(Ordering::SeqCst);
        if delta == 0 {
            return Ok(());
        }

        // Funding first: the wallet may go negative (debt) on a positive delta.
        if !self.funding_settled.swap(true, Ordering::SeqCst) {
            if delta > 0 {
                store.debit(&self.funding_account, delta)?;
            } else {
                store.credit(&self.funding_account, -delta)?;
            }
        }

        // Then the token account, which has a hard floor and may reject the
        // delta. Funding is already committed, so this is reported rather than
        // unwound: undoing the wallet debit would let a failing token adjustment
        // make the request free.
        if !self.is_playground {
            let result = if delta > 0 {
                store.debit(&self.token_account, delta)
            } else {
                store.credit(&self.token_account, -delta)
            };
            if let Err(error) = result {
                return Err(BillingError::Storage(format!(
                    "token account adjust after funding committed: {}",
                    error
                )));
            }
        }

        Ok(())
    }

    /// Return everything reserved. Idempotent, and a no-op once settled.
    pub fn refund(&self, store: &dyn QuotaStore) -> Result<(), BillingError> {
        if self.settled.load(Ordering::SeqCst) || self.funding_settled.load(Ordering::SeqCst) {
            return Ok(());
        }
        if self.refunded.swap(true, Ordering::SeqCst) {
            return Ok(());
        }

        let reserved = self.pre_consumed.swap(0, Ordering::SeqCst);
        if reserved == 0 {
            return Ok(());
        }

        store.credit(&self.funding_account, reserved)?;
        let token_amount = self.token_consumed.swap(0, Ordering::SeqCst);
        if token_amount > 0 && !self.is_playground {
            store.credit(&self.token_account, token_amount)?;
        }
        Ok(())
    }

    /// True when a refund would still move money.
    pub fn needs_refund(&self) -> bool {
        if self.settled.load(Ordering::SeqCst)
            || self.refunded.load(Ordering::SeqCst)
            || self.funding_settled.load(Ordering::SeqCst)
        {
            return false;
        }
        self.token_consumed.load(Ordering::SeqCst) > 0
            || self.pre_consumed.load(Ordering::SeqCst) > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use parking_lot::Mutex;
    use std::collections::HashMap;

    /// An in-memory store whose reserve is atomic under a lock, mirroring the
    /// conditional-UPDATE semantics the real store must provide.
    #[derive(Default)]
    struct MemStore {
        balances: Mutex<HashMap<String, i64>>,
    }

    impl MemStore {
        fn new(pairs: &[(&str, i64)]) -> Self {
            let store = Self::default();
            for (k, v) in pairs {
                store.balances.lock().insert((*k).to_string(), *v);
            }
            store
        }
        fn balance(&self, account: &str) -> i64 {
            *self.balances.lock().get(account).unwrap_or(&0)
        }
    }

    impl QuotaStore for MemStore {
        fn try_reserve(&self, account: &str, amount: i64) -> Result<bool, BillingError> {
            let mut balances = self.balances.lock();
            let entry = balances.entry(account.to_string()).or_insert(0);
            if *entry < amount {
                return Ok(false);
            }
            *entry -= amount;
            Ok(true)
        }
        fn debit(&self, account: &str, amount: i64) -> Result<(), BillingError> {
            let mut balances = self.balances.lock();
            *balances.entry(account.to_string()).or_insert(0) -= amount;
            Ok(())
        }
        fn credit(&self, account: &str, amount: i64) -> Result<(), BillingError> {
            let mut balances = self.balances.lock();
            *balances.entry(account.to_string()).or_insert(0) += amount;
            Ok(())
        }
    }

    fn session() -> BillingSession {
        BillingSession::new("token".into(), "wallet".into(), false)
    }

    #[test]
    fn reserve_then_settle_at_the_same_amount_moves_nothing_extra() {
        let store = MemStore::new(&[("token", 10_000), ("wallet", 10_000)]);
        let s = session();
        assert_eq!(s.pre_consume(&store, 500).unwrap(), 500);
        assert_eq!(store.balance("token"), 9_500);
        assert_eq!(store.balance("wallet"), 9_500);
        s.settle(&store, 500).unwrap();
        assert_eq!(store.balance("wallet"), 9_500);
        assert_eq!(s.state(), BillingState::Settled);
    }

    #[test]
    fn settle_debits_the_shortfall() {
        let store = MemStore::new(&[("token", 10_000), ("wallet", 10_000)]);
        let s = session();
        s.pre_consume(&store, 500).unwrap();
        s.settle(&store, 800).unwrap();
        assert_eq!(store.balance("token"), 9_200);
        assert_eq!(store.balance("wallet"), 9_200);
    }

    #[test]
    fn settle_refunds_the_overestimate() {
        let store = MemStore::new(&[("token", 10_000), ("wallet", 10_000)]);
        let s = session();
        s.pre_consume(&store, 900).unwrap();
        s.settle(&store, 300).unwrap();
        assert_eq!(store.balance("token"), 9_700);
        assert_eq!(store.balance("wallet"), 9_700);
    }

    #[test]
    fn refund_restores_everything() {
        let store = MemStore::new(&[("token", 10_000), ("wallet", 10_000)]);
        let s = session();
        s.pre_consume(&store, 750).unwrap();
        s.refund(&store).unwrap();
        assert_eq!(store.balance("token"), 10_000);
        assert_eq!(store.balance("wallet"), 10_000);
        assert_eq!(s.state(), BillingState::Refunded);
    }

    #[test]
    fn refund_is_idempotent() {
        let store = MemStore::new(&[("token", 10_000), ("wallet", 10_000)]);
        let s = session();
        s.pre_consume(&store, 400).unwrap();
        s.refund(&store).unwrap();
        s.refund(&store).unwrap();
        s.refund(&store).unwrap();
        assert_eq!(store.balance("wallet"), 10_000);
        assert_eq!(store.balance("token"), 10_000);
    }

    /// A session rebuilt from a stored reservation can return it: this is the
    /// path a failed task takes, where the original session is long gone.
    #[test]
    fn a_restored_session_refunds_the_reservation_it_did_not_make() {
        let store = MemStore::new(&[("token", 9_400), ("wallet", 9_400)]);
        let restored = BillingSession::restored("token".into(), "wallet".into(), false, 600);
        restored.refund(&store).unwrap();
        assert_eq!(store.balance("token"), 10_000);
        assert_eq!(store.balance("wallet"), 10_000);
    }

    /// And settling moves only the difference, never the whole charge again:
    /// the reservation was already taken before this session existed.
    #[test]
    fn a_restored_session_settles_only_the_difference() {
        // Overestimate: 900 reserved, 300 actual, 600 returned.
        let store = MemStore::new(&[("token", 9_100), ("wallet", 9_100)]);
        let restored = BillingSession::restored("token".into(), "wallet".into(), false, 900);
        restored.settle(&store, 300).unwrap();
        assert_eq!(store.balance("token"), 9_700);
        assert_eq!(store.balance("wallet"), 9_700);

        // Underestimate: 400 reserved, 750 actual, 350 more taken.
        let store = MemStore::new(&[("token", 9_600), ("wallet", 9_600)]);
        let restored = BillingSession::restored("token".into(), "wallet".into(), false, 400);
        restored.settle(&store, 750).unwrap();
        assert_eq!(store.balance("token"), 9_250);
        assert_eq!(store.balance("wallet"), 9_250);
    }

    #[test]
    fn a_settled_session_never_refunds() {
        // The critical invariant: success followed by failure must not be free.
        let store = MemStore::new(&[("token", 10_000), ("wallet", 10_000)]);
        let s = session();
        s.pre_consume(&store, 600).unwrap();
        s.settle(&store, 600).unwrap();
        s.refund(&store).unwrap();
        assert_eq!(store.balance("wallet"), 9_400);
        assert!(!s.needs_refund());
    }

    #[test]
    fn settle_is_idempotent() {
        let store = MemStore::new(&[("token", 10_000), ("wallet", 10_000)]);
        let s = session();
        s.pre_consume(&store, 500).unwrap();
        s.settle(&store, 700).unwrap();
        s.settle(&store, 700).unwrap();
        assert_eq!(store.balance("wallet"), 9_300);
    }

    #[test]
    fn reservation_exceeding_balance_is_rejected_without_side_effects() {
        let store = MemStore::new(&[("token", 100), ("wallet", 100)]);
        let s = session();
        assert!(s.pre_consume(&store, 500).is_err());
        assert_eq!(store.balance("token"), 100);
        assert_eq!(store.balance("wallet"), 100);
        assert_eq!(s.state(), BillingState::Fresh);
    }

    #[test]
    fn funding_failure_rolls_the_token_charge_back() {
        // Token account has room, wallet does not.
        let store = MemStore::new(&[("token", 10_000), ("wallet", 10)]);
        let s = session();
        assert!(s.pre_consume(&store, 500).is_err());
        assert_eq!(store.balance("token"), 10_000, "token charge must roll back");
        assert_eq!(store.balance("wallet"), 10);
        assert_eq!(s.state(), BillingState::Fresh);
    }

    #[test]
    fn trusted_session_skips_reservation_but_still_settles() {
        let store = MemStore::new(&[("token", 10_000), ("wallet", 10_000)]);
        let s = session().trusted(true);
        assert_eq!(s.pre_consume(&store, 5_000).unwrap(), 0);
        assert_eq!(store.balance("wallet"), 10_000);
        s.settle(&store, 5_000).unwrap();
        assert_eq!(store.balance("wallet"), 5_000);
    }

    #[test]
    fn playground_skips_the_token_account() {
        let store = MemStore::new(&[("token", 0), ("wallet", 10_000)]);
        let s = BillingSession::new("token".into(), "wallet".into(), true);
        s.pre_consume(&store, 300).unwrap();
        assert_eq!(store.balance("token"), 0);
        assert_eq!(store.balance("wallet"), 9_700);
        s.settle(&store, 300).unwrap();
        assert_eq!(store.balance("token"), 0);
    }

    #[test]
    fn negative_quota_is_rejected() {
        let store = MemStore::new(&[("token", 100), ("wallet", 100)]);
        let s = session();
        assert!(matches!(
            s.pre_consume(&store, -1),
            Err(BillingError::NegativeQuota(-1))
        ));
    }

    #[test]
    fn overdraw_is_permitted_at_settlement_only() {
        // A request can cost more than the balance; the wallet absorbs the debt
        // rather than the request becoming silently free.
        let store = MemStore::new(&[("token", 10_000), ("wallet", 1_000)]);
        let s = session();
        s.pre_consume(&store, 1_000).unwrap();
        s.settle(&store, 3_000).unwrap();
        assert_eq!(store.balance("wallet"), -2_000);
    }

    #[test]
    fn clamp_is_recorded_once() {
        let s = session();
        let first = QuotaClamp {
            op: "QuotaRound",
            kind: crate::quota_math::QuotaClampKind::Overflow,
            original: 1e30,
            clamped: crate::quota_math::MAX_QUOTA,
        };
        let second = QuotaClamp {
            op: "QuotaFromDecimal",
            ..first.clone()
        };
        s.note_clamp(Some(first.clone()));
        s.note_clamp(Some(second));
        assert_eq!(s.clamp(), Some(first));
    }

    #[test]
    fn concurrent_reservations_cannot_overspend() {
        use std::sync::Arc;
        use std::thread;

        // 10 accounts' worth of balance, 20 threads each asking for 10 units'
        // worth: exactly half must succeed.
        let store = Arc::new(MemStore::new(&[("token", 100), ("wallet", 100)]));
        let mut handles = Vec::new();
        for _ in 0..20 {
            let store = Arc::clone(&store);
            handles.push(thread::spawn(move || {
                let s = BillingSession::new("token".into(), "wallet".into(), false);
                s.pre_consume(&*store, 10).is_ok()
            }));
        }
        // `join` consumes the handle, so map first and count afterwards.
        let successes = handles
            .into_iter()
            .map(|h| h.join().unwrap_or(false))
            .filter(|ok| *ok)
            .count();

        assert_eq!(successes, 10, "overspend detected");
        assert_eq!(store.balance("wallet"), 0);
        assert_eq!(store.balance("token"), 0);
    }
}
