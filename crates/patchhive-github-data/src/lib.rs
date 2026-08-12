mod client;
pub mod discovery;
mod errors;
pub mod models;

pub use client::{
    code_search_count, code_search_rate_limit, decode_content, fetch_content_file, fetch_issues,
    fetch_pull_files, fetch_pull_request, fetch_pull_requests, fetch_pull_review_comments,
    fetch_pull_reviews, fetch_releases, fetch_repository, fetch_tags, fetch_workflow_jobs,
    fetch_workflow_runs, get_cursor_paginated_json, get_json, get_paginated_json, github_token,
    github_token_configured, github_token_required, request_headers, search_closed_issues,
    search_merged_pull_requests, search_repositories, valid_repo, validate_token, GH_API,
};
pub use errors::{
    classify_github_api_error, github_error_is_feature_disabled,
    github_error_is_permission_blocked, github_error_is_token_invalid,
    github_error_is_token_missing, github_message_from_body, response_preview, GitHubApiError,
    GitHubApiErrorKind,
};
