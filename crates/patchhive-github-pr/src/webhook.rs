use anyhow::{anyhow, Result};
use hmac::{Hmac, KeyInit, Mac};
use http::HeaderMap;
use sha2::Sha256;

pub fn env_value(names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| {
        std::env::var(name)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    })
}

pub fn verify_github_webhook_signature(
    headers: &HeaderMap,
    body: &[u8],
    secret: &str,
) -> Result<()> {
    if secret.trim().is_empty() {
        return Err(anyhow!("GitHub webhook secret is empty"));
    }

    let signature = headers
        .get("X-Hub-Signature-256")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| anyhow!("Missing X-Hub-Signature-256 header"))?;

    let sig_hex = signature
        .strip_prefix("sha256=")
        .ok_or_else(|| anyhow!("Malformed GitHub webhook signature"))?;
    let sig_bytes =
        hex::decode(sig_hex).map_err(|_| anyhow!("Webhook signature was not valid hex"))?;

    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())
        .map_err(|_| anyhow!("Could not initialize webhook verifier"))?;
    mac.update(body);
    mac.verify_slice(&sig_bytes)
        .map_err(|_| anyhow!("GitHub webhook signature did not match"))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::HeaderValue;

    fn signed_headers(body: &[u8], secret: &str) -> HeaderMap {
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("hmac init");
        mac.update(body);
        let signature = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));

        let mut headers = HeaderMap::new();
        headers.insert(
            "X-Hub-Signature-256",
            HeaderValue::from_str(&signature).expect("signature header"),
        );
        headers
    }

    #[test]
    fn verify_github_webhook_signature_accepts_valid_signature() {
        let body = br#"{"action":"opened"}"#;
        let headers = signed_headers(body, "super-secret");

        verify_github_webhook_signature(&headers, body, "super-secret")
            .expect("signature should verify");
    }

    #[test]
    fn verify_github_webhook_signature_rejects_wrong_secret() {
        let body = br#"{"action":"opened"}"#;
        let headers = signed_headers(body, "super-secret");

        let error = verify_github_webhook_signature(&headers, body, "wrong-secret")
            .expect_err("signature should fail");
        assert!(error.to_string().contains("did not match"));
    }

    #[test]
    fn verify_github_webhook_signature_rejects_malformed_header() {
        let mut headers = HeaderMap::new();
        headers.insert("X-Hub-Signature-256", HeaderValue::from_static("nope"));

        let error = verify_github_webhook_signature(&headers, b"{}", "super-secret")
            .expect_err("malformed signature should fail");
        assert!(error.to_string().contains("Malformed"));
    }
}
