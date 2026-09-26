//! Administrator lifecycle over user subscriptions.
//!
//! Two properties are load-bearing and easy to get wrong:
//!
//! * **A grant must not charge the user.** An admin binding a plan is gifting it,
//!   matching the reference's `AdminBindSubscription`. Reusing the purchase path
//!   would silently debit the customer for the operator's action.
//! * **Invalidate and delete are different.** Invalidating ends a live
//!   entitlement and keeps the record; deleting erases a mistaken grant. Conflating
//!   them loses the audit trail, or leaves an entitlement running.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use oxygenrouter_core::{Database, SubscriptionPlan, User, UserRole};

struct Fixture {
    db: Database,
    alice: User,
    bob: User,
}

fn setup() -> Fixture {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("subs.db");
    let db_path = path.clone();
    std::mem::forget(dir);
    let db = Database::new(&db_path).expect("database opens");
    let alice = db
        .create_user("alice", "alice@example.test", "password-1", UserRole::User)
        .expect("alice");
    let bob = db
        .create_user("bob", "bob@example.test", "password-2", UserRole::User)
        .expect("bob");
    Fixture { db, alice, bob }
}

fn plan(db: &Database, id: &str, price: i64, days: i64) -> SubscriptionPlan {
    let now = chrono::Utc::now();
    let plan = SubscriptionPlan {
        id: id.to_string(),
        name: format!("Plan {id}"),
        description: String::new(),
        price_micros: price,
        quota_micros: price * 10,
        duration_days: days,
        enabled: true,
        created_at: now,
        updated_at: now,
    };
    db.upsert_plan(&plan).unwrap();
    plan
}

/// Credit a wallet so a purchase would be affordable.
fn fund(db: &Database, user_id: &str, amount: i64) {
    db.credit_wallet(user_id, amount).unwrap();
}

#[test]
fn a_grant_activates_the_plan() {
    let f = setup();
    plan(&f.db, "p1", 0, 30);

    let granted = f.db.grant_subscription(&f.alice.id, "p1").unwrap();
    assert_eq!(granted.status, "active");
    assert_eq!(granted.plan_id, "p1");
    assert_eq!(f.db.list_subscriptions(&f.alice.id).unwrap().len(), 1);
}

#[test]
fn a_grant_does_not_charge_the_users_wallet() {
    // The reason this path exists separately from `subscribe`.
    let f = setup();
    plan(&f.db, "p1", 5_000_000, 30);
    let before = f.db.wallet_balance(&f.alice.id).unwrap();

    f.db.grant_subscription(&f.alice.id, "p1").unwrap();

    let after = f.db.wallet_balance(&f.alice.id).unwrap();
    assert_eq!(before, after, "an admin grant must not debit the user");
    // And no ledger entry was written for it either.
    let ledger = f.db.list_ledger(&f.alice.id).unwrap();
    assert!(
        !ledger.iter().any(|e| e.kind == "subscription_purchase"),
        "a grant must not record a purchase: {ledger:?}"
    );
}

#[test]
fn a_purchase_does_charge_by_contrast() {
    // Pins the distinction the previous test relies on: the purchase path is the
    // one that debits, so "grant does not charge" is a real difference.
    let f = setup();
    plan(&f.db, "p1", 5_000_000, 30);
    fund(&f.db, &f.alice.id, 10_000_000);

    f.db.subscribe(&f.alice.id, "p1").unwrap();

    assert_eq!(f.db.wallet_balance(&f.alice.id).unwrap(), 5_000_000);
}

#[test]
fn a_grant_refuses_a_second_active_subscription_on_the_same_plan() {
    let f = setup();
    plan(&f.db, "p1", 0, 30);
    f.db.grant_subscription(&f.alice.id, "p1").unwrap();

    let again = f.db.grant_subscription(&f.alice.id, "p1");
    assert!(again.is_err(), "a duplicate grant must be refused");
    assert_eq!(f.db.list_subscriptions(&f.alice.id).unwrap().len(), 1);
}

#[test]
fn a_grant_on_an_unknown_plan_is_refused() {
    let f = setup();
    assert!(f.db.grant_subscription(&f.alice.id, "nope").is_err());
    assert!(f.db.list_subscriptions(&f.alice.id).unwrap().is_empty());
}

#[test]
fn a_grant_can_use_a_disabled_plan() {
    // An operator may deliberately bind a retired plan; the reference's bind does
    // not require the plan to be purchasable.
    let f = setup();
    let mut p = plan(&f.db, "retired", 0, 30);
    p.enabled = false;
    f.db.upsert_plan(&p).unwrap();

    assert!(f.db.grant_subscription(&f.alice.id, "retired").is_ok());
}

#[test]
fn invalidating_ends_the_entitlement_but_keeps_the_record() {
    let f = setup();
    plan(&f.db, "p1", 0, 30);
    let granted = f.db.grant_subscription(&f.alice.id, "p1").unwrap();

    assert!(f.db.invalidate_subscription(&granted.id).unwrap());

    let rows = f.db.list_subscriptions(&f.alice.id).unwrap();
    assert_eq!(rows.len(), 1, "the record must survive for audit");
    assert_eq!(rows[0].status, "cancelled");
}

#[test]
fn invalidating_an_already_inactive_subscription_reports_nothing_done() {
    // Lets an operator tell "I just ended it" from "it was already over", instead
    // of rewriting history on a row that changed no entitlement.
    let f = setup();
    plan(&f.db, "p1", 0, 30);
    let granted = f.db.grant_subscription(&f.alice.id, "p1").unwrap();
    f.db.invalidate_subscription(&granted.id).unwrap();

    assert!(!f.db.invalidate_subscription(&granted.id).unwrap());
    assert!(!f.db.invalidate_subscription("no-such-id").unwrap());
}

