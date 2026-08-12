# patchhive-github-data

`patchhive-github-data` is the shared Rust GitHub data client for PatchHive products.

It owns the read-heavy GitHub access patterns that recur across visibility and memory products such as SignalHive, RepoMemory, and FlakeSting.

## Current Scope

- PatchHive-standard GitHub token and env resolution
- repository fetch and repository search
- release, tag, and safely encoded repository-content reads
- issue history and merged pull request history reads
- review, review comment, and file reads for historical ingestion
- code search count reads
- GitHub Actions workflow run and job reads

## Design Boundary

This crate is for typed GitHub data access, not product interpretation.

It does not own:

- PR webhook verification or PR comment and check publishing
- scoring heuristics
- policy logic
- product-specific route behavior

That keeps the boundary clean beside `patchhive-github-pr`, which owns the pull request lifecycle plumbing.
