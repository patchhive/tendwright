# Product, Package, And Crate Export Workflow

PatchHive uses the monorepo as the source of truth.

Standalone product repositories are exported from the monorepo when a product is ready for its own GitHub presence.

Shared packages can be exported the same way when you want them to have their own GitHub identity.

Shared Rust crates can be exported the same way when multiple standalone product backends need to consume them.

Shared templates can be exported the same way when you want starter scaffolds to have their own GitHub identity.

Shared services can be exported the same way when you want standalone service repositories for visibility, releases, or Docker image build context.

## Principles

- Develop products in the monorepo first.
- Develop shared packages and crates in the monorepo first.
- Treat standalone product repositories as exported mirrors, not the primary development home.
- Treat standalone package repositories as exported mirrors, not the primary development home.
- Treat standalone crate repositories as exported mirrors, not the primary development home.
- Treat standalone template repositories as exported mirrors, not the primary development home.
- Treat standalone service repositories as exported mirrors, not the primary development home.
- Re-export products, packages, crates, templates, and services from the monorepo instead of manually copying files around.

## Shared Packages

Product exports do not carry `packages/` with them.

That is intentional.

Standalone product repositories should:

- depend on published shared packages such as `@patchhivehq/ui`
- depend on published shared packages such as `@patchhivehq/product-shell`
- use shared service contracts for things like `PATCHHIVE_AI_URL`
- avoid local `file:` dependencies back into the monorepo

`@patchhivehq/ui` publishes to the public npm registry so standalone products can install the canonical shared interface without package-registry authentication.
`@patchhivehq/product-shell` follows the same pattern.

That means:

- standalone product repositories can depend on normal semver releases
- outside contributors can run `npm install` without GitHub package tokens
- PatchHive only needs npm publishing credentials during release, not during consumer installs

Specialist frontends use local `file:` dependencies while they live inside the
monorepo. `export-product.sh` rewrites those dependencies to the current
published package versions, regenerates the standalone lockfile, and replaces
the monorepo-context frontend Docker build with a standalone build context.

## Shared Rust Crates

Standalone product repositories should not depend on monorepo-local crate paths.

That is intentional too.

Standalone Rust product repositories should:

- carry the exact shared Rust crate snapshot used to build the export under
  `shared-crates/`
- avoid `path = "../../../crates/..."` dependencies that only work inside the monorepo
- treat that snapshot as generated export material rather than an independent
  source of truth

`export-product.sh` copies `patchhive-product-core`, `patchhive-github-pr`,
`patchhive-github-data`, and `patchhive-github-security` from the current
monorepo revision, rewrites the exported backend to those paths, and generates
the matching lockfile. This keeps a product mirror reproducible even when a
standalone shared-crate mirror has not been synchronized yet. Shared-crate
repositories remain useful package-focused mirrors, but product correctness no
longer depends on their release timing.

## Export Script

Use:

```bash
./scripts/export-product.sh <product-name>
```

Example:

```bash
./scripts/export-product.sh repo-reaper
```

This creates a local export branch from `products/repo-reaper`.

If you want to push directly to a standalone remote:

```bash
./scripts/export-product.sh repo-reaper repo-reaper main
```

That will:

1. create a subtree export branch
2. push that branch to the `repo-reaper` remote's `main` branch

The script is intentionally safe and portable:

- it requires a clean working tree and exports committed `HEAD` exactly
- it does not overwrite an existing export branch
- if `export/<product>` already exists, it creates a timestamped branch name instead
- if the product has a Rust backend, it refreshes the standalone-safe `backend/Cargo.lock` before exporting
- it snapshots current shared crates and rewrites monorepo-only paths to that
  standalone snapshot
- it rewrites local frontend package paths and Docker context for the mirror
- it resolves and commits a standalone `package-lock.json`, then uses `npm ci`
  in CI and Docker builds
- it uses repository-root Docker contexts, explicit Dockerfile paths, locked
  Cargo builds, non-root runtime users, and digest-pinned generated base images

The shared package versions referenced by the product must already exist on
npm. The suite release runner publishes and waits for them before product
export. A direct export fails before creating a portable commit if npm cannot
resolve the exact package version; it never deletes the lockfile and pretends
the export is ready.

For standalone product repositories that are treated as mirrors, you can opt into a guarded mirror update:

```bash
PATCHHIVE_EXPORT_FORCE_WITH_LEASE=1 ./scripts/export-product.sh hive-core hivecore main
```

That uses the remote branch's current SHA as a `--force-with-lease` expectation. It is meant for mirror repos that may contain generated-only standalone commits, not for repositories with independent source-of-truth work.

The suite release runner uses this guarded mirror mode by default:

```bash
./scripts/release-suite.sh --products hive-core --skip-publish
```

## Package Export Script

Use:

```bash
./scripts/export-package.sh <package-name>
```

Example:

```bash
./scripts/export-package.sh ui
```

If you want to push directly to a standalone package remote:

```bash
./scripts/export-package.sh ui patchhive-ui main
```

That creates a subtree export branch from `packages/ui` and can push it directly into a standalone package repository.

