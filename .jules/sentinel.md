## 2026-06-26 - Revenue Claims Affect Utilization

**Vulnerability:** Claiming protocol revenue burns revenue shares from total supplied value. Without a post-claim utilization check, a permissionless claim can push a market above its configured max utilization.

**Learning:** Revenue shares are part of the pool's supplied total, so claiming them is economically similar to a withdrawal even though it is protocol-owned accounting.

**Prevention:** Any flow that reduces supplied value or cash, including protocol revenue claims, must re-check pool utilization before committing state or transferring tokens.