#[test]
fn deleting_erases_the_record() {
    let f = setup();
    plan(&f.db, "p1", 0, 30);
    let granted = f.db.grant_subscription(&f.alice.id, "p1").unwrap();

    assert!(f.db.delete_subscription(&granted.id).unwrap());
    assert!(f.db.list_subscriptions(&f.alice.id).unwrap().is_empty());
    assert!(!f.db.delete_subscription(&granted.id).unwrap());
}

#[test]
fn after_a_grant_is_invalidated_the_same_plan_can_be_granted_again() {
    // Ending an entitlement must release the slot, or a user could never be
    // re-subscribed to a plan they once held.
    let f = setup();
    plan(&f.db, "p1", 0, 30);
    let first = f.db.grant_subscription(&f.alice.id, "p1").unwrap();
    f.db.invalidate_subscription(&first.id).unwrap();

    let second = f.db.grant_subscription(&f.alice.id, "p1").unwrap();
    assert_eq!(second.status, "active");
}

#[test]
fn resetting_for_a_plan_ends_only_that_plans_subscriptions() {
    let f = setup();
    plan(&f.db, "p1", 0, 30);
    plan(&f.db, "p2", 0, 30);
    f.db.grant_subscription(&f.alice.id, "p1").unwrap();
    f.db.grant_subscription(&f.alice.id, "p2").unwrap();

    let ended = f.db.reset_subscriptions_for_plan(&f.alice.id, "p1").unwrap();
    assert_eq!(ended, 1);

    let rows = f.db.list_subscriptions(&f.alice.id).unwrap();
    let p1 = rows.iter().find(|s| s.plan_id == "p1").unwrap();
    let p2 = rows.iter().find(|s| s.plan_id == "p2").unwrap();
    assert_eq!(p1.status, "cancelled");
    assert_eq!(p2.status, "active", "the other plan must be untouched");
}

#[test]
fn listing_a_plan_shows_every_user_on_it() {
    // The question an admin view answers, which a per-user list cannot.
    let f = setup();
    plan(&f.db, "p1", 0, 30);
    plan(&f.db, "p2", 0, 30);
    f.db.grant_subscription(&f.alice.id, "p1").unwrap();
    f.db.grant_subscription(&f.bob.id, "p1").unwrap();
    f.db.grant_subscription(&f.bob.id, "p2").unwrap();

    let on_p1 = f.db.list_subscriptions_for_plan("p1").unwrap();
    assert_eq!(on_p1.len(), 2);
    let users: Vec<&str> = on_p1.iter().map(|s| s.user_id.as_str()).collect();
    assert!(users.contains(&f.alice.id.as_str()));
    assert!(users.contains(&f.bob.id.as_str()));

    assert_eq!(f.db.list_subscriptions_for_plan("p2").unwrap().len(), 1);
    assert!(f.db.list_subscriptions_for_plan("p3").unwrap().is_empty());
}

#[test]
fn resetting_a_whole_plan_ends_it_for_every_user() {
    let f = setup();
    plan(&f.db, "p1", 0, 30);
    f.db.grant_subscription(&f.alice.id, "p1").unwrap();
    f.db.grant_subscription(&f.bob.id, "p1").unwrap();

    // Mirrors the endpoint, which walks users rather than issuing one bulk
    // UPDATE, so there is a single definition of "end a subscription".
    let mut ended = 0;
    for subscription in f.db.list_subscriptions_for_plan("p1").unwrap() {
        ended += f
            .db
            .reset_subscriptions_for_plan(&subscription.user_id, "p1")
            .unwrap();
    }
    assert_eq!(ended, 2);

    let remaining: Vec<_> = f
        .db
        .list_subscriptions_for_plan("p1")
        .unwrap()
        .into_iter()
        .filter(|s| s.status == "active")
        .collect();
    assert!(remaining.is_empty(), "no active subscriptions should remain");
}

#[test]
fn an_admin_list_includes_inactive_rows() {
    // An operator investigating a problem needs the history, not only the live
    // entitlement.
    let f = setup();
    plan(&f.db, "p1", 0, 30);
    let granted = f.db.grant_subscription(&f.alice.id, "p1").unwrap();
    f.db.invalidate_subscription(&granted.id).unwrap();

    let all = f.db.list_all_subscriptions(&f.alice.id).unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].status, "cancelled");
}

#[test]
fn a_lapsed_subscription_is_marked_expired_not_left_active() {
    // A grant uses a negative duration so the row expires immediately. The next
    // grant must see a lapsed row, mark it expired, and still succeed — a stale
    // "active" row would otherwise block the user forever.
    let f = setup();
    plan(&f.db, "lapsed", 0, -1);
    f.db.grant_subscription(&f.alice.id, "lapsed").unwrap();

    // The same plan can be granted again because the first row is now expired.
    let fresh = f.db.grant_subscription(&f.alice.id, "lapsed").unwrap();
    assert_eq!(fresh.status, "active");

    let rows = f.db.list_subscriptions(&f.alice.id).unwrap();
    assert_eq!(
        rows.iter().filter(|s| s.status == "expired").count(),
        1,
        "the lapsed row should be marked expired: {rows:?}"
    );
    assert_eq!(rows.iter().filter(|s| s.status == "active").count(), 1);
}

#[test]
fn a_grant_ignores_expiry_only_for_already_ended_rows() {
    // Ordering guard: an active row still blocks a duplicate grant, so the
    // lapsed-row tolerance above is not a general "duplicates are fine" relaxation.
    let f = setup();
    plan(&f.db, "p1", 0, 30);
    f.db.grant_subscription(&f.alice.id, "p1").unwrap();
    assert!(f.db.grant_subscription(&f.alice.id, "p1").is_err());
}
