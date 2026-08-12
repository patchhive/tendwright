use anyhow::{anyhow, Result};
use reqwest::RequestBuilder;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AiGatewayConfiguration {
    pub base_url: String,
    auth: AiGatewayAuth,
}

#[derive(Clone, Eq, PartialEq)]
enum AiGatewayAuth {
    Local(String),
    Remote(String),
}

impl std::fmt::Debug for AiGatewayAuth {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Local(_) => "Local(<redacted>)",
            Self::Remote(_) => "Remote(<redacted>)",
        })
    }
}

impl AiGatewayConfiguration {
    pub fn from_environment() -> Result<Option<Self>> {
        let Some(base_url) = nonempty_env("PATCHHIVE_AI_URL") else {
            return Ok(None);
        };
        Self::new(
            base_url,
            nonempty_env("PATCHHIVE_AI_GATEWAY_API_KEY"),
            nonempty_env("PATCHHIVE_AI_API_KEY"),
        )
        .map(Some)
    }

    pub fn new(
        base_url: impl Into<String>,
        local_key: Option<String>,
        remote_key: Option<String>,
    ) -> Result<Self> {
        let base_url = base_url.into().trim().trim_end_matches('/').to_owned();
        if base_url.is_empty() {
            return Err(anyhow!("AI gateway base URL must not be empty"));
        }
        let url = reqwest::Url::parse(&base_url)
            .map_err(|error| anyhow!("PATCHHIVE_AI_URL is invalid: {error}"))?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(anyhow!("PATCHHIVE_AI_URL must use http or https"));
        }

        let auth = if let Some(key) = local_key
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
        {
            // The PatchHive gateway often has a non-loopback service hostname
            // inside Docker. An explicitly supplied gateway credential is the
            // authority signal; host spelling must not turn it into a platform key.
            AiGatewayAuth::Local(key)
        } else if is_loopback_url(&url) {
            return Err(anyhow!(
                "PATCHHIVE_AI_GATEWAY_API_KEY is required for the local AI gateway"
            ));
        } else {
            AiGatewayAuth::Remote(required_secret(
                remote_key,
                "PATCHHIVE_AI_API_KEY is required for a non-local AI gateway",
            )?)
        };
        Ok(Self { base_url, auth })
    }

    pub fn chat_completions_url(&self) -> String {
        format!("{}/chat/completions", self.base_url)
    }

    pub fn health_url(&self) -> String {
        let root = self.base_url.strip_suffix("/v1").unwrap_or(&self.base_url);
        format!("{root}/health")
    }

    pub fn is_loopback(&self) -> bool {
        reqwest::Url::parse(&self.base_url).is_ok_and(|url| is_loopback_url(&url))
    }

    pub fn apply_auth(&self, request: RequestBuilder) -> RequestBuilder {
        match &self.auth {
            AiGatewayAuth::Local(key) | AiGatewayAuth::Remote(key) => request.bearer_auth(key),
        }
    }
}

fn nonempty_env(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn required_secret(value: Option<String>, message: &str) -> Result<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!(message.to_owned()))
}

fn is_loopback_url(url: &reqwest::Url) -> bool {
    url.host_str()
        .map(str::to_ascii_lowercase)
        .is_some_and(|host| {
            host == "localhost"
                || host
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|address| address.is_loopback())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_gateway_requires_its_scoped_key() {
        let error = AiGatewayConfiguration::new(
            "http://127.0.0.1:8787/v1/",
            None,
            Some("platform-key".into()),
        )
        .expect_err("local gateway must not accept a platform key");
        assert!(error.to_string().contains("PATCHHIVE_AI_GATEWAY_API_KEY"));
    }

    #[test]
    fn remote_gateway_requires_a_platform_key() {
        let error = AiGatewayConfiguration::new("https://example.com/v1", None, None)
            .expect_err("remote gateway must not accept a local key");
        assert!(error.to_string().contains("PATCHHIVE_AI_API_KEY"));
    }

    #[test]
    fn docker_gateway_hostname_uses_the_explicit_gateway_key() {
        let configuration = AiGatewayConfiguration::new(
            "http://patchhive-ai-local:8787/v1",
            Some("gateway-secret".into()),
            None,
        )
        .expect("Docker gateway configuration should be valid");
        assert_eq!(
            configuration.chat_completions_url(),
            "http://patchhive-ai-local:8787/v1/chat/completions"
        );
    }

    #[test]
    fn credentials_are_redacted_and_url_is_normalized() {
        let configuration = AiGatewayConfiguration::new(
            "http://localhost:8787/v1/",
            Some("gateway-secret".into()),
            None,
        )
        .expect("valid configuration");
        let rendered = format!("{configuration:?}");
        assert!(!rendered.contains("gateway-secret"));
        assert_eq!(
            configuration.chat_completions_url(),
            "http://localhost:8787/v1/chat/completions"
        );
        assert_eq!(configuration.health_url(), "http://localhost:8787/health");
        assert!(configuration.is_loopback());
    }
}
