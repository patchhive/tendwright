use anyhow::{anyhow, Result};
pub use patchhive_github_data::{
    get_cursor_paginated_json, get_paginated_json, github_token, github_token_configured,
    github_token_required, valid_repo, validate_token,
};
use reqwest::Client;

use crate::models::{GitHubCodeScanningAlert, GitHubDependabotAlert};

fn validate_security_repo(repo: &str) -> Result<()> {
    if valid_repo(repo) {
        Ok(())
    } else {
        Err(anyhow!("Repository must be in owner/name format"))
    }
}

pub async fn fetch_code_scanning_alerts(
    client: &Client,
    repo: &str,
    limit: u32,
) -> Result<Vec<GitHubCodeScanningAlert>> {
    validate_security_repo(repo)?;

    let token = github_token();
    get_paginated_json(
        client,
        "patchhive-github-security/0.1",
        &format!("/repos/{repo}/code-scanning/alerts"),
        &[
            ("state", "open".into()),
            ("sort", "created".into()),
            ("direction", "desc".into()),
        ],
        token.as_deref(),
        limit.max(1) as usize,
    )
    .await
}

pub async fn fetch_dependabot_alerts(
    client: &Client,
    repo: &str,
    limit: u32,
) -> Result<Vec<GitHubDependabotAlert>> {
    validate_security_repo(repo)?;

    let token = github_token_required()?;
    get_cursor_paginated_json(
        client,
        "patchhive-github-security/0.1",
        &format!("/repos/{repo}/dependabot/alerts"),
        &[("state", "open".into())],
        Some(token.as_str()),
        limit.max(1) as usize,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::validate_security_repo;

    #[test]
    fn repository_validation_accepts_only_owner_name_pairs() {
        assert!(validate_security_repo("patchhive/tendwright").is_ok());

        for invalid in [
            "",
            "tendwright",
            "/tendwright",
            "patchhive/",
            "patchhive/tendwright/extra",
            "patch hive/tendwright",
        ] {
            let error = validate_security_repo(invalid).expect_err("repository must be rejected");
            assert_eq!(error.to_string(), "Repository must be in owner/name format");
        }
    }
}
