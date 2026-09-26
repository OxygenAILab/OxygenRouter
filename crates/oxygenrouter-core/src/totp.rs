//! TOTP (RFC 6238) for two-factor authentication.
//!
//! Implemented against the RFC rather than pulled from a crate so the parameters
//! the reference uses are explicit and testable: HMAC-SHA1, a 30-second period,
//! six digits — the same set `common.GenerateTOTPSecret` asks for. RFC 6238
//! Appendix B publishes reference codes, and `tests/totp.rs` checks this
//! implementation against them, which is what makes "the algorithm is right"
//! verifiable rather than assumed.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use hmac::{Hmac, Mac};
use sha1::Sha1;

type HmacSha1 = Hmac<Sha1>;

/// Seconds per time step, fixed by the reference (and by every authenticator app).
pub const PERIOD_SECONDS: u64 = 30;

/// Digits in a generated code.
pub const DIGITS: u32 = 6;

/// How many steps either side of the current one are accepted.
///
/// One step each way tolerates clock drift and a code entered as the period rolls
/// over, without widening the window far enough to make guessing easier. This
/// matches the common `totp.Validate` behaviour the reference relies on.
pub const SKEW_STEPS: i64 = 1;

/// The raw dynamic-truncation value for a counter, per RFC 4226 §5.3.
///
/// Public because RFC 6238's published vectors are 8-digit: asserting against
/// them directly is stronger evidence than re-deriving a 6-digit expectation, so
/// the tests need the untruncated value.
pub fn hotp_value(secret: &[u8], counter: u64) -> u32 {
    let mut mac = HmacSha1::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(&counter.to_be_bytes());
    let digest = mac.finalize().into_bytes();

    // Dynamic truncation: the low nibble of the last byte selects a 4-byte window,
    // and the high bit of that window is masked off so the value is positive.
    //
    // This returns the 31-bit value *before* the `mod 10^digits` step. Folding
    // the modulus in here would make a wider request (the RFC's 8-digit vectors)
    // re-reduce an already-reduced value and silently lose the high digits.
    let offset = (digest[digest.len() - 1] & 0x0f) as usize;
    let binary = ((digest[offset] as u32 & 0x7f) << 24)
        | ((digest[offset + 1] as u32) << 16)
        | ((digest[offset + 2] as u32) << 8)
        | (digest[offset + 3] as u32);
    binary
}

/// Format a HOTP value for a counter with `digits` digits.
///
/// Zero-padded, as every TOTP implementation and authenticator app expects.
pub fn hotp_digits(secret: &[u8], counter: u64, digits: u32) -> String {
    format!(
        "{:0width$}",
        hotp_value(secret, counter) % 10u32.pow(digits),
        width = digits as usize
    )
}

/// The time step a unix timestamp falls in.
pub fn time_step(unix_seconds: u64) -> u64 {
    unix_seconds / PERIOD_SECONDS
}

/// The code for `unix_seconds`. Exposed so tests can pin exact instants.
pub fn code_at(secret: &[u8], unix_seconds: u64) -> String {
    hotp_digits(secret, time_step(unix_seconds), DIGITS)
}

/// The current code.
pub fn code_now(secret: &[u8]) -> String {
    code_at(secret, chrono::Utc::now().timestamp().max(0) as u64)
}

/// Whether `candidate` is valid for `unix_seconds`, allowing [`SKEW_STEPS`].
///
/// The comparison is constant-time over the code, so a wrong code cannot be
/// narrowed down by timing how long the check takes.
pub fn verify_at(secret: &[u8], candidate: &str, unix_seconds: u64) -> bool {
    let cleaned: String = candidate.chars().filter(|c| !c.is_whitespace()).collect();
    if cleaned.len() != DIGITS as usize || !cleaned.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    let step = time_step(unix_seconds) as i64;
    (-SKEW_STEPS..=SKEW_STEPS).any(|delta| {
        let candidate_step = step + delta;
        if candidate_step < 0 {
            return false;
        }
        let expected = hotp_digits(secret, candidate_step as u64, DIGITS);
        constant_time_eq(expected.as_bytes(), cleaned.as_bytes())
    })
}

