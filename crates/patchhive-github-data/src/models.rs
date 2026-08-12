use serde::{Deserialize, Deserializer, Serialize};

fn default_on_null<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GitHubUser {
    #[serde(default)]
    pub login: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GitHubLabel {
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GitHubRepositoryOwner {
    #[serde(default)]
    pub login: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GitHubRepository {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub full_name: String,
    #[serde(default)]
    pub html_url: String,
    pub description: Option<String>,
    pub language: Option<String>,
    #[serde(default)]
    pub stargazers_count: u32,
    #[serde(default)]
    pub open_issues_count: u32,
    #[serde(default)]
    pub default_branch: String,
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub disabled: bool,
    pub pushed_at: Option<String>,
    #[serde(default)]
    pub owner: GitHubRepositoryOwner,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GitHubRelease {
    #[serde(default)]
    pub tag_name: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub html_url: String,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub prerelease: bool,
    pub published_at: Option<String>,
    #[serde(default)]
    pub target_commitish: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GitHubTagCommit {
    #[serde(default)]
    pub sha: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GitHubTag {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub zipball_url: String,
    #[serde(default)]
    pub tarball_url: String,
    #[serde(default)]
    pub commit: GitHubTagCommit,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GitHubContentFile {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub html_url: String,
    #[serde(default)]
    pub download_url: Option<String>,
    #[serde(default)]
    pub encoding: String,
    #[serde(default)]
    pub content: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GitHubSearchRepositoriesResponse {
    #[serde(default)]
    pub items: Vec<GitHubRepository>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GitHubIssue {
    #[serde(default)]
    pub number: u32,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub html_url: String,
    pub body: Option<String>,
    pub closed_at: Option<String>,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub comments: u32,
    #[serde(default)]
    pub labels: Vec<GitHubLabel>,
    pub user: Option<GitHubUser>,
    pub pull_request: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GitHubPullRequest {
    #[serde(default)]
    pub number: u32,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub html_url: String,
    #[serde(default)]
    pub state: String,
    pub body: Option<String>,
    pub merged_at: Option<String>,
    #[serde(default)]
    pub updated_at: String,
    pub additions: Option<u32>,
    pub deletions: Option<u32>,
    pub changed_files: Option<u32>,
    pub user: Option<GitHubUser>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GitHubReview {
    pub body: Option<String>,
    pub html_url: Option<String>,
    pub submitted_at: Option<String>,
    #[serde(default)]
    pub state: String,
    pub user: Option<GitHubUser>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GitHubReviewComment {
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub html_url: String,
    pub path: Option<String>,
    #[serde(default)]
    pub created_at: String,
    pub user: Option<GitHubUser>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GitHubPullFile {
    #[serde(default)]
    pub filename: String,
    #[serde(default)]
    pub additions: u32,
    #[serde(default)]
    pub deletions: u32,
    #[serde(default)]
    pub status: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GitHubCodeSearchResponse {
    #[serde(default)]
    pub total_count: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GitHubCodeSearchRateLimit {
    #[serde(default)]
    pub limit: u32,
    #[serde(default)]
    pub remaining: u32,
    #[serde(default)]
    pub reset: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct GitHubRateLimitResources {
    #[serde(default)]
    pub code_search: GitHubCodeSearchRateLimit,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct GitHubRateLimitResponse {
    #[serde(default)]
    pub resources: GitHubRateLimitResources,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GitHubActionsWorkflowRun {
    #[serde(default)]
    pub id: i64,
    #[serde(default, deserialize_with = "default_on_null")]
    pub name: String,
    #[serde(default, deserialize_with = "default_on_null")]
    pub html_url: String,
    #[serde(default, deserialize_with = "default_on_null")]
    pub head_branch: String,
    #[serde(default, deserialize_with = "default_on_null")]
    pub conclusion: String,
    #[serde(
        default = "default_run_attempt",
        deserialize_with = "run_attempt_on_null"
    )]
    pub run_attempt: u32,
    #[serde(default)]
    pub run_number: u32,
}

fn default_run_attempt() -> u32 {
    1
}

fn run_attempt_on_null<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(Option::<u32>::deserialize(deserializer)?.unwrap_or_else(default_run_attempt))
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GitHubActionsWorkflowStep {
    #[serde(default, deserialize_with = "default_on_null")]
    pub name: String,
    #[serde(default, deserialize_with = "default_on_null")]
    pub conclusion: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GitHubActionsWorkflowJob {
    #[serde(default, deserialize_with = "default_on_null")]
    pub name: String,
    #[serde(default, deserialize_with = "default_on_null")]
    pub conclusion: String,
    #[serde(default, deserialize_with = "default_on_null")]
    pub html_url: String,
    #[serde(default, deserialize_with = "default_on_null")]
    pub runner_name: String,
    #[serde(default, deserialize_with = "default_on_null")]
    pub labels: Vec<String>,
    #[serde(default, deserialize_with = "default_on_null")]
    pub steps: Vec<GitHubActionsWorkflowStep>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GitHubActionsWorkflowRunsResponse {
    #[serde(default)]
    pub workflow_runs: Vec<GitHubActionsWorkflowRun>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GitHubActionsWorkflowJobsResponse {
    #[serde(default)]
    pub jobs: Vec<GitHubActionsWorkflowJob>,
}

#[cfg(test)]
mod tests {
    use super::{
        GitHubActionsWorkflowJob, GitHubActionsWorkflowRun, GitHubActionsWorkflowStep,
        GitHubRateLimitResponse,
    };

    #[test]
    fn rate_limit_response_exposes_code_search_capacity() {
        let response: GitHubRateLimitResponse = serde_json::from_str(
            r#"{
              "resources": {
                "core": {"limit": 5000, "remaining": 4999, "reset": 1000},
                "code_search": {"limit": 10, "remaining": 3, "reset": 1060}
              }
            }"#,
        )
        .expect("rate-limit response should decode");

        assert_eq!(response.resources.code_search.limit, 10);
        assert_eq!(response.resources.code_search.remaining, 3);
        assert_eq!(response.resources.code_search.reset, 1060);
    }

    #[test]
    fn workflow_run_allows_nullable_github_actions_fields() {
        let run: GitHubActionsWorkflowRun = serde_json::from_str(
            r#"{
              "id": 42,
              "name": null,
              "html_url": null,
              "head_branch": null,
              "conclusion": null,
              "run_attempt": null,
              "run_number": 7
            }"#,
        )
        .expect("workflow run should decode");

        assert_eq!(run.id, 42);
        assert_eq!(run.name, "");
        assert_eq!(run.conclusion, "");
        assert_eq!(run.run_attempt, 1);
        assert_eq!(run.run_number, 7);
    }

    #[test]
    fn workflow_job_allows_nullable_steps_and_labels() {
        let job: GitHubActionsWorkflowJob = serde_json::from_str(
            r#"{
              "name": null,
              "conclusion": null,
              "html_url": null,
              "runner_name": null,
              "labels": null,
              "steps": null
            }"#,
        )
        .expect("workflow job should decode");

        assert_eq!(job.name, "");
        assert!(job.labels.is_empty());
        assert!(job.steps.is_empty());

        let step: GitHubActionsWorkflowStep =
            serde_json::from_str(r#"{"name": null, "conclusion": null}"#)
                .expect("workflow step should decode");
        assert_eq!(step.name, "");
        assert_eq!(step.conclusion, "");
    }
}
