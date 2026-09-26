//! User access tokens: generation, rotation, revocation, and lookup.
//!
//! An access token is a long-lived credential held on the user row, distinct from
//! a login session. The properties worth pinning are that regeneration rotates
//! rather than accumulating, that revocation is scoped to the named user, and
//! that a token belongs to exactly one account.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use oxygenrouter_core::{Database, User, UserRole};

struct Fixture {
    db: Database,
    _dir: tempfile::TempDir,
    alice: User,
    bob: User,
}

fn setup() -> Fixture {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("tokens.db");
    let db = Database::new(&path).expect("database opens");
    let alice = db
        .create_user("alice", "alice@example.test", "password-1", UserRole::User)
        .expect("alice");
    let bob = db
        .create_user("bob", "bob@example.test", "password-2", UserRole::User)
        .expect("bob");
    Fixture {
        db,
        _dir: dir,
        alice,
        bob,
    }
}

#[test]
fn a_fresh_user_has_no_access_token() {
    let f = setup();
    assert!(f.db.access_token_status(&f.alice.id).unwrap().is_none());
    assert!(f.db.user_by_access_token("anything").unwrap().is_none());
}

#[test]
fn setting_a_token_activates_it_for_that_user_only() {
    let f = setup();
    f.db.set_access_token(&f.alice.id, "token-alice").unwrap();

    let found = f
        .db
        .user_by_access_token("token-alice")
        .unwrap()
        .expect("token resolves");
    assert_eq!(found.id, f.alice.id);
    // It must not resolve to anyone else.
    assert!(f.db.access_token_status(&f.bob.id).unwrap().is_none());
}

#[test]
fn generating_again_rotates_rather_than_accumulating() {
    // The reference keeps at most one token per user, so a regeneration replaces
    // the old one and the previous value stops working.
    let f = setup();
    f.db.set_access_token(&f.alice.id, "first").unwrap();
    f.db.set_access_token(&f.alice.id, "second").unwrap();

    assert!(
        f.db.user_by_access_token("second").unwrap().is_some(),
        "the new token must work"
    );
    assert!(
        f.db.user_by_access_token("first").unwrap().is_none(),
        "the rotated-out token must stop working"
    );
}

#[test]
fn revocation_clears_the_token_and_reports_whether_one_existed() {
    let f = setup();
    f.db.set_access_token(&f.alice.id, "token-alice").unwrap();

    assert!(f.db.revoke_access_token(&f.alice.id).unwrap());
    assert!(f.db.access_token_status(&f.alice.id).unwrap().is_none());
    assert!(f.db.user_by_access_token("token-alice").unwrap().is_none());

    // Revoking again is a no-op, not an error.
    assert!(!f.db.revoke_access_token(&f.alice.id).unwrap());
}

#[test]
fn one_users_revocation_leaves_anothers_token_alone() {
    let f = setup();
    f.db.set_access_token(&f.alice.id, "token-alice").unwrap();
    f.db.set_access_token(&f.bob.id, "token-bob").unwrap();

    f.db.revoke_access_token(&f.alice.id).unwrap();

    assert!(f.db.user_by_access_token("token-alice").unwrap().is_none());
    assert!(
        f.db.user_by_access_token("token-bob").unwrap().is_some(),
        "bob's token must not be collateral"
    );
}

#[test]
fn the_status_reports_when_the_token_was_created() {
    let f = setup();
    f.db.set_access_token(&f.alice.id, "token-alice").unwrap();

    let created = f
        .db
        .access_token_status(&f.alice.id)
        .unwrap()
        .expect("a timestamp");
    // Within a generous window of now, which is all this needs to assert.
    let age = chrono::Utc::now().signed_duration_since(created);
    assert!(age.num_seconds().abs() < 60, "timestamp should be ~now");
}

#[test]
fn a_deactivated_account_stops_resolving_its_token() {
    // The lookup joins on status, so disabling a user ends their API access
    // without having to remember to revoke the token separately.
    let f = setup();
    f.db.set_access_token(&f.alice.id, "token-alice").unwrap();
    assert!(f.db.user_by_access_token("token-alice").unwrap().is_some());

    f.db.update_user(
        &f.alice.id,
        "alice",
        "alice@example.test",
        UserRole::User,
        "disabled",
    )
    .unwrap();

    assert!(f.db.user_by_access_token("token-alice").unwrap().is_none());
}

#[test]
fn existence_check_finds_a_live_token_and_not_a_missing_one() {
    // The generator consults this before handing a value out, so a collision is
    // detected rather than silently giving one user another's credential.
    let f = setup();
    f.db.set_access_token(&f.alice.id, "taken").unwrap();
    assert!(f.db.access_token_exists("taken").unwrap());
    assert!(!f.db.access_token_exists("free").unwrap());
}
