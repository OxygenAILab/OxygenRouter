//! AWS Signature Version 4 request signing.
//!
//! Implements the standard SigV4 algorithm: canonical request with the SHA-256
//! payload hash, signed headers, a scope, and a derived signing key. No AWS SDK.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use hmac::{Hmac, Mac};
use reqwest::header::{HeaderMap, HeaderValue, HOST};
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Clone)]
pub struct AwsCredentials {
    pub access_key: String,
    pub secret_key: String,
    pub region: String,
    pub service: String,
}

/// Sign `headers` in place for the given method/url/body.
pub fn sign(
    cred: &AwsCredentials,
    method: &str,
    url: &str,
    headers: &mut HeaderMap,
    body: &[u8],
) -> Result<(), String> {
    let parsed = url::Url::parse(url).map_err(|e| format!("bad url: {}", e))?;
    let host = parsed
        .host_str()
        .ok_or_else(|| "url has no host".to_string())?;
    let path = if parsed.path().is_empty() {
        "/"
    } else {
        parsed.path()
    };
    let query = canonical_query(parsed.query().unwrap_or(""));

    let now = chrono::Utc::now();
    let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
    let date_stamp = now.format("%Y%m%d").to_string();

    let payload_hash = hex::encode(Sha256::digest(body));

    headers.insert(HOST, HeaderValue::from_str(host).map_err(|e| e.to_string())?);
    headers.insert(
        "x-amz-date",
        HeaderValue::from_str(&amz_date).map_err(|e| e.to_string())?,
    );
    headers.insert(
        "x-amz-content-sha256",
        HeaderValue::from_str(&payload_hash).map_err(|e| e.to_string())?,
    );

    // Canonical headers — sorted lowercase name:trimmed value.
    let mut signed: Vec<(String, String)> = Vec::new();
    for (name, value) in headers.iter() {
        let n = name.as_str().to_lowercase();
        if n == "authorization" || n == "content-length" {
            continue;
        }
        if let Ok(v) = value.to_str() {
            signed.push((n, v.trim().to_string()));
        }
    }
    signed.sort_by(|a, b| a.0.cmp(&b.0));
    let canonical_headers: String = signed
        .iter()
        .map(|(n, v)| format!("{}:{}\n", n, v))
        .collect();
    let signed_headers: String = signed
        .iter()
        .map(|(n, _)| n.clone())
        .collect::<Vec<_>>()
        .join(";");

    let canonical_request = format!(
        "{}\n{}\n{}\n{}\n{}\n{}",
        method, path, query, canonical_headers, signed_headers, payload_hash
    );

    let scope = format!(
        "{}/{}/{}/aws4_request",
        date_stamp, cred.region, cred.service
    );
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{}\n{}\n{}",
        amz_date,
        scope,
        hex::encode(Sha256::digest(canonical_request.as_bytes()))
    );

    let signing_key = derive_signing_key(&cred.secret_key, &date_stamp, &cred.region, &cred.service);
    let signature = hex::encode(hmac(&signing_key, string_to_sign.as_bytes()));

    let authorization = format!(
        "AWS4-HMAC-SHA256 Credential={}/{}, SignedHeaders={}, Signature={}",
        cred.access_key, scope, signed_headers, signature
    );
    headers.insert(
        "authorization",
        HeaderValue::from_str(&authorization).map_err(|e| e.to_string())?,
    );

    Ok(())
}

fn canonical_query(query: &str) -> String {
    if query.is_empty() {
        return String::new();
    }
    let mut pairs: Vec<(String, String)> = query
        .split('&')
        .filter(|s| !s.is_empty())
        .map(|kv| match kv.split_once('=') {
            Some((k, v)) => (k.to_string(), v.to_string()),
            None => (kv.to_string(), String::new()),
        })
        .collect();
    pairs.sort();
    pairs
        .into_iter()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect::<Vec<_>>()
        .join("&")
}

fn hmac(key: &[u8], msg: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("hmac accepts any key length");
    mac.update(msg);
    mac.finalize().into_bytes().to_vec()
}

fn derive_signing_key(secret: &str, date: &str, region: &str, service: &str) -> Vec<u8> {
    let k_date = hmac(format!("AWS4{}", secret).as_bytes(), date.as_bytes());
    let k_region = hmac(&k_date, region.as_bytes());
    let k_service = hmac(&k_region, service.as_bytes());
    hmac(&k_service, b"aws4_request")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// AWS-published SigV4 test vector shape: determinism + key derivation sanity.
    #[test]
    fn derive_key_is_deterministic() {
        let a = derive_signing_key("wJalrXUtnFEMI", "20150830", "us-east-1", "iam");
        let b = derive_signing_key("wJalrXUtnFEMI", "20150830", "us-east-1", "iam");
        assert_eq!(a, b);
        assert_eq!(a.len(), 32);
    }

    #[test]
    fn sign_inserts_authorization() {
        let cred = AwsCredentials {
            access_key: "AKID".into(),
            secret_key: "secret".into(),
            region: "us-east-1".into(),
            service: "bedrock".into(),
        };
        let mut headers = HeaderMap::new();
        sign(&cred, "POST", "https://bedrock.us-east-1.amazonaws.com/x", &mut headers, b"{}").unwrap();
        let auth = headers.get("authorization").unwrap().to_str().unwrap();
        assert!(auth.starts_with("AWS4-HMAC-SHA256 Credential=AKID/"));
        assert!(headers.get("x-amz-date").is_some());
    }
}
