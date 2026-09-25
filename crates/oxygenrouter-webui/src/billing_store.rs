//! SQLite-backed `BillingStore`.
//!
//! Adapts `oxygenrouter-core`'s atomic quota primitives to the billing engine's
//! storage trait. Every method here maps to a single statement in `db.rs`, so the
//! reservation guarantee survives the indirection: the guard and the mutation
//! are still one atomic operation.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use std::sync::Arc;

use oxygenrouter_billing::{BillingError, BillingStore};
use oxygenrouter_core::Database;

/// Wraps the database so the billing engine never sees `rusqlite`.
pub struct SqliteBillingStore {
    db: Arc<Database>,
}

impl SqliteBillingStore {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }
}

/// A failed statement is a storage fault, not a refusal. Keeping these distinct
/// matters: a refusal is a normal "insufficient balance" outcome the caller may
/// report to the client, while a storage fault must not be mistaken for one.
fn storage(error: impl std::fmt::Display) -> BillingError {
    BillingError::Storage(error.to_string())
}

impl BillingStore for SqliteBillingStore {
    fn try_reserve_key(&self, key_id: &str, amount: i64) -> Result<bool, BillingError> {
        self.db
            .try_reserve_key_quota(key_id, amount)
            .map_err(storage)
    }

    fn try_reserve_wallet(&self, user_id: &str, amount: i64) -> Result<bool, BillingError> {
        self.db.try_reserve_wallet(user_id, amount).map_err(storage)
    }

    fn debit_key(&self, key_id: &str, amount: i64) -> Result<(), BillingError> {
        self.db.debit_key_quota(key_id, amount).map_err(storage)
    }

    fn credit_key(&self, key_id: &str, amount: i64) -> Result<(), BillingError> {
        self.db.credit_key_quota(key_id, amount).map_err(storage)
    }

    fn debit_wallet(&self, user_id: &str, amount: i64) -> Result<(), BillingError> {
        self.db.debit_wallet(user_id, amount).map_err(storage)
    }

    fn credit_wallet(&self, user_id: &str, amount: i64) -> Result<(), BillingError> {
        self.db.credit_wallet(user_id, amount).map_err(storage)
    }

    fn record_charge(
        &self,
        user_id: &str,
        amount: i64,
        description: &str,
        reference_id: Option<&str>,
    ) -> Result<(), BillingError> {
        // Audit only: `append_ledger_entry` does not move the balance.
        self.db
            .append_ledger_entry(user_id, amount, "consume", description, reference_id)
            .map(|_| ())
            .map_err(storage)
    }

