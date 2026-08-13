use aes_gcm::{
    aead::{Aead, Generate, KeyInit},
    Aes256Gcm, Nonce,
};
use anyhow::{anyhow, Context, Result};
use sha2::{Digest, Sha256};

const ENCRYPTED_SECRET_PREFIX: &str = "enc:v1:";

#[derive(Clone, Debug, Default)]
pub struct TokenProtector {
    key: Option<[u8; 32]>,
}

impl TokenProtector {
    pub fn from_env(env_key: &str) -> Self {
        Self::from_secret(
            std::env::var(env_key)
                .ok()
                .filter(|value| !value.trim().is_empty())
                .as_deref(),
        )
    }

    pub fn from_env_candidates(env_keys: &[&str]) -> Self {
        let secret = env_keys.iter().find_map(|env_key| {
            std::env::var(env_key)
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        });
        Self::from_secret(secret.as_deref())
    }

    pub fn configured(&self) -> bool {
        self.key.is_some()
    }

    pub fn is_encrypted_value(raw: &str) -> bool {
        raw.trim_start().starts_with(ENCRYPTED_SECRET_PREFIX)
    }

    pub fn protect_for_storage(&self, plaintext: &str) -> Result<String> {
        if plaintext.trim().is_empty() {
            return Ok(String::new());
        }

        let Some(key) = self.key else {
            return Ok(plaintext.to_string());
        };

        let cipher =
            Aes256Gcm::new_from_slice(&key).context("failed to build PatchHive secret cipher")?;
        let nonce = Nonce::generate();
        let ciphertext = cipher
            .encrypt(&nonce, plaintext.as_bytes())
            .map_err(|_| anyhow!("failed to encrypt PatchHive secret"))?;

        Ok(format!(
            "{ENCRYPTED_SECRET_PREFIX}{}:{}",
            hex::encode(nonce),
            hex::encode(ciphertext)
        ))
    }

    pub fn reveal_from_storage(&self, stored: &str) -> Result<String> {
        if stored.trim().is_empty() {
            return Ok(String::new());
        }

        if !Self::is_encrypted_value(stored) {
            return Ok(stored.to_string());
        }

        let Some(key) = self.key else {
            return Err(anyhow!(
                "a PatchHive encryption key is required to read encrypted stored secrets"
            ));
        };

        let payload = stored
            .trim()
            .strip_prefix(ENCRYPTED_SECRET_PREFIX)
            .ok_or_else(|| anyhow!("invalid encrypted PatchHive secret prefix"))?;
        let (nonce_hex, ciphertext_hex) = payload
            .split_once(':')
            .ok_or_else(|| anyhow!("invalid encrypted PatchHive secret payload"))?;
        let nonce_bytes =
            hex::decode(nonce_hex).context("failed to decode encrypted PatchHive secret nonce")?;
        if nonce_bytes.len() != 12 {
            return Err(anyhow!("invalid encrypted PatchHive secret nonce length"));
        }
        let ciphertext = hex::decode(ciphertext_hex)
            .context("failed to decode encrypted PatchHive secret ciphertext")?;

        let cipher =
            Aes256Gcm::new_from_slice(&key).context("failed to build PatchHive secret cipher")?;
        let nonce = Nonce::try_from(nonce_bytes.as_slice())
            .map_err(|_| anyhow!("invalid encrypted PatchHive secret nonce length"))?;
        let plaintext = cipher
            .decrypt(&nonce, ciphertext.as_ref())
            .map_err(|_| anyhow!("failed to decrypt PatchHive secret"))?;
        String::from_utf8(plaintext).context("decrypted PatchHive secret was not utf-8")
    }

    pub fn from_secret(secret: Option<&str>) -> Self {
        Self {
            key: secret.map(derive_key),
        }
    }
}

pub fn validate_encryption_secret(secret: &str) -> Result<()> {
    let secret = secret.trim();
    if secret.len() < 32 {
        return Err(anyhow!(
            "encryption keys must contain at least 32 characters of machine-random material"
        ));
    }

    let normalized = secret.to_ascii_lowercase();
    if [
        "replace-with",
        "change-me",
        "changeme",
        "password",
        "example",
    ]
    .iter()
    .any(|marker| normalized.contains(marker))
    {
        return Err(anyhow!(
            "encryption key looks like a placeholder or human password; generate one with `openssl rand -hex 32`"
        ));
    }

    Ok(())
}

