# Phase Output Schema

Every phase writes one markdown file per domain following this exact structure. The fixed structure makes the next phase's parsing trivial.

## Domain header (top of every file)

```markdown
# Domain N — <domain name>

**Phase:** 1 | 2 | 3
**Files in scope:**
- `test-harness/tests/<file_a>.rs`
- `test-harness/tests/<file_b>.rs`
- ...

**Totals:** broken=X weak=Y nit=Z (Phase 1)
            confirmed=A refuted=B refined=C new=D (Phase 2)
            applied=A failed=F skipped=S (Phase 3)
```

## Per-test entry (Phase 1)

````markdown
### `<test_file>.rs::<test_name>`

**Severity:** broken | weak | nit | none
**Rubric items failed:** [1, 3, 4]   (or "none" if severity = none)
**Why:** one-paragraph explanation citing line numbers.

**Patch (suggested):**
```diff
--- before
+++ after
@@ ... @@
-old code
+new code
```
````

If `Severity: none`, omit `Rubric items failed`, `Why`, and `Patch` blocks — just the heading.

## Per-test entry (Phase 2)

Same as Phase 1 plus:

```markdown
**Disposition:** confirmed | refuted | refined | new
**Reviewer note:** required when disposition = refuted or refined; one-paragraph justification or replacement-patch rationale.
```

If `disposition = refined`, the `Patch (suggested)` block is replaced with the reviewer's rewritten patch.
If `disposition = new`, the entire entry is added by the reviewer (Phase 1 didn't flag it).

## Per-test entry (Phase 3)

Same as Phase 2 plus:

```markdown
**Result:** applied | failed | skipped
**Test run:** `cargo test --test <file_stem>` -> X passed, Y failed
**Rollback reason:** required when result = failed; one-line description of why the patch broke a test.
```

`skipped` is used only for `disposition = refuted` from Phase 2.
