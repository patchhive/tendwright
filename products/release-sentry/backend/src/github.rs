use anyhow::Result;
use patchhive_github_data::{
    fetch_issues as fetch_shared_issues, fetch_workflow_runs as fetch_shared_workflow_runs,
};
use reqwest::Client;

pub use patchhive_github_data::models::{
    GitHubActionsWorkflowRun, GitHubIssue, GitHubRelease,
    GitHubRepository as GitHubRepositoryDetail, GitHubTag,
};
pub use patchhive_github_data::{
    decode_content, fetch_content_file as fetch_content_text, fetch_releases, fetch_repository,
    fetch_tags,
};

pub async fn fetch_workflow_runs(
    client: &Client,
    repo: &str,
    branch: Option<&str>,
    limit: u32,
) -> Result<Vec<GitHubActionsWorkflowRun>> {
    fetch_shared_workflow_runs(client, repo, branch, limit).await
}

pub async fn fetch_open_issues(
    client: &Client,
    repo: &str,
    limit: u32,
) -> Result<Vec<GitHubIssue>> {
    fetch_shared_issues(client, repo, "open", "updated", "desc", limit).await
}

pub use patchhive_github_data::{github_token_configured, validate_token};