pub fn generate_suite_bootstrap_secret() -> String {
    let bytes = <[u8; 32]>::generate();
    format!("ph-suite-{}", hex::encode(bytes))
}

pub fn validate_suite_bootstrap_secret(secret: &str) -> Result<()> {
    let secret = secret.trim();
    anyhow::ensure!(
        secret.len() >= 32,
        "suite bootstrap secrets must contain at least 32 characters of machine-random material"
    );
    anyhow::ensure!(
        !secret.contains(['\n', '\r'])
            && secret.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
            }),
        "suite bootstrap secrets contain unsupported characters"
    );
    let normalized = secret.to_ascii_lowercase();
    anyhow::ensure!(
        !normalized.contains("replace")
            && !normalized.contains("example")
            && !normalized.contains("your-")
            && !normalized.contains("xxxxxxxx"),
        "suite bootstrap secrets must not use placeholder material"
    );
    Ok(())
}

fn derive_key(secret: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(secret.as_bytes());
    hasher.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::{
        generate_suite_bootstrap_secret, validate_encryption_secret,
        validate_suite_bootstrap_secret, TokenProtector,
    };

    #[test]
    fn token_protector_round_trips_encrypted_values() {
        let protector = TokenProtector::from_secret(Some("test-secret"));
        let encrypted = protector
            .protect_for_storage("svc_signal")
            .expect("token should encrypt");
        assert!(TokenProtector::is_encrypted_value(&encrypted));
        assert_eq!(
            protector
                .reveal_from_storage(&encrypted)
                .expect("token should decrypt"),
            "svc_signal"
        );
    }

    #[test]
    fn generated_suite_bootstrap_secrets_are_valid_and_distinct() {
        let first = generate_suite_bootstrap_secret();
        let second = generate_suite_bootstrap_secret();
        validate_suite_bootstrap_secret(&first).unwrap();
        validate_suite_bootstrap_secret(&second).unwrap();
        assert_ne!(first, second);
    }

    #[test]
    fn suite_bootstrap_secrets_reject_short_or_placeholder_material() {
        assert!(validate_suite_bootstrap_secret("short").is_err());
        assert!(validate_suite_bootstrap_secret("replace-with-a-long-random-secret-now").is_err());
    }

    #[test]
    fn token_protector_uses_a_fresh_random_nonce() {
        let protector = TokenProtector::from_secret(Some("test-secret"));
        let first = protector
            .protect_for_storage("svc_signal")
            .expect("first encryption");
        let second = protector
            .protect_for_storage("svc_signal")
            .expect("second encryption");

        assert_ne!(first, second);
        assert_eq!(
            protector.reveal_from_storage(&first).unwrap(),
            protector.reveal_from_storage(&second).unwrap()
        );
    }

    #[test]
    fn encryption_secret_validation_rejects_short_and_placeholder_values() {
        assert!(validate_encryption_secret("short password").is_err());
        assert!(validate_encryption_secret("replace-with-a-long-random-secret-value").is_err());
        assert!(validate_encryption_secret(
            "24df1138bde289be6554bb1bd3ae475a789d59523d8039fd1110b25f25d1f153"
        )
        .is_ok());
    }

    #[test]
    fn token_protector_leaves_plaintext_when_unconfigured() {
        let protector = TokenProtector::default();
        assert_eq!(
            protector
                .protect_for_storage("svc_signal")
                .expect("plaintext should pass through"),
            "svc_signal"
        );
        assert_eq!(
            protector
                .reveal_from_storage("svc_signal")
                .expect("plaintext should load"),
            "svc_signal"
        );
    }

    #[test]
    fn token_protector_requires_key_to_decrypt_ciphertext() {
        let configured = TokenProtector::from_secret(Some("test-secret"));
        let encrypted = configured
            .protect_for_storage("svc_signal")
            .expect("token should encrypt");

        assert!(TokenProtector::default()
            .reveal_from_storage(&encrypted)
            .is_err());
    }

    #[test]
    fn token_protector_can_use_env_candidates() {
        std::env::remove_var("PATCHHIVE_TEST_SECRET_A");
        std::env::set_var("PATCHHIVE_TEST_SECRET_B", "candidate-secret");
        let protector = TokenProtector::from_env_candidates(&[
            "PATCHHIVE_TEST_SECRET_A",
            "PATCHHIVE_TEST_SECRET_B",
        ]);
        std::env::remove_var("PATCHHIVE_TEST_SECRET_B");
        assert!(protector.configured());
    }
}
