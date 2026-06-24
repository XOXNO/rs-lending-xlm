# CI/CD Flow Redesign — rs-lending-xlm (+ scout-audit action)

Date: 2026-06-24
Status: Approved (design), pending implementation plan

## Problem

The protocol CI was recently decomposed into one workflow per task (`tests`,
`security`, `scout`, `coverage`, `fuzz`, `certora-*`, `release`). Three of those —
the pure gates `tests`, `security`, `scout` — trigger on **both** `push` and
`pull_request` over the same branch set, which wastes self-hosted runner time:

1. **Double run** on every push to a `feat/*`/`rc/*` branch that has an open PR
   (the `push` event and the `pull_request: synchronize` event both fire).
2. **Redundant post-merge run** on `main`: merging a PR re-runs all three on the
   `push` to main, re-validating code the PR just validated.

Two adjacent gaps:

3. The Scout action can only gate on **any** finding (`fail-on-findings`), not on a
   severity threshold, so it cannot block a PR on *Critical only* without noise.
4. The protocol pins the Scout action to a **commit hash** (unreadable) after a
   moving-tag (`@v1`) stale-cache failure on the self-hosted runner.

## Context (current state)

- **Merge model:** PRs required; branch protection requires branches to be
  **up to date** before merging. Therefore a PR's merge-ref equals exactly what
  `main` becomes, and `main` only advances via a merged, passing, up-to-date PR.
- **Runners:** self-hosted (persistent `$HOME` + workspace; jobs on a single
  runner queue, multiple runner instances parallelise with isolated `_work`).
- **Workflows today:** `tests` (build/test/clippy/wasm-size), `security`
  (OpenZeppelin soroban-scanner; hard-gates Critical/High on PRs), `scout` (XOXNO
  Scout, advisory), `coverage` (push=baseline refresh, PR=diff gate), `fuzz`
  (PR smoke + miri, nightly deep campaign), `certora-fastRules` +
  `certora-verification` (manual `workflow_dispatch`), `release` (`release:` event).

## Decisions

### 1. PR-only gating for the three pure gates

`tests`, `security`, `scout` trigger on `pull_request` (+ `workflow_dispatch`) over
`[master, main, feat/*, rc/*]`; **drop the `push` trigger**.

Rationale: with up-to-date-required PRs, the `pull_request` merge-ref is the exact
tree that lands on `main`, and branch protection already requires these checks — so
the post-merge `push` run is pure duplication. Dropping `push` also collapses the
feat-branch double-run to a single `synchronize` run. `main` stays green because
nothing merges without an up-to-date passing PR. Keep `workflow_dispatch` for manual
re-runs.

`coverage`, `fuzz`, `certora-*`, `release` are **unchanged** — their non-PR triggers
are load-bearing (coverage's `push` refreshes the baseline PRs diff against; fuzz
already splits PR-smoke from the nightly deep run; Certora is intentionally manual;
release is the mainnet path).

### 2. Stage map

| Stage | Runs |
|---|---|
| **Every PR** (minutes) | `tests`, `security` (gates Critical/High), `scout` (gates Critical), `fuzz` smoke + miri, `coverage` diff-vs-baseline |
| **Push to `main`** | `coverage` baseline refresh **only** |
| **Nightly (cron)** | `fuzz` deep campaign |
| **Manual / pre-release** | `certora-fastRules`, `certora-verification` |
| **Release event** | reproducible build + deploy artifacts |

### 3. Scout — severity-aware gating

Add a **`fail-on-severity`** input to the scout-audit composite action:
`none` (default) | `critical` | `medium` | `minor` | `enhancement` | `any`. When set,
the action parses Scout's per-severity counts (`by_severity`: Critical/Medium/Minor/
Enhancement) from the JSON report and fails the step if any finding **at or above**
the threshold exists. Severity order (high→low): Critical > Medium > Minor >
Enhancement. The existing `fail-on-findings` remains as a back-compat alias for
`fail-on-severity: any`. Gating stays self-contained in the action — no grep step in
the consumer workflow.

`rs-lending-xlm` `scout.yml` inputs:

- `contracts`: pool, controller, governance, defindex-strategy
- `exclude`: `dos-unexpected-revert-with-storage`
- `output-format`: **`json`** (required for robust severity parsing)
- `fail-on-severity`: **`critical`**
- `extra-args`: `"-- --locked"`
- auto-upload on (`upload-reports: true`, `artifact-name: scout-audit-reports`)

Trade-off (accepted): gating requires JSON, so the uploaded artifact is JSON rather
than the prettier `md`. Rejected alternative: keep `md` and parse the md Summary's
Critical column — more fragile than parsing structured JSON.

### 4. Immutable semver tags

Root cause of the prior breakage: a **moving** `@v1` tag + the self-hosted runner
caching action checkouts under `_work/_actions/.../v1/` → stale build served.

- Cut **`v1.1.0`** on `scout-audit` at the current feature-complete state (composite
  action + `env` passthrough + spec-shaking shim + auto-upload + new
  `fail-on-severity`). Immutable — never moved.
- Keep **`v1`** as a moving major alias for general/GitHub-hosted consumers (README),
  with a documented note that self-hosted users should pin an immutable `vX.Y.Z`.
- Pin `rs-lending-xlm` `scout.yml` to **`@v1.1.0`** — readable (not a hash) and
  cache-safe (an immutable tag never goes stale on a self-hosted runner).

## Implementation footprint

**scout-audit** (`/Users/mihaieremia/GitHub/scout-audit`):
- `action.yml`: add `fail-on-severity` input + severity-threshold gate logic over the
  JSON report (`by_severity`); keep `fail-on-findings` as the `any` alias.
- `README.md` + `examples/scout.yml`: document `fail-on-severity` and the tag scheme
  (moving `v1` for general use, pin immutable `vX.Y.Z` on self-hosted).
- Cut + push the immutable `v1.1.0` tag (and move `v1` to the same commit).

**rs-lending-xlm** (`/Users/mihaieremia/GitHub/rs-lending-xlm`):
- `tests.yml`, `security.yml`, `scout.yml`: `on:` → `pull_request` + `workflow_dispatch`
  (drop `push`).
- `scout.yml`: `output-format: json`, add `fail-on-severity: critical`, pin
  `mihaieremia/scout-audit@v1.1.0` (replaces the commit hash).

## Verification

1. All edited workflows parse as valid YAML.
2. A scratch PR shows each fast gate runs **once** (no push+PR double-run); after
   merge, `main` shows no re-run of `tests`/`security`/`scout` (only `coverage`
   baseline refresh).
3. Scout gate: locally exercise `fail-on-severity: critical` against a contract with
   a Critical finding (fails) and a clean contract (passes); confirm Enhancements do
   not trip the gate.
4. `@v1.1.0` resolves and is immutable; the protocol scan runs the intended action
   build (no stale cache).

## Non-goals

- Automating Certora (stays manual `workflow_dispatch`).
- Changing `coverage` or `fuzz` (already optimal).
- Converting standalone workflows to reusable (`workflow_call`) workflows — standalone
  per-task files match the repo's existing convention.
- Branch-protection edits (the maintainer updates required checks to the per-task
  workflow names; noted as a follow-up, not automated here).
