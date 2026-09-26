//! The subscription quota pool: snapshot, spending, selection, refund.
//!
//! The reference keeps this pool on the *subscription*, not in the wallet
//! (`model/subscription.go:258-259`, `AmountTotal`/`AmountUsed`). That distinction
//! matters: crediting the wallet instead would make subscription quota spendable
//! after the subscription expired, and would mix it with money the user can top
//! up. These tests pin the pool semantics.
//!
//! Selection follows `model/subscription.go:1334-1358`: among active unexpired
//! subscriptions ordered by soonest expiry, the first that can cover the amount.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use oxygenrouter_core::{Database, SubscriptionPlan, User, UserRole};

struct Fixture {
    db: Database,
    alice: User,
}

fn setup() -> Fixture {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("subquota.db");
    let db_path = path.clone();
    std::mem::forget(dir);
    let db = Database::new(&db_path).expect("database opens");
    let alice = db
        .create_user("alice", "alice@example.test", "password-1", UserRole::User)
        .expect("alice");
    Fixture { db, alice }
}

/// A free plan (so no wallet needs funding) with the given pool and duration.
fn plan(db: &Database, id: &str, quota: i64, days: i64) -> SubscriptionPlan {
    let now = chrono::Utc::now();
    let plan = SubscriptionPlan {
        id: id.to_string(),
        name: format!("Plan {id}"),
        description: String::new(),
        price_micros: 0,
        quota_micros: quota,
        duration_days: days,
        enabled: true,
        created_at: now,
        updated_at: now,
    };
    db.upsert_plan(&plan).unwrap();
    plan
}

#[test]
fn a_subscription_snapshots_the_plans_pool() {
    let f = setup();
    plan(&f.db, "p1", 1_000, 30);
    let sub = f.db.grant_subscription(&f.alice.id, "p1").unwrap();

    assert_eq!(sub.amount_total, 1_000);
    assert_eq!(sub.amount_used, 0);
    assert_eq!(sub.remaining(), Some(1_000));
}

#[test]
fn editing_the_plan_does_not_change_an_existing_subscription() {
    // The snapshot is the point: a later reprice must not retroactively alter what
    // the user already bought.
    let f = setup();
    let p = plan(&f.db, "p1", 1_000, 30);
    let sub = f.db.grant_subscription(&f.alice.id, "p1").unwrap();

    let mut edited = p;
    edited.quota_micros = 99;
    f.db.upsert_plan(&edited).unwrap();

    let listed = f.db.list_subscriptions(&f.alice.id).unwrap();
    assert_eq!(listed[0].id, sub.id);
    assert_eq!(listed[0].amount_total, 1_000, "the snapshot must be stable");
}

#[test]
fn a_zero_pool_means_unlimited() {
    let f = setup();
    plan(&f.db, "unlimited", 0, 30);
    let sub = f.db.grant_subscription(&f.alice.id, "unlimited").unwrap();
    assert_eq!(sub.remaining(), None);
}

#[test]
fn spending_reduces_the_pool() {
    let f = setup();
    plan(&f.db, "p1", 1_000, 30);
    let sub = f.db.grant_subscription(&f.alice.id, "p1").unwrap();

    // The return value is how much the pool actually took.
    assert_eq!(f.db.reserve_subscription_quota(&sub.id, 400).unwrap(), 400);
    let listed = f.db.list_subscriptions(&f.alice.id).unwrap();
    assert_eq!(listed[0].amount_used, 400);
    assert_eq!(listed[0].remaining(), Some(600));
}

#[test]
fn spending_cannot_exceed_the_pool() {
    // The guard is in the UPDATE's WHERE clause, so this holds under concurrency,
    // not merely when callers check first.
    let f = setup();
    plan(&f.db, "p1", 1_000, 30);
    let sub = f.db.grant_subscription(&f.alice.id, "p1").unwrap();

    assert_eq!(f.db.reserve_subscription_quota(&sub.id, 1_000).unwrap(), 1_000);
    assert_eq!(
        f.db.reserve_subscription_quota(&sub.id, 1).unwrap(),
        0,
        "an exhausted pool must take nothing"
    );
    assert_eq!(
        f.db.list_subscriptions(&f.alice.id).unwrap()[0].amount_used,
        1_000
    );
}

#[test]
fn an_unlimited_pool_records_no_usage() {
    // Matches the reference, which only accumulates `AmountUsed` when a total is
    // set: an unlimited subscription has no meaningful "used".
    let f = setup();
    plan(&f.db, "unlimited", 0, 30);
    let sub = f.db.grant_subscription(&f.alice.id, "unlimited").unwrap();

    assert_eq!(
        f.db.reserve_subscription_quota(&sub.id, 5_000).unwrap(),
        5_000,
        "an unlimited pool takes the full amount"
    );
    let listed = f.db.list_subscriptions(&f.alice.id).unwrap();
    assert_eq!(listed[0].amount_used, 0);
}

#[test]
fn a_billable_amount_selects_the_pool_that_can_cover_it() {
    let f = setup();
    plan(&f.db, "small", 100, 30);
    plan(&f.db, "large", 10_000, 30);
    f.db.grant_subscription(&f.alice.id, "small").unwrap();
    f.db.grant_subscription(&f.alice.id, "large").unwrap();

    // 500 does not fit the small pool, so the large one funds it.
    let chosen = f
        .db
        .subscription_funding_source(&f.alice.id, 500)
        .unwrap()
        .expect("a pool should cover it");
    assert_eq!(chosen.plan_id, "large");
}

