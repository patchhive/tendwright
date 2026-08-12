use anyhow::{anyhow, Context, Result};
use patchhive_product_core::github_auth::github_read_token;
use reqwest::{
    header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION, LINK, USER_AGENT},
    Client, Response, StatusCode,
};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::Value;

use crate::models::{
    GitHubActionsWorkflowJob, GitHubActionsWorkflowRun, GitHubCodeSearchRateLimit,
    GitHubCodeSearchResponse, GitHubIssue, GitHubPullFile, GitHubPullRequest,
    GitHubRateLimitResponse, GitHubRepository, GitHubReview, GitHubReviewComment,
    GitHubSearchRepositoriesResponse,
};
use crate::{response_preview, GitHubApiError};

pub const GH_API: &str = "https://api.github.com";
const GITHUB_DATA_USER_AGENT: &str = "patchhive-github-data/0.1";
const TRANSIENT_RETRY_DELAYS_MS: [u64; 2] = [300, 900];
const REPOSITORY_FORMAT_ERROR: &str = "Repository must be in owner/name format";

fn is_transient_github_status(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::BAD_GATEWAY | StatusCode::SERVICE_UNAVAILABLE | StatusCode::GATEWAY_TIMEOUT
    )
}

async fn send_get_with_retry(
    client: &Client,
    user_agent: &str,
    path: &str,
    query: &[(&str, String)],
    token: Option<&str>,
) -> Result<Response> {
    let mut retry_index = 0usize;

    loop {
        let result = client
            .get(format!("{GH_API}{path}"))
            .headers(request_headers(user_agent, token)?)
            .query(query)
            .send()
            .await;

        match result {
            Ok(response)
                if is_transient_github_status(response.status())
                    && retry_index < TRANSIENT_RETRY_DELAYS_MS.len() =>
            {
                tokio::time::sleep(std::time::Duration::from_millis(
                    TRANSIENT_RETRY_DELAYS_MS[retry_index],
                ))
                .await;
                retry_index += 1;
            }
            Ok(response) => return Ok(response),
            Err(error)
                if (error.is_connect() || error.is_timeout())
                    && retry_index < TRANSIENT_RETRY_DELAYS_MS.len() =>
            {
                tokio::time::sleep(std::time::Duration::from_millis(
                    TRANSIENT_RETRY_DELAYS_MS[retry_index],
                ))
                .await;
                retry_index += 1;
            }
            Err(error) => {
                return Err(error).with_context(|| format!("GitHub request failed for {path}"));
            }
        }
    }
}

pub fn github_token() -> Option<String> {
    github_read_token()
}

pub fn github_token_required() -> Result<String> {
    github_token().ok_or_else(|| anyhow!("[missing_token]: PATCHHIVE_GITHUB_TOKEN_RO is not set"))
}

pub fn github_token_configured() -> bool {
    github_token().is_some()
}

pub fn request_headers(user_agent: &str, token: Option<&str>) -> Result<HeaderMap> {
    let mut headers = HeaderMap::new();
    headers.insert(USER_AGENT, HeaderValue::from_str(user_agent)?);
    headers.insert(
        "X-GitHub-Api-Version",
        HeaderValue::from_static("2022-11-28"),
    );
    headers.insert(
        ACCEPT,
        HeaderValue::from_static("application/vnd.github+json"),
    );
    if let Some(token) = token {
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {token}"))?,
        );
    }
    Ok(headers)
}

pub fn valid_repo(repo: &str) -> bool {
    let mut parts = repo.split('/');
    matches!(
        (parts.next(), parts.next(), parts.next()),
        (Some(owner), Some(name), None)
            if valid_repo_segment(owner) && valid_repo_segment(name)
    )
}

fn valid_repo_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment != "."
        && segment != ".."
        && segment.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
}

fn ensure_valid_repo(repo: &str) -> Result<()> {
    if valid_repo(repo) {
        Ok(())
    } else {
        Err(anyhow!(REPOSITORY_FORMAT_ERROR))
    }
}