    fn wallet_balance(&self, user_id: &str) -> Result<i64, BillingError> {
        self.db.wallet_balance(user_id).map_err(storage)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxygenrouter_billing::BillingStore;
    use oxygenrouter_core::{ApiKey, UserRole};

    fn db() -> (Arc<Database>, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("billing-store-test.db");
        let db = Database::new(&path).expect("database");
        (Arc::new(db), dir)
    }

    /// Create a funded user through the public API, then top the balance up.
    fn user(db: &Database, name: &str, balance: i64) -> String {
        let u = db
            .create_user(name, &format!("{name}@example.test"), "password123", UserRole::User)
            .expect("create user");
        db.admin_adjust_balance(&u.id, balance, "test funding")
            .expect("fund");
        u.id
    }

    /// Create a key bound to a user with a quota ceiling.
    fn key(db: &Database, user_id: &str, quota: i64) -> String {
        let k = ApiKey::new(format!("sk-{user_id}"), "test-key".to_string())
            .owned_by(user_id)
            .with_quota(quota);
        db.upsert_api_key(&k).expect("upsert key");
        k.id
    }

    fn store(db: &Arc<Database>) -> SqliteBillingStore {
        SqliteBillingStore::new(Arc::clone(db))
    }

    #[test]
    fn reserve_moves_both_accounts() {
        let (db, _dir) = db();
        let uid = user(&db, "alice", 1000);
        let kid = key(&db, &uid, 1000);
        let s = store(&db);

        assert!(s.try_reserve_key(&kid, 100).unwrap());
        assert!(s.try_reserve_wallet(&uid, 100).unwrap());
        assert_eq!(db.key_used_micros(&kid).unwrap(), 100);
        assert_eq!(db.wallet_balance(&uid).unwrap(), 900);
    }

    #[test]
    fn reserve_refuses_when_the_wallet_cannot_cover_it() {
        let (db, _dir) = db();
        let uid = user(&db, "bob", 50);
        let s = store(&db);
        assert!(!s.try_reserve_wallet(&uid, 100).unwrap());
        assert_eq!(db.wallet_balance(&uid).unwrap(), 50);
    }

    #[test]
    fn reserve_refuses_when_the_key_ceiling_is_reached() {
        let (db, _dir) = db();
        let uid = user(&db, "carol", 1000);
        let kid = key(&db, &uid, 50);
        let s = store(&db);
        assert!(!s.try_reserve_key(&kid, 100).unwrap());
        assert_eq!(db.key_used_micros(&kid).unwrap(), 0);
    }

    #[test]
    fn zero_key_quota_means_unlimited() {
        let (db, _dir) = db();
        let uid = user(&db, "dave", 1000);
        let kid = key(&db, &uid, 0);
        let s = store(&db);
        assert!(s.try_reserve_key(&kid, 1_000_000).unwrap());
    }

    #[test]
    fn refund_restores_both_accounts() {
        let (db, _dir) = db();
        let uid = user(&db, "erin", 1000);
        let kid = key(&db, &uid, 1000);
        let s = store(&db);

        s.try_reserve_key(&kid, 300).unwrap();
        s.try_reserve_wallet(&uid, 300).unwrap();
        s.credit_key(&kid, 300).unwrap();
        s.credit_wallet(&uid, 300).unwrap();

        assert_eq!(db.key_used_micros(&kid).unwrap(), 0);
        assert_eq!(db.wallet_balance(&uid).unwrap(), 1000);
    }

    #[test]
    fn wallet_may_overdraw_on_settlement() {
        let (db, _dir) = db();
        let uid = user(&db, "frank", 100);
        let s = store(&db);
        s.debit_wallet(&uid, 500).unwrap();
        assert_eq!(db.wallet_balance(&uid).unwrap(), -400);
    }

    #[test]
    fn record_charge_is_audit_only() {
        // The billing session owns balance movement; a ledger write must not
        // move money or every request would be charged twice. Regression guard.
        let (db, _dir) = db();
        let uid = user(&db, "grace", 1000);
        let s = store(&db);

        s.record_charge(&uid, -250, "test charge", None).unwrap();
        assert_eq!(
            db.wallet_balance(&uid).unwrap(),
            1000,
            "record_charge must not change the balance"
        );
        let ledger = db.list_ledger(&uid).unwrap();
        // One entry from the funding adjustment, one from the charge.
        assert_eq!(ledger.len(), 2, "the ledger entry itself must be written");
        assert!(ledger.iter().any(|e| e.amount_micros == -250));
    }

    #[test]
    fn concurrent_reservations_cannot_overspend_a_wallet() {
        // The guarantee the single-statement UPDATE exists to provide.
        use std::thread;
        let (db, _dir) = db();
        let uid = user(&db, "heidi", 100);
        let mut handles = Vec::new();
        for _ in 0..20 {
            let db = Arc::clone(&db);
            let uid = uid.clone();
            handles.push(thread::spawn(move || {
                let s = SqliteBillingStore::new(db);
                s.try_reserve_wallet(&uid, 10).unwrap_or(false)
            }));
        }
        let granted = handles
            .into_iter()
            .map(|h| h.join().unwrap_or(false))
            .filter(|ok| *ok)
            .count();
        assert_eq!(granted, 10, "exactly ten 10-unit reservations fit in 100");
        assert_eq!(db.wallet_balance(&uid).unwrap(), 0);
    }
}