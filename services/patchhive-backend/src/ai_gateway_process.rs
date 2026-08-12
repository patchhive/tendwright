use std::{env, path::PathBuf, process::Stdio, time::Duration};

use anyhow::{bail, Context, Result};
use patchhive_product_core::ai_gateway::AiGatewayConfiguration;
use serde_json::Value;
use tokio::{process::Child, time::sleep};

const PATCHHIVE_AI_GATEWAY_ID: &str = "patchhive-ai-local";

pub struct LocalAiGatewayProcess {
    child: Option<Child>,
}

impl LocalAiGatewayProcess {
    pub async fn start() -> Result<Self> {
        if !autostart_enabled()? {
            tracing::info!(
                "PatchHive AI gateway autostart is disabled; configured products will use the external gateway as supplied"
            );
            return Ok(Self { child: None });
        }

        let Some(configuration) = AiGatewayConfiguration::from_environment()? else {
            tracing::info!(
                "PatchHive AI gateway is not configured; products preserve explicit not_observed AI evidence"
            );
            return Ok(Self { child: None });
        };
        if !configuration.is_loopback() {
            tracing::info!(
                base_url = %configuration.base_url,
                "PatchHive AI gateway is external and remains operator-managed"
            );
            return Ok(Self { child: None });
        }

        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(2))
            .timeout(Duration::from_secs(3))
            .build()
            .context("could not build PatchHive AI gateway readiness client")?;
        if gateway_ready(&http, &configuration).await {
            tracing::info!(
                base_url = %configuration.base_url,
                "PatchHive AI gateway is already running"
            );
            return Ok(Self { child: None });
        }

        let cli = ai_gateway_cli()?;
        let node = nonempty_env("PATCHHIVE_AI_NODE_PATH").unwrap_or_else(|| "node".into());
        let mut child = tokio::process::Command::new(&node)
            .arg(&cli)
            .current_dir(
                cli.parent()
                    .and_then(|src| src.parent())
                    .and_then(|package| package.parent())
                    .and_then(|packages| packages.parent())
                    .context("could not resolve PatchHive monorepo root from AI gateway CLI")?,
            )
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .with_context(|| {
                format!(
                    "could not start @patchhive/ai-local with {node}; set PATCHHIVE_AI_NODE_PATH when Node is not on PATH"
                )
            })?;

        for _ in 0..50 {
            if let Some(status) = child
                .try_wait()
                .context("could not inspect PatchHive AI gateway child process")?
            {
                bail!("@patchhive/ai-local exited before readiness with {status}");
            }
            if gateway_ready(&http, &configuration).await {
                tracing::info!(
                    base_url = %configuration.base_url,
                    "PatchHive AI gateway started with the suite backend"
                );
                return Ok(Self { child: Some(child) });
            }
            sleep(Duration::from_millis(100)).await;
        }

        let _ = child.kill().await;
        let _ = child.wait().await;
        bail!(
            "@patchhive/ai-local did not become ready at {} within 5 seconds",
            configuration.base_url
        )
    }

    pub async fn shutdown(&mut self) {
        let Some(child) = self.child.as_mut() else {
            return;
        };
        match child.try_wait() {
            Ok(Some(_)) => {}
            Ok(None) => {
                if let Err(error) = child.kill().await {
                    tracing::warn!(%error, "could not stop managed PatchHive AI gateway");
                }
                let _ = child.wait().await;
            }
            Err(error) => tracing::warn!(%error, "could not inspect managed PatchHive AI gateway"),
        }
        self.child = None;
        tracing::info!("managed PatchHive AI gateway stopped");
    }
}

async fn gateway_ready(http: &reqwest::Client, configuration: &AiGatewayConfiguration) -> bool {
    let request = configuration.apply_auth(http.get(configuration.health_url()));
    match request.send().await {
        Ok(response) if response.status().is_success() => response
            .json::<Value>()
            .await
            .ok()
            .is_some_and(|payload| gateway_identity_matches(&payload)),
        _ => false,
    }
}

fn gateway_identity_matches(payload: &Value) -> bool {
    payload.get("gateway").and_then(Value::as_str) == Some(PATCHHIVE_AI_GATEWAY_ID)
}

fn ai_gateway_cli() -> Result<PathBuf> {
    let current = env::current_dir().context("could not determine PatchHive working directory")?;
    let repo_root = patchhive_product_core::environment::find_repo_root(&current).context(
        "PATCHHIVE_AI_URL points at loopback, but the monorepo checkout was not found; run @patchhive/ai-local as a sidecar or set PATCHHIVE_AI_AUTOSTART=false",
    )?;
    let cli = repo_root.join("packages/ai-local/src/cli.js");
    if !cli.is_file() {
        bail!(
            "PATCHHIVE_AI_URL points at loopback, but {} is missing",
            cli.display()
        );
    }
    Ok(cli)
}

fn autostart_enabled() -> Result<bool> {
    let value = nonempty_env("PATCHHIVE_AI_AUTOSTART");
    parse_autostart(value.as_deref())
}

fn parse_autostart(value: Option<&str>) -> Result<bool> {
    let Some(value) = value else {
        return Ok(true);
    };
    match value.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => bail!("PATCHHIVE_AI_AUTOSTART must be true/false, yes/no, on/off, or 1/0"),
    }
}

fn nonempty_env(key: &str) -> Option<String> {
    env::var(key)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_autostart_is_enabled() {
        assert!(parse_autostart(None).expect("valid default"));
        assert!(parse_autostart(Some("true")).expect("valid true"));
        assert!(!parse_autostart(Some("off")).expect("valid false"));
    }

    #[test]
    fn invalid_autostart_is_rejected() {
        assert!(parse_autostart(Some("sometimes")).is_err());
    }

    #[test]
    fn readiness_uses_the_stable_gateway_identity_across_implementations() {
        for implementation in ["node", "rust"] {
            assert!(gateway_identity_matches(&serde_json::json!({
                "gateway": PATCHHIVE_AI_GATEWAY_ID,
                "gateway_implementation": implementation,
            })));
        }

        assert!(!gateway_identity_matches(&serde_json::json!({
            "gateway": "unrelated-local-service",
            "gateway_implementation": "rust",
        })));
    }
}