pub async fn get_json<T: DeserializeOwned>(
    client: &Client,
    user_agent: &str,
    path: &str,
    query: &[(&str, String)],
    token: Option<&str>,
) -> Result<T> {
    let response = send_get_with_retry(client, user_agent, path, query, token).await?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(GitHubApiError::from_response("GET", path, status, &body).into());
    }

    let body = response
        .text()
        .await
        .with_context(|| format!("Could not read GitHub response body for {path}"))?;
    serde_json::from_str::<T>(&body).with_context(|| {
        format!(
            "Could not decode GitHub JSON for {path}. Response preview: {}",
            response_preview(&body)
        )
    })
}

pub async fn get_paginated_json<T: DeserializeOwned>(
    client: &Client,
    user_agent: &str,
    path: &str,
    query: &[(&str, String)],
    token: Option<&str>,
    max_items: usize,
) -> Result<Vec<T>> {
    if max_items == 0 {
        return Ok(Vec::new());
    }

    let mut page = 1usize;
    let mut items = Vec::new();

    loop {
        let remaining = max_items.saturating_sub(items.len());
        if remaining == 0 {
            break;
        }

        let mut page_query = query.to_vec();
        page_query.push(("per_page", remaining.min(100).to_string()));
        page_query.push(("page", page.to_string()));

        let mut page_items: Vec<T> = get_json(client, user_agent, path, &page_query, token).await?;
        let page_len = page_items.len();
        items.append(&mut page_items);

        if page_len < remaining.min(100) {
            break;
        }
        page += 1;
    }

    Ok(items)
}

pub async fn get_cursor_paginated_json<T: DeserializeOwned>(
    client: &Client,
    user_agent: &str,
    path: &str,
    query: &[(&str, String)],
    token: Option<&str>,
    max_items: usize,
) -> Result<Vec<T>> {
    if max_items == 0 {
        return Ok(Vec::new());
    }

    let mut after: Option<String> = None;
    let mut items = Vec::new();

    loop {
        let remaining = max_items.saturating_sub(items.len());
        if remaining == 0 {
            break;
        }

        let page_size = remaining.min(100);
        let mut page_query = query.to_vec();
        page_query.push(("per_page", page_size.to_string()));
        if let Some(cursor) = after.as_deref() {
            page_query.push(("after", cursor.to_string()));
        }

        let response = send_get_with_retry(client, user_agent, path, &page_query, token).await?;

        let status = response.status();
        let link_header = response
            .headers()
            .get(LINK)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(GitHubApiError::from_response("GET", path, status, &body).into());
        }

        let body = response
            .text()
            .await
            .with_context(|| format!("Could not read GitHub response body for {path}"))?;
        let mut page_items: Vec<T> = serde_json::from_str(&body).with_context(|| {
            format!(
                "Could not decode GitHub JSON for {path}. Response preview: {}",
                response_preview(&body)
            )
        })?;
        let page_len = page_items.len();
        items.append(&mut page_items);

        after = link_header.as_deref().and_then(next_after_cursor);
        if page_len < page_size || after.is_none() {
            break;
        }
    }

    Ok(items)
}

fn next_after_cursor(link_header: &str) -> Option<String> {
    let next_link = link_header
        .split(',')
        .map(str::trim)
        .find(|part| part.contains("rel=\"next\""))?;
    let start = next_link.find('<')? + 1;
    let end = next_link[start..].find('>')? + start;
    let url = &next_link[start..end];
    let query = url.split_once('?')?.1;
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key == "after").then(|| value.to_string())
    })
}

#[cfg(test)]
mod pagination_tests {
    use super::{is_transient_github_status, next_after_cursor};
    use reqwest::StatusCode;

    #[test]
    fn extracts_next_after_cursor_from_link_header() {
        let header = r#"<https://api.github.com/repos/o/r/dependabot/alerts?after=cursor-123&per_page=100>; rel="next", <https://api.github.com/repos/o/r/dependabot/alerts?before=cursor-456&per_page=100>; rel="prev""#;

        assert_eq!(next_after_cursor(header).as_deref(), Some("cursor-123"));
    }

    #[test]
    fn returns_none_without_next_link() {
        let header = r#"<https://api.github.com/repos/o/r/dependabot/alerts?before=cursor-456&per_page=100>; rel="prev""#;

        assert_eq!(next_after_cursor(header), None);
    }

