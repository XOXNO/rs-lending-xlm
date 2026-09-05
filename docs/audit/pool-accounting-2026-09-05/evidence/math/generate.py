"""Generate independent unbounded-integer expectations; standard library only."""
from pathlib import Path
import itertools
import random

out = Path(__file__).resolve().parent
lo = -(1 << 127)
hi = (1 << 127) - 1
xs = [lo, lo + 1, -10**38, -10**27 - 1, -10**27, -10**18, -2**64,
      -3, -2, -1, 0, 1, 2, 3, 2**63 - 1, 2**64, 10**18, 10**27,
      10**27 + 1, 10**38, hi - 1, hi]
triples = list(itertools.product(xs, xs, [lo, -10**27, -3, -2, -1, 1, 2, 3, 10**27, hi]))
rng = random.Random(99613335)
for _ in range(12000):
    triples.append((rng.randrange(lo, hi + 1), rng.randrange(lo, hi + 1),
                    rng.randrange(lo, hi + 1) or 1))
for _ in range(4000):
    triples.append((rng.randrange(0, hi + 1), rng.randrange(0, hi + 1),
                    rng.randrange(1, hi + 1)))

def opt(value):
    return str(value) if lo <= value <= hi else "_"

rows = []
for x, y, d in triples:
    p = x * y
    floor = p // d
    ceil = -((-p) // d)
    half_up = (p + d // 2) // d if x >= 0 and y >= 0 and d > 0 else None
    rows.append("\t".join([str(x), str(y), str(d), opt(floor), opt(ceil),
                            str(max(lo, min(hi, floor))),
                            opt(half_up) if half_up is not None else "_"]))
(out / "mul-div.tsv").write_text("\n".join(rows) + "\n")
print(f"Generated {len(rows)} independent integer-reference cases in {out}")
