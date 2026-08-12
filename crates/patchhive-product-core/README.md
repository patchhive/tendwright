# patchhive-product-core

`patchhive-product-core` is the shared Rust foundation for PatchHive backends.

It holds the backend behavior that already repeats across products and should stay consistent everywhere: auth bootstrap, auth verification, SQLite pooling, startup checks, and cross-product service primitives that make the suite easier to run independently today and easier to orchestrate later.

## Current Scope

- API-key hashing, verification, persistence, and middleware
- shared auth bootstrap behavior and error shapes
- auth module generation with `define_api_key_auth_module!`
- standard specialist auth, health, startup, capabilities, run, overview, and history routing
- shared SQLite connection pooling with `SqlitePool`
- typed startup checks and shared startup logging helpers
- shared cross-product client primitives such as RepoMemory context access

## Design Boundary

`patchhive-product-core` is for real backend seams that are already shared across multiple products.

It is not the place for:

- product-specific GitHub logic
- product scoring or policy heuristics
- product route behavior that only one product owns

## Repository Model

The PatchHive monorepo is the source of truth for this crate. The standalone `patchhive/patchhive-product-core` repository is an exported mirror used by product repos and standalone CI.