/// The current-time variant.
pub fn verify(secret: &[u8], candidate: &str) -> bool {
    verify_at(secret, candidate, chrono::Utc::now().timestamp().max(0) as u64)
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// A fresh base32 secret, sized for HMAC-SHA1 as RFC 4226 §4 recommends (160 bits).
///
/// Entropy comes from UUIDv4 bytes, which are CSPRNG-backed, rather than a weaker
/// source: this value is the second factor, so its unpredictability is the point.
pub fn generate_secret() -> String {
    let mut bytes = Vec::with_capacity(20);
    while bytes.len() < 20 {
        bytes.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
    }
    bytes.truncate(20);
    base32_encode(&bytes)
}

/// A `otpauth://` URI an authenticator app can consume.
///
/// The label and issuer are percent-encoded: a username containing a space or `:`
/// would otherwise produce a URI the app parses incorrectly.
pub fn provisioning_uri(issuer: &str, account: &str, secret: &str) -> String {
    format!(
        "otpauth://totp/{}:{}?secret={}&issuer={}&algorithm=SHA1&digits={}&period={}",
        percent_encode(issuer),
        percent_encode(account),
        secret,
        percent_encode(issuer),
        DIGITS,
        PERIOD_SECONDS
    )
}

fn percent_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{:02X}", byte)),
        }
    }
    out
}

// ── Base32 (RFC 4648) ─────────────────────────────────────────────────────
//
// Hand-rolled rather than dependency-pulled: it is a dozen lines, and the only
// consumer is the secret encoding above.

const BASE32_ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/// Encode bytes as RFC 4648 base32 without padding.
///
/// Unpadded is what authenticator apps emit and accept, and a padded secret typed
/// by hand tends to lose its `=` characters.
pub fn base32_encode(data: &[u8]) -> String {
    let mut out = String::new();
    let mut buffer: u32 = 0;
    let mut bits = 0u32;

    for &byte in data {
        buffer = (buffer << 8) | byte as u32;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            let index = ((buffer >> bits) & 0x1f) as usize;
            out.push(BASE32_ALPHABET[index] as char);
        }
    }
    if bits > 0 {
        let index = ((buffer << (5 - bits)) & 0x1f) as usize;
        out.push(BASE32_ALPHABET[index] as char);
    }
    out
}

