//! Two-factor storage: staging, activation, backup codes, disable.
//!
//! The load-bearing security property is that backup codes are stored as salted
//! digests, never in the clear. They are equivalent to a second password, and the
//! obvious implementation — keeping the generated strings so they can be shown
//! again — would mean a database read yields usable recovery credentials. The
//! tests below assert the stored value is a digest by checking the plaintext does
//! not appear, and that a code works exactly once.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use oxygenrouter_core::{Database, User, UserRole};

struct Fixture {
    db: Database,
    path: std::path::PathBuf,
    alice: User,
    bob: User,
}

fn setup() -> Fixture {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("twofa.db");
    // The directory is leaked deliberately: only the path matters to the store,
    // and the file must outlive this function.
    let db_path = path.clone();
    std::mem::forget(dir);
    let db = Database::new(&db_path).expect("database opens");
    let alice = db
        .create_user("alice", "alice@example.test", "password-1", UserRole::User)
        .expect("alice");
    let bob = db
        .create_user("bob", "bob@example.test", "password-2", UserRole::User)
        .expect("bob");
    Fixture {
        db,
        path: db_path,
        alice,
        bob,
    }
}

/// Read the raw stored value of a column, bypassing the API, so a test can prove
/// what actually landed on disk.
fn raw_column(f: &Fixture, user_id: &str, column: &str) -> Option<String> {
    // A separate connection so we are not reading through any in-process cache.
    let conn = rusqlite::Connection::open(&f.path).expect("open");
    conn.query_row(
        &format!("SELECT {column} FROM users WHERE id=?1"),
        rusqlite::params![user_id],
        |row| row.get::<_, Option<String>>(0),
    )
    .expect("query")
}

#[test]
fn two_factor_is_off_by_default() {
    let f = setup();
    assert!(f.db.enabled_two_fa_secret(&f.alice.id).unwrap().is_none());
    assert!(f.db.pending_two_fa_secret(&f.alice.id).unwrap().is_none());
    assert!(f.db.two_fa_backup_code_digests(&f.alice.id).unwrap().is_empty());
}

#[test]
fn staging_a_secret_does_not_activate_two_factor() {
    // Setup must be safe: the secret is stored but 2FA is not yet gating logins,
    // so a mis-scanned or discarded QR code cannot lock the account out.
    let f = setup();
    f.db.set_pending_two_fa_secret(&f.alice.id, "ABCDEFGHIJKLMNOP").unwrap();

    assert_eq!(
        f.db.pending_two_fa_secret(&f.alice.id).unwrap().as_deref(),
        Some("ABCDEFGHIJKLMNOP")
    );
    assert!(
        f.db.enabled_two_fa_secret(&f.alice.id).unwrap().is_none(),
        "a staged secret must not count as enabled"
    );
}

#[test]
fn enabling_activates_the_secret_and_issues_backup_codes() {
    let f = setup();
    f.db.set_pending_two_fa_secret(&f.alice.id, "ABCDEFGHIJKLMNOP").unwrap();
    let codes = vec!["AAAA1111".to_string(), "BBBB2222".to_string()];
    f.db.enable_two_fa(&f.alice.id, &codes).unwrap();

    assert_eq!(
        f.db.enabled_two_fa_secret(&f.alice.id).unwrap().as_deref(),
        Some("ABCDEFGHIJKLMNOP")
    );
    assert_eq!(f.db.two_fa_backup_code_digests(&f.alice.id).unwrap().len(), 2);
}

#[test]
fn backup_codes_are_never_stored_in_the_clear() {
    // The security-critical property. If the raw column contains the code, a
    // database read hands out a working recovery credential.
    let f = setup();
    f.db.set_pending_two_fa_secret(&f.alice.id, "ABCDEFGHIJKLMNOP").unwrap();
    let code = "SECRETCODE".to_string();
    f.db.enable_two_fa(&f.alice.id, &[code.clone()]).unwrap();

    let raw = raw_column(&f, &f.alice.id, "two_fa_backup_codes").expect("a value");
    assert!(
        !raw.contains(&code),
        "the plaintext code must not appear in storage: {raw}"
    );
    // And it is still a real digest, not an empty placeholder.
    assert!(raw.len() > 20, "expected a hash, got: {raw}");
}

#[test]
fn a_backup_code_works_exactly_once() {
    let f = setup();
    f.db.set_pending_two_fa_secret(&f.alice.id, "ABCDEFGHIJKLMNOP").unwrap();
    let code = "ONETIME01".to_string();
    f.db.enable_two_fa(&f.alice.id, &[code.clone(), "OTHER234".to_string()])
        .unwrap();

    assert!(f.db.consume_two_fa_backup_code(&f.alice.id, &code).unwrap());
    assert!(
        !f.db.consume_two_fa_backup_code(&f.alice.id, &code).unwrap(),
        "a consumed code must not be reusable"
    );
    // The other code is unaffected.
    assert!(f.db
        .consume_two_fa_backup_code(&f.alice.id, "OTHER234")
        .unwrap());
    assert!(f.db.two_fa_backup_code_digests(&f.alice.id).unwrap().is_empty());
}

