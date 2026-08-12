# Specialist UI Architecture

PatchHive's eleven specialist products use one canonical frontend location and
one shared visual system:

- product applications live in `products/<product>/frontend/`;
- shared specialist components live in `packages/ui/` and are published as
  `@patchhivehq/ui`;
- authentication and browser API helpers live in `packages/product-shell/`;
- product APIs are served in-process by `services/patchhive-backend` under
  `/api/products/<product>`.

There is no active specialist UI migration track. Directories named
`frontend-v2`, `frontend-v3`, or `frontend-legacy` are not valid specialist
product targets. HiveCore is a separate control-plane application and is not
covered by this document.

## Product boundary

Every specialist remains an independent frontend, container image, product
workflow, and API namespace. Share stable shell and interaction primitives;
keep product copy, evidence, forms, controls, actions, and safety decisions in
the product.

Use `IntegratedProductApp` for the standard workspace, history, checks, and
sources flow. Products with a genuinely different workflow may compose the
same `ProductShell`, controls, history, list, timeline, and diagnostics
primitives directly.

## Required behavior

- Persist the suite theme as `patchhive.theme` and apply it before React mounts.
- Show aggregate KPIs once. Use reclaimed dashboard space for prioritized,
  clickable evidence rather than duplicate totals.
- Read-only products describe an assessment and explain what drives review
  priority; they do not imply a merge or execution decision.
- Retain every first-class finding inside the configured input scope. APIs may
  paginate retained evidence, while the UI progressively reveals it and filters
  the complete collection.
- Use saved dashboard views, shared filter/sort controls, and activity timelines
  for repeatable investigation.
- Put presets, schedules, target selection, repository controls, and suite
  service integration in a **Controls** tab when supported.
- Label target selection **Target repo** and **Autonomous discovery** and store
  them as `direct` and `discovery`; never infer discovery from an empty target.
- Keep startup checks and operational summaries in independent responsive
  columns. Do not stretch compact cards to match the tallest column.
- Render GitHub as verified only after authenticated identity verification.
- Keep the footer identity `<Product> by PatchHive`, the product subtitle, and
  `Autonomous maintenance suite`.

## Design source and verification

`packages/ui/` is the canonical implementation and design reference for tokens,
typography, spacing, radii, glass surfaces, shadows, backgrounds, motion, and
responsive behavior. Product screenshots are verification evidence, not a
replacement implementation source.

For a changed specialist frontend, run its production build and the shared
frontend dependency smoke test. Exercise loading, empty, authenticated,
degraded, permission-failed, narrow viewport, history, details, checks, sources,
and controls states that the product supports.
