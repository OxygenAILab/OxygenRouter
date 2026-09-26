//! Session listing and revocation.
//!
//! The load-bearing property is the scoping: revocation is keyed on
//! `(user_id, session_id)`, not on the session id alone. Keying on the id alone
//! is the obvious implementation and would let any signed-in user sign another
//! user out by naming their session. These tests pin the scoped behaviour.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use chrono::Duration;
use oxygenrouter_core::{Database, User, UserRole};

/// Two users plus the database, so every test can reason about ownership.
struct Fixture {
    db: Database,
    _dir: tempfile::TempDir,
    alice: User,
    bob: User,
}

fn setup() -> Fixture {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("sessions.db");
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
fn a_created_session_is_listed_for_its_user() {
    let f = setup();
    let session = f.db.create_session(&f.alice.id, Duration::days(7)).unwrap();

    let sessions = f.db.sessions_for_user(&f.alice.id).unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].id, session.id);
    // The token must round-trip, or the console cannot tie a row to a live call.
    assert_eq!(sessions[0].token, session.token);
}

#[test]
fn sessions_are_scoped_to_their_own_user() {
    let f = setup();
    f.db.create_session(&f.alice.id, Duration::days(7)).unwrap();
    f.db.create_session(&f.alice.id, Duration::days(7)).unwrap();
    f.db.create_session(&f.bob.id, Duration::days(7)).unwrap();

    assert_eq!(f.db.sessions_for_user(&f.alice.id).unwrap().len(), 2);
    assert_eq!(f.db.sessions_for_user(&f.bob.id).unwrap().len(), 1);
}

#[test]
fn one_user_cannot_revoke_another_users_session() {
    // The security-relevant case: bob names alice's session id. Scoping by user
    // must make this a no-op rather than signing alice out.
    let f = setup();
    let alice_session = f.db.create_session(&f.alice.id, Duration::days(7)).unwrap();

    let revoked = f
        .db
        .revoke_session_by_id(&f.bob.id, &alice_session.id)
        .unwrap();
    assert!(
        !revoked,
        "revoking another user's session must report nothing removed"
    );

    assert_eq!(
        f.db.sessions_for_user(&f.alice.id).unwrap().len(),
        1,
        "alice's session must survive bob's attempt"
    );
    assert!(f.db.session_user(&alice_session.token).unwrap().is_some());
}

#[test]
fn a_user_can_revoke_their_own_session() {
    let f = setup();
    let session = f.db.create_session(&f.alice.id, Duration::days(7)).unwrap();

    assert!(f.db.revoke_session_by_id(&f.alice.id, &session.id).unwrap());
    assert!(f.db.sessions_for_user(&f.alice.id).unwrap().is_empty());
    // The token must stop authenticating, which is the point of revoking.
    assert!(f.db.session_user(&session.token).unwrap().is_none());
}

#[test]
fn an_expired_session_is_not_listed_and_does_not_authenticate() {
    let f = setup();
    // A negative duration puts expiry in the past without waiting for a clock.
    let expired = f
        .db
        .create_session(&f.alice.id, Duration::seconds(-60))
        .unwrap();

    assert!(f.db.sessions_for_user(&f.alice.id).unwrap().is_empty());
    assert!(f.db.session_by_token(&expired.token).unwrap().is_none());
    assert!(f.db.session_user(&expired.token).unwrap().is_none());
}

#[test]
fn revoke_others_keeps_the_named_session_and_clears_the_rest() {
    let f = setup();
    let keep = f.db.create_session(&f.alice.id, Duration::days(7)).unwrap();
    let other_a = f.db.create_session(&f.alice.id, Duration::days(7)).unwrap();
    let other_b = f.db.create_session(&f.alice.id, Duration::days(7)).unwrap();
    // Another user's session must be untouched by alice's revoke-others.
    let bob_session = f.db.create_session(&f.bob.id, Duration::days(7)).unwrap();

    let removed = f
        .db
        .revoke_other_sessions(&f.alice.id, Some(&keep.id))
        .unwrap();
    assert_eq!(removed, 2);

    let remaining = f.db.sessions_for_user(&f.alice.id).unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].id, keep.id);

    // The kept session still works; the others do not.
    assert!(f.db.session_user(&keep.token).unwrap().is_some());
    assert!(f.db.session_user(&other_a.token).unwrap().is_none());
    assert!(f.db.session_user(&other_b.token).unwrap().is_none());

    assert!(
        f.db.session_user(&bob_session.token).unwrap().is_some(),
        "another user's session must not be collateral"
    );
    assert_eq!(f.db.sessions_for_user(&f.bob.id).unwrap().len(), 1);
}

#[test]
fn revoke_others_with_no_keep_clears_every_session_for_that_user() {
    let f = setup();
    f.db.create_session(&f.alice.id, Duration::days(7)).unwrap();
    f.db.create_session(&f.alice.id, Duration::days(7)).unwrap();
    f.db.create_session(&f.bob.id, Duration::days(7)).unwrap();

    let removed = f.db.revoke_other_sessions(&f.alice.id, None).unwrap();
    assert_eq!(removed, 2);
    assert!(f.db.sessions_for_user(&f.alice.id).unwrap().is_empty());
    assert_eq!(f.db.sessions_for_user(&f.bob.id).unwrap().len(), 1);
}

#[test]
fn session_by_token_identifies_which_session_is_calling() {
    // The console marks the current session with this, which is what stops the
    // "sign out everywhere" action from signing the caller out mid-request.
    let f = setup();
    let first = f.db.create_session(&f.alice.id, Duration::days(7)).unwrap();
    let second = f.db.create_session(&f.alice.id, Duration::days(7)).unwrap();

    let found = f
        .db
        .session_by_token(&first.token)
        .unwrap()
        .expect("session found");
    assert_eq!(found.id, first.id);
    assert_ne!(found.id, second.id);
    assert!(f.db.session_by_token("not-a-token").unwrap().is_none());
}

#[test]
fn sessions_are_listed_newest_first() {
    let f = setup();
    let older = f.db.create_session(&f.alice.id, Duration::days(7)).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));
    let newer = f.db.create_session(&f.alice.id, Duration::days(7)).unwrap();

    let sessions = f.db.sessions_for_user(&f.alice.id).unwrap();
    assert_eq!(sessions[0].id, newer.id);
    assert_eq!(sessions[1].id, older.id);
}
