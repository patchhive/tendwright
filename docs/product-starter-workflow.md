# Product Starter Workflow

PatchHive has a canonical product starter so new specialist products do not
begin with a manual copy of an existing app.

The starter lives at `templates/product-starter/`, and the actual copied scaffold lives under `templates/product-starter/scaffold/`.

## Why It Exists

By the time PatchHive had RepoReaper, SignalHive, and TrustGate, the same shell was already repeating:

- Rust backend auth wiring
- startup checks
- SQLite path setup
- canonical specialist React shell
- API-key auth bootstrap
- frontend checks panel
- Docker files
- standalone CI

The starter keeps that repeated shape in one place so new products begin from a consistent shell and diverge only where the product logic actually changes.

## Create A New Product

Use:

```bash
./scripts/new-product.sh <product-slug>
```

Example:

```bash
./scripts/new-product.sh review-bee --icon "🐝" --tagline "Turn review churn into concrete follow-up work."
```

The script will:

1. copy the shared starter template into `products/<product-slug>`
2. pick the next available backend and frontend ports unless you override them
3. wire in shared PatchHive auth, specialist UI, and CI
4. generate a mountable backend engine plus thin standalone launcher
5. resolve shared package paths for the generated monorepo location
6. refresh the containing Cargo workspace lockfile unless you pass `--skip-lockfile`

Inside Tendwright, the generated backend joins the root Cargo workspace through
the `products/*/backend` member glob and uses the root `Cargo.lock`. A scaffold
created outside that workspace receives its own backend lockfile.

Useful flags:

```bash
./scripts/new-product.sh repo-memory \
  --icon "🧠" \
  --backend-port 8040 \
  --frontend-port 5177 \
  --tagline "Give coding agents memory of how your repo actually works."
```

## What The Starter Includes

- `backend/` with a mountable router, shared auth, startup checks, SQLite pool,
  rate limiting, and placeholder overview route
- `frontend/` with the canonical specialist shell, API-key login, workspace,
  checks, and sources surfaces
- `.env.example`
- `.gitignore`
- `docker-compose.yml`
- backend and frontend Dockerfiles
- standalone GitHub Actions CI
- starter README copy

## After Scaffolding

Do these early:

1. Replace all starter copy and placeholder routes with the real product loop.
2. Adjust startup checks so they reflect the product's real dependencies.
3. Add the product brand and accent tokens to `packages/ui/`.
4. Add a product manifest under
   `services/patchhive-backend/registry/products/` and mount the engine.
5. Add the product to `scripts/suite-common.sh`, root `.env.example`, and the
   canonical docs.
6. Run `./scripts/check-suite-drift.sh` before committing the scaffold.

## Standalone Lockfile Helper

Exported Rust products carry a snapshot of PatchHive's shared crates, so their
standalone lockfiles must be generated against that exported layout rather than
the monorepo paths.

Use:

```bash
./scripts/refresh-product-lockfile.sh <product-slug>
```

Example:

```bash
./scripts/refresh-product-lockfile.sh trust-gate
```

This copies the product and current shared crates to a temporary standalone
layout, rewrites PatchHive-owned dependencies to that snapshot, and verifies
that a standalone `backend/Cargo.lock` can be generated. Pass
`--output <path>` only when another release tool needs the generated artifact.

Use it:

- before the first standalone export
- after shared crate dependency changes
- any time standalone CI says `cargo check --locked` wants to update the lockfile

`export-product.sh` runs this validation automatically for Rust-backed products
and commits the generated lockfile only to the export branch, so the helper is
mostly useful when you want to preflight without exporting.

## Standalone Template Repo

If you want the starter itself to have its own GitHub repo mirror, use:

```bash
./scripts/export-template.sh product-starter <remote-name> main
```

If you need to refresh the template scaffold lockfile directly, use:

```bash
./scripts/refresh-template-lockfile.sh product-starter
```

For PatchHive, `patchhive-product-starter` should still be treated as a mirror of `templates/product-starter`, not as the primary editing location.
