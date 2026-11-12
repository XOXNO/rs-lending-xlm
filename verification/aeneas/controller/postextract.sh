#!/usr/bin/env bash
# Minimal fix of one Aeneas code-generation bug, run by ../run.sh on the
# generated Lean library directory (argument 1) after extraction.
#
# Bug: for the shim's `impl Iterator for VecIter<T>`, Aeneas emits the
# provided methods the code uses (`all`, `find`) as fields of the trait
# instance, although the Aeneas Lean library's `Iterator` structure has no
# such fields (Aeneas itself warns: "could not find the information for item
# 'all'/'find'"). The calls themselves go to
# `core.iter.traits.iterator.Iterator.{all,find}.default` directly, so the
# fix deletes the two field definitions from the instance and nothing else.
# See ../docs/controller.md ("Aeneas issues").
set -euo pipefail
f="$1/Funs.lean"
python3 - "$f" <<'PY'
import sys
p = sys.argv[1]
lines = open(p).read().split("\n")
out, fixed, i = [], 0, 0
head = "impl_def aeneas_soroban_shim.VecIter.Insts.CoreIterTraitsIteratorIterator "
while i < len(lines):
    line = lines[i]
    out.append(line)
    i += 1
    if line.startswith(head):
        # copy up to and including the `next := ...` field
        while not lines[i].startswith("  next := "):
            out.append(lines[i]); i += 1
        out.append(lines[i]); i += 1
        # drop the `all := ...` / `find := ...` fields (each continues on
        # lines indented by 4) up to the closing brace
        while lines[i].startswith("  all := ") or lines[i].startswith("  find := "):
            i += 1
            while lines[i].startswith("    "):
                i += 1
        if lines[i] != "}":
            sys.exit("postextract.sh: unexpected VecIter Iterator instance shape")
        fixed += 1
if fixed != 1:
    sys.exit(f"postextract.sh: expected one VecIter Iterator instance to fix, found {fixed}")
open(p, "w").write("\n".join(out))
PY