    #[test]
    fn retries_only_transient_gateway_failures() {
        assert!(is_transient_github_status(StatusCode::BAD_GATEWAY));
        assert!(is_transient_github_status(StatusCode::SERVICE_UNAVAILABLE));
        assert!(is_transient_github_status(StatusCode::GATEWAY_TIMEOUT));
        assert!(!is_transient_github_status(StatusCode::FORBIDDEN));
        assert!(!is_transient_github_status(
            StatusCode::INTERNAL_SERVER_ERROR
        ));
    }
}

pub async fn get_paginated_field_json<T: DeserializeOwned>(
    client: &Client,
    user_agent: &str,
    path: &str,
    query: &[(&str, String)],
    token: Option<&str>,
    array_key: &str,
    max_items: usize,
) -> Result<Vec<T>> {
    if max_items == 0 {
        return Ok(Vec::new());
    }

    let mut page = 1usize;
    let mut items = Vec::new();

    loop {
        let remaining = max_items.saturating_sub(items.len());
        if remaining == 0 {
            break;
        }

        let mut page_query = query.to_vec();
        let page_size = remaining.min(100);
        page_query.push(("per_page", page_size.to_string()));
        page_query.push(("page", page.to_string()));

        let value: Value = get_json(client, user_agent, path, &page_query, token).await?;
        let page_items = value[array_key]
            .as_array()
            .ok_or_else(|| anyhow!("GitHub response field `{array_key}` was not an array"))?;

        let page_len = page_items.len();
        for item in page_items {
            items.push(serde_json::from_value::<T>(item.clone()).with_context(|| {
                format!("Could not decode GitHub JSON field `{array_key}` for {path}")
            })?);
        }

        if page_len < page_size {
            break;
        }
        page += 1;
    }

    Ok(items)
}

async fn get_public_json<T: DeserializeOwned>(
    client: &Client,
    path: &str,
    query: &[(&str, String)],
) -> Result<T> {
    let token = github_token();
    get_json(
        client,
        GITHUB_DATA_USER_AGENT,
        path,
        query,
        token.as_deref(),
    )
    .await
}

async fn get_authenticated_json<T: DeserializeOwned>(
    client: &Client,
    path: &str,
    query: &[(&str, String)],
) -> Result<T> {
    let token = github_token_required()?;
    get_json(
        client,
        GITHUB_DATA_USER_AGENT,
        path,
        query,
        Some(token.as_str()),
    )
    .await
}

pub async fn validate_token(client: &Client) -> Result<()> {
    let _: serde_json::Value = get_authenticated_json(client, "/rate_limit", &[]).await?;
    Ok(())
}

pub async fn fetch_repository(client: &Client, full_name: &str) -> Result<GitHubRepository> {
    ensure_valid_repo(full_name)?;

    get_public_json(client, &format!("/repos/{full_name}"), &[]).await
}

pub async fn search_repositories(
    client: &Client,
    query: &str,
    per_page: u32,
    sort: &str,
    order: &str,
) -> Result<GitHubSearchRepositoriesResponse> {
    let limit = per_page.clamp(1, 1000) as usize;
    let token = github_token();
    let items = get_paginated_field_json(
        client,
        GITHUB_DATA_USER_AGENT,
        "/search/repositories",
        &[
            ("q", query.trim().to_string()),
            ("sort", sort.trim().to_string()),
            ("order", order.trim().to_string()),
        ],
        token.as_deref(),
        "items",
        limit,
    )
    .await?;
    Ok(GitHubSearchRepositoriesResponse { items })
}

pub async fn fetch_issues(
    client: &Client,
    repo: &str,
    state: &str,
    sort: &str,
    direction: &str,
    per_page: u32,
) -> Result<Vec<GitHubIssue>> {
    ensure_valid_repo(repo)?;

    get_paginated_json(
        client,
        GITHUB_DATA_USER_AGENT,
        &format!("/repos/{repo}/issues"),
        &[
            ("state", state.trim().to_string()),
            ("sort", sort.trim().to_string()),
            ("direction", direction.trim().to_string()),
        ],
        github_token().as_deref(),
        per_page.max(1) as usize,
    )
    .await
}

