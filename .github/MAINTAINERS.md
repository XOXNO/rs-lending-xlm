# Maintainer notes

Repository settings and CI rules maintainers keep. Contributors do not need them.

## Required repository settings

These live in the GitHub repository configuration, not workflow YAML. Keep every one on.

- [ ] GitHub Actions requires approval for workflow runs from outside
      collaborators.
- [ ] Branch protection on `main` requires the `Static Gates` check.
- [ ] Branch protection on `main` requires the `Fork pull request` check.
- [ ] Actions must be pinned to a full-length commit SHA.

GitHub reports a skipped job as successful, so without both required checks a fork pull request can merge without the gates.

## Self-hosted CI safety

Some workflows run pull-request-controlled code on persistent self-hosted
runners. The outside-collaborator approval setting is the real boundary; the
workflow guard is a second layer:

- Every self-hosted job that can run on `pull_request` carries
  `github.event.pull_request.head.repo.full_name == github.repository` in its
  `if`.
- `.github/scripts/check_workflows.py` fails such a job without the guard.
- A job whose `if` admits only push, schedule or `workflow_dispatch` events is
  exempt.
- A `pull_request` run uses the pull request's own workflow files, so the guard
  protects only a fork pull request that leaves `.github/**` unchanged. Never
  approve a fork run that changes `.github/**`.

A fork pull request skips the self-hosted gates and gets a failing
`Fork pull request` check, which runs on a GitHub-hosted runner and checks out
no code. To run the gates, read the change, then push its branch here.

The release build job runs on a GitHub-hosted runner, restores no cache, and
builds with `--locked`. Its attested WASM comes only from the checked-out
commit (a tag, or a branch on a dry run) and the committed `Cargo.lock`.

## Action and tool pinning

- Pin every action to a full commit SHA with a trailing `# <ref>` comment.
  Static Gates fails an unpinned action on every same-repository pull request.
- The stellar-cli installer checks the release tarball against a pinned
  SHA-256. A `STELLAR_VERSION` bump needs the new digests in
  `.github/scripts/install-stellar-cli.sh`.

Do not weaken pinned-action, least-privilege or deployable-ABI safeguards to
make CI pass. Raise a maintainer discussion when a legitimate change needs a
different CI permission or execution model.

## Dependency maintenance

Accepted and tracked: RUSTSEC-2024-0436 for `paste` 1.0.15 in the root and
fuzz lockfiles, an unmaintained build-time proc-macro from the Soroban SDK tree
with no fixed release. Recheck it on an SDK upgrade or when an upstream
replacement appears.

Run workflow audits with `zizmor .github`, which scopes the scan to active
workflows and Dependabot configuration. Workflow files under `vendor/` are
upstream source that GitHub does not run here.
