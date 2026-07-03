# CI/CD security notes

## Reviewer-approval gate for self-hosted PR jobs

Several jobs run PR-controlled code (build scripts, tests, Makefile targets, Scout,
fuzz smoke, Miri) on a **persistent self-hosted runner**. A malicious pull request
could otherwise execute arbitrary code on that runner and read caches, tools, or
runner-local state.

These jobs reference the `ci-untrusted-pr` deployment environment:

- `tests.yml` → `build-and-test`
- `security.yml` → `security-scan`
- `scout.yml` → `scout`
- `fuzz.yml` → `pr-smoke`, `miri`

A job that targets an environment with **required reviewers** pauses before it starts
and waits for a maintainer to approve the run. This turns "any PR auto-runs on our
runner" into "a maintainer approves each PR run first."

### Required one-time repo setup (admin, GitHub Settings — cannot be done in YAML)

1. **Settings → Environments → New environment** → name it exactly `ci-untrusted-pr`.
2. Add a **Required reviewers** protection rule listing the maintainers who may
   approve untrusted PR runs.
3. (Optional) Under **Deployment branches**, restrict the environment so only
   protected branches / internal PRs can use it.

Until this environment exists with required reviewers, the gated jobs stay
**pending** on every PR. Creating the environment without reviewers would remove the
gate — the reviewers rule is the control.

## Other hardening in place

- Third-party actions are pinned to immutable commit SHAs (e.g. `scout-audit`), not
  mutable tags.
- Workflows declare least-privilege `permissions:` (`contents: read` for PR jobs;
  the release e2e job is scoped to `contents: write` only).
- `make wasm-size-check` runs `wasm-testing-abi-check`, which fails the build if the
  deployable `governance.wasm` ever exports the test-only `set_controller` ABI.