pub async fn fetch_pull_requests(
    client: &Client,
    repo: &str,
    state: &str,
    sort: &str,
    direction: &str,
    per_page: u32,
) -> Result<Vec<GitHubPullRequest>> {
    ensure_valid_repo(repo)?;

    get_paginated_json(
        client,
        GITHUB_DATA_USER_AGENT,
        &format!("/repos/{repo}/pulls"),
        &[
            ("state", state.trim().to_string()),
            ("sort", sort.trim().to_string()),
            ("direction", direction.trim().to_string()),
        ],
        github_token().as_deref(),
        per_page.clamp(1, 1000) as usize,
    )
    .await
}

/// Fetch one pull request by its stable repository and number.
pub async fn fetch_pull_request(
    client: &Client,
    repo: &str,
    number: u32,
) -> Result<GitHubPullRequest> {
    ensure_valid_repo(repo)?;
    if number == 0 {
        return Err(anyhow!("Pull request number must be greater than zero"));
    }

    get_json(
        client,
        GITHUB_DATA_USER_AGENT,
        &format!("/repos/{repo}/pulls/{number}"),
        &[],
        github_token().as_deref(),
    )
    .await
}

#[derive(Debug, Deserialize)]
struct GitHubSearchIssuesResponse {
    #[serde(default)]
    items: Vec<GitHubIssue>,
}

fn merged_pull_from_search_issue(issue: GitHubIssue) -> Option<GitHubPullRequest> {
    let merged_at = issue
        .pull_request
        .as_ref()?
        .get("merged_at")?
        .as_str()?
        .to_string();

    Some(GitHubPullRequest {
        number: issue.number,
        title: issue.title,
        html_url: issue.html_url,
        body: issue.body,
        merged_at: Some(merged_at),
        updated_at: issue.updated_at,
        user: issue.user,
        ..GitHubPullRequest::default()
    })
}

pub async fn search_merged_pull_requests(
    client: &Client,
    repo: &str,
    merged_since: &str,
    max_items: u32,
) -> Result<Vec<GitHubPullRequest>> {
    ensure_valid_repo(repo)?;

    let response: GitHubSearchIssuesResponse = get_json(
        client,
        GITHUB_DATA_USER_AGENT,
        "/search/issues",
        &[
            (
                "q",
                format!("repo:{repo} is:pr is:merged merged:>={merged_since}"),
            ),
            ("sort", "updated".to_string()),
            ("order", "desc".to_string()),
            ("per_page", max_items.clamp(1, 100).to_string()),
        ],
        github_token().as_deref(),
    )
    .await?;

    Ok(response
        .items
        .into_iter()
        .filter_map(merged_pull_from_search_issue)
        .take(max_items as usize)
        .collect())
}