#[test]
fn selection_prefers_the_pool_closest_to_expiring() {
    // Drains the soonest-lapsing entitlement first, so paid-for quota is not lost
    // when a subscription expires mid-way.
    let f = setup();
    plan(&f.db, "soon", 1_000, 1);
    plan(&f.db, "later", 1_000, 365);
    f.db.grant_subscription(&f.alice.id, "later").unwrap();
    f.db.grant_subscription(&f.alice.id, "soon").unwrap();

    let chosen = f
        .db
        .subscription_funding_source(&f.alice.id, 100)
        .unwrap()
        .expect("a pool");
    assert_eq!(chosen.plan_id, "soon");
}

#[test]
fn an_exhausted_pool_is_skipped_in_favour_of_one_that_can_pay() {
    let f = setup();
    plan(&f.db, "drained", 100, 1);
    plan(&f.db, "backup", 1_000, 365);
    let drained = f.db.grant_subscription(&f.alice.id, "drained").unwrap();
    f.db.grant_subscription(&f.alice.id, "backup").unwrap();
    assert_eq!(f.db.reserve_subscription_quota(&drained.id, 100).unwrap(), 100);

    let chosen = f
        .db
        .subscription_funding_source(&f.alice.id, 50)
        .unwrap()
        .expect("a pool");
    assert_eq!(chosen.plan_id, "backup");
}

#[test]
fn an_expired_subscription_cannot_fund_a_request_even_with_quota_left() {
    // The property that crediting the wallet would have broken: leftover
    // subscription quota must not be spendable after expiry.
    let f = setup();
    plan(&f.db, "lapsed", 1_000, -1);
    f.db.grant_subscription(&f.alice.id, "lapsed").unwrap();

    assert!(
        f.db.subscription_funding_source(&f.alice.id, 1).unwrap().is_none(),
        "an expired subscription must not fund anything"
    );
}

#[test]
fn a_cancelled_subscription_cannot_fund_a_request() {
    let f = setup();
    plan(&f.db, "p1", 1_000, 30);
    let sub = f.db.grant_subscription(&f.alice.id, "p1").unwrap();
    f.db.invalidate_subscription(&sub.id).unwrap();

    assert!(f.db
        .subscription_funding_source(&f.alice.id, 1)
        .unwrap()
        .is_none());
}

#[test]
fn nobody_s_plan_funds_nobody() {
    let f = setup();
    plan(&f.db, "p1", 1_000, 30);
    assert!(f.db
        .subscription_funding_source(&f.alice.id, 1)
        .unwrap()
        .is_none());
}

#[test]
fn a_refund_returns_quota_and_cannot_go_negative() {
    let f = setup();
    plan(&f.db, "p1", 1_000, 30);
    let sub = f.db.grant_subscription(&f.alice.id, "p1").unwrap();
    assert_eq!(f.db.reserve_subscription_quota(&sub.id, 300).unwrap(), 300);

    f.db.refund_subscription_quota(&sub.id, 300).unwrap();
    assert_eq!(
        f.db.list_subscriptions(&f.alice.id).unwrap()[0].amount_used,
        0
    );

    // A second refund must not manufacture quota.
    f.db.refund_subscription_quota(&sub.id, 300).unwrap();
    assert_eq!(
        f.db.list_subscriptions(&f.alice.id).unwrap()[0].amount_used,
        0,
        "a double refund must not push usage below zero"
    );
}

#[test]
fn a_refund_restores_a_pool_that_had_been_exhausted() {
    let f = setup();
    plan(&f.db, "p1", 100, 30);
    let sub = f.db.grant_subscription(&f.alice.id, "p1").unwrap();
    assert_eq!(f.db.reserve_subscription_quota(&sub.id, 100).unwrap(), 100);
    assert!(f.db.subscription_funding_source(&f.alice.id, 100).unwrap().is_none());

    f.db.refund_subscription_quota(&sub.id, 100).unwrap();

    let chosen = f.db.subscription_funding_source(&f.alice.id, 100).unwrap();
    assert!(chosen.is_some(), "the refunded pool should fund again");
}

#[test]
fn a_zero_amount_spend_needs_no_pool() {
    // A free request must not fail for want of quota, and must not be recorded.
    let f = setup();
    plan(&f.db, "p1", 100, 30);
    let sub = f.db.grant_subscription(&f.alice.id, "p1").unwrap();

    assert_eq!(
        f.db.reserve_subscription_quota(&sub.id, 0).unwrap(),
        0,
        "a zero amount takes nothing"
    );
    assert_eq!(
        f.db.list_subscriptions(&f.alice.id).unwrap()[0].amount_used,
        0
    );
}

#[test]
fn each_subscription_keeps_its_own_pool() {
    // Spending one plan must not touch another's balance.
    let f = setup();
    plan(&f.db, "a", 1_000, 30);
    plan(&f.db, "b", 1_000, 30);
    let a = f.db.grant_subscription(&f.alice.id, "a").unwrap();
    f.db.grant_subscription(&f.alice.id, "b").unwrap();

    assert_eq!(f.db.reserve_subscription_quota(&a.id, 250).unwrap(), 250);

    let listed = f.db.list_subscriptions(&f.alice.id).unwrap();
    let a_row = listed.iter().find(|s| s.plan_id == "a").unwrap();
    let b_row = listed.iter().find(|s| s.plan_id == "b").unwrap();
    assert_eq!(a_row.amount_used, 250);
    assert_eq!(b_row.amount_used, 0, "the other pool must be untouched");
}
