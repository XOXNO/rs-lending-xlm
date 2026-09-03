# Coordination protocol (mandatory for all agents)

1. **Stay on the current git branch.** Do not `git checkout`, `git switch`,
   create branches, commit, or push. The coordinator alone manages git.
2. Write only under `docs/audit/controller-defense/findings/` (and optional
   `disagreements/`). Do not edit production Rust.
3. One finding file per agent id: `AXXX-<slug>.md`.
4. Read `shared/SEED.md` and peer findings before claiming a novel critical gap.
5. Concurrent async agents are capped at 10; waves batch toward 110 scopes.
