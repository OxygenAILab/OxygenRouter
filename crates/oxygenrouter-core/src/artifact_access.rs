//! The artifact-access capability: a stable, path-bound URL credential.
//!
//! Ported from `service/task_artifact_access.go`. A capability is an
//! HMAC-SHA256 over `"v1\0{taskID}\0{artifactKey}"` keyed by the instance's
//! crypto secret, base64url-encoded without padding. It contains no user or
//! upstream data and is bound to exactly one task and one artifact, so a URL
//! handed to a client cannot be replayed against another object. Verification
//! reads no database state and compares in constant time, which is what lets
//! the router refuse a bad capability before touching the task row.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use hmac::{Hmac, Mac};
use sha2::Sha256;

/// The capability's wire version, part of the signed message so a future
/// change cannot be confused with this one.
const ACCESS_VERSION: &str = "v1";
/// 32 signature bytes in unpadded base64url.
const ACCESS_LENGTH: usize = 43;
/// The reference's bounds (`service/task_artifact_access.go:20-21`).
const MAX_TASK_ID_LENGTH: usize = 191;
const MAX_ARTIFACT_KEY_LENGTH: usize = 128;

fn message(task_id: &str, artifact_key: &str) -> Vec<u8> {
    format!("{ACCESS_VERSION}\0{task_id}\0{artifact_key}").into_bytes()
}

/// Issue the capability for one task and artifact, or `None` when the inputs or
/// the secret cannot support one.
pub fn issue(secret: &str, task_id: &str, artifact_key: &str) -> Option<String> {
    let task_id = task_id.trim();
    let artifact_key = artifact_key.trim();
    if !usable(secret, task_id, artifact_key) {
        return None;
    }
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).ok()?;
    mac.update(&message(task_id, artifact_key));
    Some(URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes()))
}

/// Verify a capability against the route it claims to open.
///
/// Constant-time comparison, and no database read: a tampered or empty value is
/// simply not this object's capability.
pub fn verify(secret: &str, access: &str, task_id: &str, artifact_key: &str) -> bool {
    let task_id = task_id.trim();
    let artifact_key = artifact_key.trim();
    if access.len() != ACCESS_LENGTH || !usable(secret, task_id, artifact_key) {
        return false;
    }
    let Ok(signature) = URL_SAFE_NO_PAD.decode(access) else {
        return false;
    };
    if signature.len() != 32 {
        return false;
    }
    let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(secret.as_bytes()) else {
        return false;
    };
    mac.update(&message(task_id, artifact_key));
    mac.verify_slice(&signature).is_ok()
}

fn usable(secret: &str, task_id: &str, artifact_key: &str) -> bool {
    !secret.is_empty()
        && !task_id.is_empty()
        && task_id.len() <= MAX_TASK_ID_LENGTH
        && !artifact_key.is_empty()
        && artifact_key.len() <= MAX_ARTIFACT_KEY_LENGTH
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "test-secret";

    #[test]
    fn a_capability_verifies_only_against_its_own_object() {
        let access = issue(SECRET, "task-1", "video").expect("issue");
        assert_eq!(access.len(), ACCESS_LENGTH);
        assert!(verify(SECRET, &access, "task-1", "video"));
        // Bound to the task, the artifact and the secret.
        assert!(!verify(SECRET, &access, "task-2", "video"));
        assert!(!verify(SECRET, &access, "task-1", "audio"));
        assert!(!verify("other-secret", &access, "task-1", "video"));
    }

    #[test]
    fn tampered_or_malformed_capabilities_are_refused() {
        let access = issue(SECRET, "task-1", "video").expect("issue");
        let mut tampered = access.clone();
        tampered.replace_range(0..1, if access.starts_with('A') { "B" } else { "A" });
        assert!(!verify(SECRET, &tampered, "task-1", "video"));
        assert!(!verify(SECRET, "", "task-1", "video"));
        assert!(!verify(SECRET, "not-a-signature", "task-1", "video"));
        assert!(!verify(SECRET, &access, "", "video"));
    }

    #[test]
    fn issuing_refuses_what_cannot_be_bound() {
        assert!(issue("", "task-1", "video").is_none());
        assert!(issue(SECRET, "", "video").is_none());
        assert!(issue(SECRET, "task-1", "").is_none());
        assert!(issue(SECRET, &"t".repeat(MAX_TASK_ID_LENGTH + 1), "video").is_none());
        assert!(issue(SECRET, "task-1", &"k".repeat(MAX_ARTIFACT_KEY_LENGTH + 1)).is_none());
    }
}