## Package Mirror Sync Script

For shared package repositories, PatchHive prefers clean package-only mirror history over raw subtree history.

After the first export, use:

```bash
./scripts/sync-package-mirror.sh ui patchhive-ui main
```

That creates one package-focused sync commit in the standalone mirror repository instead of replaying mixed monorepo commit messages.

If you want to reset an existing package mirror onto the clean sync history model, use:

```bash
./scripts/sync-package-mirror.sh ui patchhive-ui main --reset-history
```

That force-pushes a fresh root commit into the standalone package repository.

## Crate Export Script

Use:

```bash
./scripts/export-crate.sh <crate-name>
```

Example:

```bash
./scripts/export-crate.sh patchhive-product-core
```

or:

```bash
./scripts/export-crate.sh patchhive-github-pr
```

or:

```bash
./scripts/export-crate.sh patchhive-github-data
```

or:

```bash
./scripts/export-crate.sh patchhive-github-security
```

If you want to push directly to a standalone crate remote:

```bash
./scripts/export-crate.sh patchhive-product-core product-core main
```

That creates a subtree export branch from `crates/patchhive-product-core` and can push it directly into a standalone crate repository.

If a shared crate's git dependencies change, validate its standalone-safe
lockfile before exporting:

```bash
./scripts/refresh-crate-lockfile.sh patchhive-github-security
```

`export-crate.sh` runs that validation automatically and writes the generated
lockfile only to the standalone export branch.

## Crate Mirror Sync Script

For shared crate repositories, PatchHive prefers clean crate-only mirror history over raw subtree history.

After the first export, use:

```bash
./scripts/sync-crate-mirror.sh patchhive-product-core product-core main
```

or:

```bash
./scripts/sync-crate-mirror.sh patchhive-github-pr github-pr main
```

or:

```bash
./scripts/sync-crate-mirror.sh patchhive-github-data github-data main
```

or:

```bash
./scripts/sync-crate-mirror.sh patchhive-github-security github-security main
```

If you want to reset an existing crate mirror onto the clean sync history model, use:

```bash
./scripts/sync-crate-mirror.sh patchhive-product-core product-core main --reset-history
```

## Template Export Script

Use:

```bash
./scripts/export-template.sh <template-name>
```

Example:

```bash
./scripts/export-template.sh product-starter
```

If you want to push directly to a standalone template remote:

```bash
./scripts/export-template.sh product-starter product-starter main
```

That creates a subtree export branch from `templates/product-starter` and can push it directly into a standalone template repository.

If the template scaffold has a Rust backend, `export-template.sh` now refreshes the scaffold's standalone-safe `backend/Cargo.lock` before exporting.

## Service Export Script

Use:

```bash
./scripts/export-service.sh <service-name>
```

Example:

```bash
./scripts/export-service.sh patchhive-backend
```

If you want to push directly to a standalone service remote:

```bash
PATCHHIVE_EXPORT_FORCE_WITH_LEASE=1 ./scripts/export-service.sh patchhive-backend https://github.com/patchhive/patchhive-unified-backend.git main
```

PatchHive services currently use monorepo-relative Rust path dependencies.
`export-service.sh` therefore fails closed instead of producing a service-only
subtree that cannot compile or build its Docker image. Build and publish the
unified backend image from the monorepo root for now. A future standalone
service mirror must carry an explicit dependency bundle and preserve the root
Docker build context before this guard can be removed.

## Recommended First Export

1. Create an empty GitHub repository for the product.
2. Add it as a remote in the monorepo.
3. Run the export script.
4. Push the export branch to the product repo.
5. Update the standalone product repo to use published shared packages.

## Recommended Package Export

1. Create an empty GitHub repository for the package.
2. Add it as a remote in the monorepo.
3. Run the package export script.
4. Push the export branch to the package repo.
5. Keep releases and canonical history rooted in the monorepo.
6. After the initial export, prefer `sync-package-mirror.sh` for future mirror updates.

## Recommended Crate Export

1. Create an empty GitHub repository for the crate.
2. Add it as a remote in the monorepo.
3. Run the crate export script.
4. Push the export branch to the crate repo.
5. Point exported product backends at the crate's standalone git dependency.

## Recommended Template Export

1. Create an empty GitHub repository for the template.
2. Add it as a remote in the monorepo.
3. Run the template export script.
4. Push the export branch to the template repo.
5. Keep the canonical scaffold in the monorepo.
6. Keep the canonical template history and docs rooted in the monorepo.

## Recommended Service Export

1. Keep the service source under `services/<service-name>` in the monorepo.
2. Build and publish its image from the monorepo root.
3. Do not publish a service-only subtree while its Cargo manifest retains
   monorepo-relative path dependencies.
4. Add and verify a portable dependency bundle before enabling a standalone
   service mirror.

## Day-To-Day Workflow

The intended long-term flow is:

1. Build inside the monorepo.
2. Commit and push monorepo changes first.
3. Export a product, package, crate, template, or service when you want its standalone repository updated.
4. Push the export branch into the corresponding standalone repository.

This keeps one clean source of truth while still giving each product its own GitHub identity.