pub async fn search_closed_issues(
    client: &Client,
    repo: &str,
    closed_since: &str,
    max_items: u32,
) -> Result<Vec<GitHubIssue>> {
    ensure_valid_repo(repo)?;

    let response: GitHubSearchIssuesResponse = get_json(
        client,
        GITHUB_DATA_USER_AGENT,
        "/search/issues",
        &[
            (
                "q",
                format!("repo:{repo} is:issue is:closed closed:>={closed_since}"),
            ),
            ("sort", "updated".to_string()),
            ("order", "desc".to_string()),
            ("per_page", max_items.clamp(1, 100).to_string()),
        ],
        github_token().as_deref(),
    )
    .await?;

    Ok(response
        .items
        .into_iter()
        .filter(|issue| issue.pull_request.is_none())
        .take(max_items as usize)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_repo_accepts_owner_name_format() {
        assert!(valid_repo("patchhive/repo.reaper_1"));
    }

    #[test]
    fn valid_repo_rejects_extra_or_missing_segments() {
        assert!(!valid_repo("patchhive/repo/extra"));
        assert!(!valid_repo("patchhive/"));
        assert!(!valid_repo("/repo"));
        assert!(!valid_repo("patch hive/repo"));
        assert!(!valid_repo("patchhive/repo?tab=actions"));
        assert!(!valid_repo("patchhive/.."));
    }

    #[test]
    fn ensure_valid_repo_preserves_the_public_error_contract() {
        let error = ensure_valid_repo("patchhive/repo/extra")
            .expect_err("invalid repository should be rejected");

        assert_eq!(error.to_string(), REPOSITORY_FORMAT_ERROR);
        ensure_valid_repo("patchhive/repo").expect("valid repository should be accepted");
    }

    #[test]
    fn request_headers_adds_bearer_token_when_present() {
        let headers =
            request_headers("patchhive-test", Some("secret-token")).expect("headers should build");

        assert_eq!(
            headers
                .get(AUTHORIZATION)
                .and_then(|value| value.to_str().ok()),
            Some("Bearer secret-token")
        );
        assert_eq!(
            headers
                .get(USER_AGENT)
                .and_then(|value| value.to_str().ok()),
            Some("patchhive-test")
        );
    }

    #[test]
    fn merged_search_result_maps_to_pull_request() {
        let issue = GitHubIssue {
            number: 42,
            title: "Merged work".into(),
            html_url: "https://github.com/patchhive/example/pull/42".into(),
            pull_request: Some(serde_json::json!({
                "merged_at": "2026-07-12T11:33:00Z"
            })),
            ..GitHubIssue::default()
        };

        let pull = merged_pull_from_search_issue(issue).expect("merged result should map");

        assert_eq!(pull.number, 42);
        assert_eq!(pull.merged_at.as_deref(), Some("2026-07-12T11:33:00Z"));
    }
}

pub async fn fetch_pull_reviews(
    client: &Client,
    repo: &str,
    number: u32,
) -> Result<Vec<GitHubReview>> {
    ensure_valid_repo(repo)?;

    get_paginated_json(
        client,
        GITHUB_DATA_USER_AGENT,
        &format!("/repos/{repo}/pulls/{number}/reviews"),
        &[],
        github_token().as_deref(),
        1000,
    )
    .await
}

pub async fn fetch_pull_review_comments(
    client: &Client,
    repo: &str,
    number: u32,
) -> Result<Vec<GitHubReviewComment>> {
    ensure_valid_repo(repo)?;

    get_paginated_json(
        client,
        GITHUB_DATA_USER_AGENT,
        &format!("/repos/{repo}/pulls/{number}/comments"),
        &[],
        github_token().as_deref(),
        1000,
    )
    .await
}

pub async fn fetch_pull_files(
    client: &Client,
    repo: &str,
    number: u32,
) -> Result<Vec<GitHubPullFile>> {
    ensure_valid_repo(repo)?;

    get_paginated_json(
        client,
        GITHUB_DATA_USER_AGENT,
        &format!("/repos/{repo}/pulls/{number}/files"),
        &[],
        github_token().as_deref(),
        1000,
    )
    .await
}

pub async fn code_search_count(client: &Client, query: &str) -> Result<u32> {
    let response: GitHubCodeSearchResponse = get_public_json(
        client,
        "/search/code",
        &[("q", query.trim().to_string()), ("per_page", "1".into())],
    )
    .await?;
    Ok(response.total_count)
}

pub async fn code_search_rate_limit(client: &Client) -> Result<GitHubCodeSearchRateLimit> {
    let response: GitHubRateLimitResponse = get_public_json(client, "/rate_limit", &[]).await?;
    Ok(response.resources.code_search)
}

pub async fn fetch_workflow_runs(
    client: &Client,
    repo: &str,
    branch: Option<&str>,
    limit: u32,
) -> Result<Vec<GitHubActionsWorkflowRun>> {
    ensure_valid_repo(repo)?;

    let mut query = vec![("exclude_pull_requests", "false".into())];

    if let Some(branch) = branch.filter(|value| !value.trim().is_empty()) {
        query.push(("branch", branch.trim().to_string()));
    }

    get_paginated_field_json(
        client,
        GITHUB_DATA_USER_AGENT,
        &format!("/repos/{repo}/actions/runs"),
        &query,
        github_token().as_deref(),
        "workflow_runs",
        limit.max(1) as usize,
    )
    .await
}

pub async fn fetch_workflow_jobs(
    client: &Client,
    repo: &str,
    run_id: i64,
) -> Result<Vec<GitHubActionsWorkflowJob>> {
    ensure_valid_repo(repo)?;

    get_paginated_field_json(
        client,
        GITHUB_DATA_USER_AGENT,
        &format!("/repos/{repo}/actions/runs/{run_id}/jobs"),
        &[],
        github_token().as_deref(),
        "jobs",
        1000,
    )
    .await
}