/// Decode RFC 4648 base32, tolerating padding and lower case.
///
/// A user pasting a secret from an app may include `=` padding or lower case, and
/// rejecting those would be a usability failure rather than a security one.
pub fn base32_decode(value: &str) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut buffer: u32 = 0;
    let mut bits = 0u32;

    for ch in value.chars() {
        if ch == '=' {
            continue;
        }
        let upper = ch.to_ascii_uppercase() as u8;
        let index = BASE32_ALPHABET.iter().position(|&c| c == upper)? as u32;
        buffer = (buffer << 5) | index;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            bytes.push(((buffer >> bits) & 0xff) as u8);
        }
    }
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base32_round_trips() {
        let raw = b"hello world!";
        let encoded = base32_encode(raw);
        assert_eq!(base32_decode(&encoded).unwrap(), raw);
    }

    #[test]
    fn base32_matches_a_known_encoding() {
        // RFC 4648 §10: "foobar" -> "MZXW6YTBOI".
        assert_eq!(base32_encode(b"foobar"), "MZXW6YTBOI");
    }

    #[test]
    fn base32_decode_tolerates_padding_and_case() {
        assert_eq!(base32_decode("mzxw6ytboi").unwrap(), b"foobar");
        assert_eq!(base32_decode("MZXW6YTBOI======").unwrap(), b"foobar");
    }

    #[test]
    fn base32_decode_rejects_a_non_alphabet_character() {
        assert!(base32_decode("MZXW6YTBO!").is_none());
    }

    #[test]
    fn a_generated_secret_is_the_right_shape() {
        let secret = generate_secret();
        // 20 bytes encodes to 32 base32 characters, unpadded.
        assert_eq!(secret.len(), 32, "secret: {secret}");
        assert!(secret.chars().all(|c| BASE32_ALPHABET.contains(&(c as u8))));
        assert_eq!(base32_decode(&secret).unwrap().len(), 20);
    }

    #[test]
    fn two_secrets_differ() {
        assert_ne!(generate_secret(), generate_secret());
    }

    #[test]
    fn the_provisioning_uri_carries_the_parameters_apps_need() {
        let uri = provisioning_uri("OxygenRouter", "alice", "ABCDEF");
        assert!(uri.starts_with("otpauth://totp/OxygenRouter:alice?"));
        assert!(uri.contains("secret=ABCDEF"));
        assert!(uri.contains("issuer=OxygenRouter"));
        assert!(uri.contains("algorithm=SHA1"));
        assert!(uri.contains("digits=6"));
        assert!(uri.contains("period=30"));
    }

    #[test]
    fn the_provisioning_uri_escapes_awkward_account_names() {
        // A space or colon would break the label if left raw.
        let uri = provisioning_uri("Oxygen Router", "a b:c", "ABC");
        assert!(uri.contains("Oxygen%20Router"), "{uri}");
        assert!(uri.contains("a%20b%3Ac"), "{uri}");
    }

    #[test]
    fn a_code_is_always_six_digits() {
        let secret = b"12345678901234567890";
        for second in [0u64, 1, 59, 1111111109, 2000000000] {
            let code = code_at(secret, second);
            assert_eq!(code.len(), 6, "code {code} at {second}");
            assert!(code.chars().all(|c| c.is_ascii_digit()));
        }
    }

    #[test]
    fn the_current_code_verifies() {
        let secret = b"12345678901234567890";
        let now = chrono::Utc::now().timestamp().max(0) as u64;
        assert!(verify_at(secret, &code_at(secret, now), now));
    }

    #[test]
    fn a_wrong_code_is_rejected() {
        let secret = b"12345678901234567890";
        let now = 1_111_111_111u64;
        let good = code_at(secret, now);
        let bad = if good == "000000" { "000001" } else { "000000" };
        assert!(!verify_at(secret, bad, now));
    }

    #[test]
    fn malformed_codes_are_rejected_without_panicking() {
        let secret = b"12345678901234567890";
        let now = 1_111_111_111u64;
        for candidate in ["", "1", "12345", "1234567", "abcdef", "12 34 5", "１２３４５６"] {
            assert!(
                !verify_at(secret, candidate, now),
                "{candidate:?} should be rejected"
            );
        }
    }

    #[test]
    fn whitespace_inside_a_code_is_ignored() {
        // Authenticator apps display codes as "123 456"; a user may paste that.
        let secret = b"12345678901234567890";
        let now = 1_111_111_111u64;
        let good = code_at(secret, now);
        let spaced = format!("{} {}", &good[..3], &good[3..]);
        assert!(verify_at(secret, &spaced, now));
    }

    #[test]
    fn a_code_from_the_neighbouring_step_is_accepted_but_a_distant_one_is_not() {
        let secret = b"12345678901234567890";
        let now = 1_111_111_111u64;
        assert!(verify_at(secret, &code_at(secret, now - PERIOD_SECONDS), now));
        assert!(verify_at(secret, &code_at(secret, now + PERIOD_SECONDS), now));
        // Two steps away is outside the window.
        assert!(!verify_at(
            secret,
            &code_at(secret, now + 2 * PERIOD_SECONDS),
            now
        ));
    }

    #[test]
    fn a_code_does_not_verify_against_a_different_secret() {
        let code = code_at(b"12345678901234567890", 1_111_111_111);
        assert!(!verify_at(b"09876543210987654321", &code, 1_111_111_111));
    }
}
