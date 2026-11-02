# pyk 7.1.337 patch used by the Komet proofs

Komet pins `kframework==7.1.337` (pyk) in `uv.lock`. The proofs use three small
changes to that package, kept here as a patch with Runtime Verification's
BSD-3-Clause license (`LICENSE.md`):

1. `konvert/_kast_to_kore.py`: emit the `preserves-definedness` rule attribute
   to Kore. Stock pyk raises `Do not know how to convert AttEntry to Kore` for
   an `--extra-module` rule that carries it.
2. `proof/reachability.py` `_may_subsume`: treat only a failed match (`None`) as
   "cannot subsume". Stock code also rejects a successful match with an empty
   substitution (falsy), which skips valid subsumption checks.
3. `proof/reachability.py` `_check_subsume`: an SMT `unknown` (retry limit
   reached) while checking a cover yields no cover and leaves the node to be
   explored, instead of aborting the proof.

None of these closes a proof obligation by itself: (1) passes an attribute the
backend already understands, (2) only stops skipping a check the backend still
performs, (3) only declines to add a cover.

`apply.sh <site-packages>/pyk` checks the stock file hashes, applies the patch
and checks the patched hashes in `SHA256SUMS` (the same bytes as the original
borrow-double runtime's pyk overlay).
