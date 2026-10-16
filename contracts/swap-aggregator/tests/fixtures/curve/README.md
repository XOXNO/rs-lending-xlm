# Curve pool fixtures

Fetched from Stellar testnet on 2026-10-16. These are deployed pool executables,
not local substitutes. Tests construct fresh local pool storage and liquidity;
they do not reproduce deployed balances or prove resource fit (unlimited budget).

| File | Source contract | SHA-256 |
|---|---|---|
| `pool-2coin.wasm` | `CD65V5XD6B3AVNALJAAXNR4ZCIIKISYO4GKOIAT6ZXBQFPEI26WDVV3Z` | `af5c110c313f1bb9d7605415ff53ad44c6197eabe8d8c8204972c1fb12cb895d` |
| `pool-3coin.wasm` | `CCAFTS7MA6ZXKNSGZWVPK7WNXSZULY2UEHQNHS7TJ3CFHWNH3HMTFI6K` | `d04f845f374548e30cb72c921d8e50d90f96165722d4816ee51f11eebb2e21d1` |

Both expose the same pool ABI; tests import the two-coin spec once and deploy
the selected executable through a small native factory. Setup and swaps mock
only exact account authorization trees; contract transfer authorization comes
from the production router. No chain writes.
