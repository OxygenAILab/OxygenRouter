//! TOTP conformance against RFC 6238.
//!
//! Appendix B of the RFC publishes reference codes for a known secret at known
//! timestamps. Checking against them is what makes "the algorithm is correct"
//! verifiable rather than assumed: a self-consistent implementation can be
//! internally tidy and still disagree with every authenticator app in the world,
//! and only the published vectors catch that.
//!
//! The RFC's own table is quoted with the secret as ASCII, so it exercises the
//! raw-key path directly.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use oxygenrouter_core::totp;

/// The RFC 6238 Appendix B secret, ASCII `12345678901234567890` (20 bytes =
/// the SHA-1 block size the RFC chose).
const SEED: &[u8] = b"12345678901234567890";

/// RFC 6238 Appendix B, SHA1 rows, as published (8 digits).
///
/// Asserted at 8 digits because that is the table the RFC prints, which makes
/// this a check against the *published* values rather than against my own
/// restatement of them. A 6-digit code is the same truncated integer taken
/// modulo 10^6, so the row below also pins the relationship the truncation
/// depends on — getting that backwards (`&expected[..6]`) yields the high six
/// digits and is wrong.
const APPENDIX_B_SHA1: &[(u64, &str)] = &[
    (59, "94287082"),
    (1_111_111_109, "07081804"),
    (1_111_111_111, "14050471"),
    (1_234_567_890, "89005924"),
    (2_000_000_000, "69279037"),
    (20_000_000_000, "65353130"),
];

#[test]
fn appendix_b_sha1_vectors_match_exactly() {
    for (unix_seconds, expected) in APPENDIX_B_SHA1 {
        let actual = totp::hotp_digits(SEED, totp::time_step(*unix_seconds), 8);
        assert_eq!(
            &actual, expected,
            "RFC 6238 vector at t={unix_seconds}: expected {expected}, got {actual}"
        );
    }
}

#[test]
fn a_six_digit_code_is_the_eight_digit_value_modulo_a_million() {
    // The convention that a truncated test got wrong. Six digits are the LOW six
    // of the RFC's eight, because both are `truncate(...) mod 10^digits`.
    for (unix_seconds, expected_8) in APPENDIX_B_SHA1 {
        let expected_6 = &expected_8[expected_8.len() - 6..];
        let actual = totp::code_at(SEED, *unix_seconds);
        assert_eq!(
            actual, expected_6,
            "6-digit code at t={unix_seconds} should be the low six digits"
        );
    }
}

#[test]
fn each_rfc_vector_verifies_through_the_public_check() {
    // The vectors must also pass the real entry point, not just the raw
    // generator, or the skew window or digit handling could be wrong.
    for (unix_seconds, expected_8) in APPENDIX_B_SHA1 {
        let code = &expected_8[expected_8.len() - 6..];
        assert!(
            totp::verify_at(SEED, code, *unix_seconds),
            "vector at t={unix_seconds} should verify"
        );
    }
}

#[test]
fn the_rfc_vectors_are_not_accidentally_valid_at_other_times() {
    // A guard against an over-wide skew window: the code for t=59 must not be
    // accepted at an unrelated instant.
    let code_for_59 = "287082";
    assert!(totp::verify_at(SEED, code_for_59, 59));
    assert!(
        !totp::verify_at(SEED, code_for_59, 1_111_111_111),
        "a code far from its step must be rejected"
    );
}

#[test]
fn the_time_step_is_thirty_second_buckets() {
    assert_eq!(totp::time_step(0), 0);
    assert_eq!(totp::time_step(29), 0);
    assert_eq!(totp::time_step(30), 1);
    assert_eq!(totp::time_step(59), 1);
    assert_eq!(totp::time_step(60), 2);
}
