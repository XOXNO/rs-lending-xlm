# Maintainer notes

Repository settings and CI rules that maintainers keep. Contributors do not
need these to open a pull request.

## Self-hosted CI safety

Some workflows execute pull-request-controlled code on persistent self-hosted
runners. The real boundary is the repository setting that requires approval
for workflow runs from outside collaborators. This setting lives in the
repository's GitHub Actions configuration, not in workflow YAML. Keep it on.

The workflow guard is a second layer. Every self-hosted job that can run on
`pull_request` carries
`github.event.pull_request.head.repo.full_name == github.repository` in its
`if`, and `.github/scripts/check_workflows.py` fails such a job without it. A
job whose `if` admits only push, schedule or `workflow_dispatch` events is
exempt. A `pull_request` run uses the workflow files of the pull request
itself, so the guard protects only a fork pull request that leaves
`.github/**` unchanged. A maintainer must never approve a fork run that
changes `.github/**`.

A fork pull request skips the self-hosted gates and gets a failing
`Fork pull request` check. That check runs on a GitHub-hosted runner and checks
out no code. To run the gates, a maintainer who has read the change pushes its
branch to this repository. GitHub reports a skipped job as successful, so
administrators should make both the `Static Gates` and the `Fork pull request`
checks required on `main`. They should also require actions pinned to a
full-length commit SHA.

The release build job runs on a GitHub-hosted runner, restores no cache, and
builds with `--locked`. Its attested WASM therefore comes only from the
checked-out commit (a tag, or a branch on a dry run) and the committed
`Cargo.lock`.

Pin every action to a full commit SHA with a trailing `# <ref>` comment. The
Static Gates job runs on every same-repository pull request and fails on an
unpinned action. The stellar-cli installer checks the release tarball against a
pinned SHA-256, so a `STELLAR_VERSION` bump needs the new digests in
`.github/scripts/install-stellar-cli.sh`.

Do not weaken pinned-action, least-privilege, or deployable-ABI safeguards to
make CI pass. Raise a maintainer discussion if a legitimate change needs a
different CI permission or execution model.

## Dependency maintenance

Accepted and tracked: RUSTSEC-2024-0436 for `paste` 1.0.15
in the root and fuzz lockfiles. It is an unmaintained build-time proc-macro
pulled in by the Soroban SDK dependency tree, with no fixed release. Recheck
this exception when upgrading the SDK or when an upstream replacement becomes
available.

Run workflow audits with `zizmor .github`. This scopes the scan to this
repository's active workflows and Dependabot configuration; workflow files
under `vendor/` are upstream source and GitHub does not execute them here.
