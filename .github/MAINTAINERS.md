# Maintainer notes

Repository settings and CI rules that maintainers keep. Contributors do not
need these to open a pull request.

## Required repository settings

These settings live in the GitHub repository configuration, not in workflow
YAML. Keep every one on.

- [ ] GitHub Actions requires approval for workflow runs from outside
      collaborators.
- [ ] Branch protection on `main` requires the `Static Gates` check.
- [ ] Branch protection on `main` requires the `Fork pull request` check.
- [ ] Actions must be pinned to a full-length commit SHA.

GitHub reports a skipped job as successful. Both checks must therefore be
required, or a fork pull request can merge without the gates.

## Self-hosted CI safety

Some workflows run pull-request-controlled code on persistent self-hosted
runners. The outside-collaborator approval setting is the real boundary.

The workflow guard is a second layer:

- Every self-hosted job that can run on `pull_request` carries
  `github.event.pull_request.head.repo.full_name == github.repository` in its
  `if`.
- `.github/scripts/check_workflows.py` fails such a job without the guard.
- A job whose `if` admits only push, schedule or `workflow_dispatch` events is
  exempt.
- A `pull_request` run uses the workflow files of the pull request itself. The
  guard therefore protects only a fork pull request that leaves `.github/**`
  unchanged.
- Never approve a fork run that changes `.github/**`.

A fork pull request skips the self-hosted gates and gets a failing
`Fork pull request` check. That check runs on a GitHub-hosted runner and checks
out no code. To run the gates, read the change, then push its branch to this
repository.

The release build job runs on a GitHub-hosted runner, restores no cache, and
builds with `--locked`. Its attested WASM comes only from the checked-out
commit (a tag, or a branch on a dry run) and the committed `Cargo.lock`.

## Action and tool pinning

- Pin every action to a full commit SHA with a trailing `# <ref>` comment. The
  Static Gates job runs on every same-repository pull request and fails on an
  unpinned action.
- The stellar-cli installer checks the release tarball against a pinned
  SHA-256. A `STELLAR_VERSION` bump needs the new digests in
  `.github/scripts/install-stellar-cli.sh`.

Do not weaken pinned-action, least-privilege, or deployable-ABI safeguards to
make CI pass. If a legitimate change needs a different CI permission or
execution model, raise a maintainer discussion.

## Dependency maintenance

Accepted and tracked: RUSTSEC-2024-0436 for `paste` 1.0.15 in the root and
fuzz lockfiles. It is an unmaintained build-time proc-macro from the Soroban
SDK dependency tree, with no fixed release. Recheck this exception when you
upgrade the SDK or when an upstream replacement appears.

Run workflow audits with `zizmor .github`. This scopes the scan to this
repository's active workflows and Dependabot configuration. Workflow files
under `vendor/` are upstream source, and GitHub does not run them here.