#[test]
fn a_wrong_backup_code_is_rejected_and_changes_nothing() {
    let f = setup();
    f.db.set_pending_two_fa_secret(&f.alice.id, "ABCDEFGHIJKLMNOP").unwrap();
    f.db.enable_two_fa(&f.alice.id, &["REALCODE".to_string()]).unwrap();

    assert!(!f.db.consume_two_fa_backup_code(&f.alice.id, "WRONGCOD").unwrap());
    assert_eq!(
        f.db.two_fa_backup_code_digests(&f.alice.id).unwrap().len(),
        1,
        "a failed attempt must not consume anything"
    );
}

#[test]
fn a_backup_code_is_matched_regardless_of_case_or_separators() {
    // A user reading the code off paper may type it in lower case or with a dash.
    let f = setup();
    f.db.set_pending_two_fa_secret(&f.alice.id, "ABCDEFGHIJKLMNOP").unwrap();
    f.db.enable_two_fa(&f.alice.id, &["ABCD1234".to_string()]).unwrap();

    assert!(f.db.consume_two_fa_backup_code(&f.alice.id, "abcd-1234").unwrap());
}

#[test]
fn backup_codes_do_not_cross_between_users() {
    let f = setup();
    f.db.set_pending_two_fa_secret(&f.alice.id, "ALICESECRET").unwrap();
    f.db.enable_two_fa(&f.alice.id, &["ALICECODE".to_string()]).unwrap();
    f.db.set_pending_two_fa_secret(&f.bob.id, "BOBSECRET").unwrap();
    f.db.enable_two_fa(&f.bob.id, &["BOBCODE22".to_string()]).unwrap();

    assert!(
        !f.db.consume_two_fa_backup_code(&f.bob.id, "ALICECODE").unwrap(),
        "bob must not be able to use alice's code"
    );
    assert_eq!(f.db.two_fa_backup_code_digests(&f.alice.id).unwrap().len(), 1);
}

#[test]
fn disabling_clears_the_secret_and_the_backup_codes() {
    // Leaving either behind would let a later re-enable silently reuse a value
    // the user believes they revoked.
    let f = setup();
    f.db.set_pending_two_fa_secret(&f.alice.id, "ABCDEFGHIJKLMNOP").unwrap();
    f.db.enable_two_fa(&f.alice.id, &["SOMECODE1".to_string()]).unwrap();

    f.db.disable_two_fa(&f.alice.id).unwrap();

    assert!(f.db.enabled_two_fa_secret(&f.alice.id).unwrap().is_none());
    assert!(f.db.pending_two_fa_secret(&f.alice.id).unwrap().is_none());
    assert!(f.db.two_fa_backup_code_digests(&f.alice.id).unwrap().is_empty());
    assert!(!f.db.consume_two_fa_backup_code(&f.alice.id, "SOMECODE1").unwrap());
}

#[test]
fn disabling_one_user_leaves_another_enabled() {
    let f = setup();
    f.db.set_pending_two_fa_secret(&f.alice.id, "ALICESECRET").unwrap();
    f.db.enable_two_fa(&f.alice.id, &[]).unwrap();
    f.db.set_pending_two_fa_secret(&f.bob.id, "BOBSECRET").unwrap();
    f.db.enable_two_fa(&f.bob.id, &[]).unwrap();

    f.db.disable_two_fa(&f.alice.id).unwrap();

    assert!(f.db.enabled_two_fa_secret(&f.alice.id).unwrap().is_none());
    assert_eq!(
        f.db.enabled_two_fa_secret(&f.bob.id).unwrap().as_deref(),
        Some("BOBSECRET"),
        "bob's two-factor must not be collateral"
    );
}

#[test]
fn restarting_a_setup_replaces_a_stale_pending_secret() {
    let f = setup();
    f.db.set_pending_two_fa_secret(&f.alice.id, "FIRSTSECRET").unwrap();
    f.db.set_pending_two_fa_secret(&f.alice.id, "SECONDSECRET").unwrap();

    assert_eq!(
        f.db.pending_two_fa_secret(&f.alice.id).unwrap().as_deref(),
        Some("SECONDSECRET")
    );
}

#[test]
fn a_new_setup_clears_backup_codes_from_a_previous_one() {
    let f = setup();
    f.db.set_pending_two_fa_secret(&f.alice.id, "OLD").unwrap();
    f.db.enable_two_fa(&f.alice.id, &["OLDCODE11".to_string()]).unwrap();

    f.db.set_pending_two_fa_secret(&f.alice.id, "NEW").unwrap();

    assert!(
        f.db.two_fa_backup_code_digests(&f.alice.id).unwrap().is_empty(),
        "codes from the previous enrolment must not survive"
    );
    assert!(
        f.db.enabled_two_fa_secret(&f.alice.id).unwrap().is_none(),
        "a restarted setup leaves two-factor inactive until confirmed"
    );
}
