# Chainlink testnet rollout — 2026-10-14

Deployed source commit `d15ffc0ac105c5b1bdb6760118bddacd4a8a230e` from `feat/chainlink-oracle-support`. Testnet was verified unpaused at ledger 5198179. All 23 deployment and smoke-test transactions were independently confirmed `SUCCESS`.

Local wallet: `deployer` (`GDBBOILYIJBSUQKC3Z3USAW3DGPFHIGVKYA5T4ZUZBO56HBUPHJEN3FV`). RPC: `https://stellar-testnet-gateway.xoxno.com`. Passphrase: `Test SDF Network ; September 2015`. Proposer, executor and guardian roles were checked; governance delay remained 12 ledgers.

| Upgraded contract | Address | Verified WASM SHA-256 |
| --- | --- | --- |
| price_aggregator | `CBMARXSYE56NPDLX42TTICM5GS5XQCYRD5VMCJME624TS6C2S53VETYU` | `6c143e38a4f751331ba92f46c668f906940ca4717a3a0146ef13bb50166896a5` |
| governance | `CDS33JDOYH3F3FL4QUQ6DV4WKHML2AKHF4LADTZ57FRUAEBTE7NMY5FQ` | `07b7befd9b5b57a78b7cba34a55c10572d55e86a2034d0d2f722c7ac8d4b4207` |

Independent reviews found no blocking contract defect. Chainlink reader ABI, signed price conversion, freshness gates and legacy storage compatibility were checked against live testnet interfaces. Governance and price aggregator were upgraded individually; controller and pool retained their existing code hashes.

USDC, XLM, BTC and ETH now use Chainlink primary prices with their existing RedStone Fundamental anchors. Chainlink source and asset freshness budgets are 93,600 seconds (26 hours); RedStone remains 86,400 seconds. Stored configs were read back, confirming unchanged asset decimals, independence, tolerance and sanity bounds. USDC_HUB2 and USDC_LIQVID share the USDC token oracle key.

The XLMUSDC_LP freshness ceiling was raised from 86,400 to 93,600 seconds before applying the underlying Chainlink configs. Dependent revalidation initially rejected the USDC change with `InvalidStalenessConfig #218`; this failed simulation submitted no transaction. The LP update preserved all other fields. An earlier simulation also correctly rejected execution before timelock readiness.

Optimized deploy WASMs were built with `RUSTC_WRAPPER= make deploy-artifacts`, disabling a local sccache sandbox failure. After building the missing WASM fixtures, the common, governance and price-aggregator suites passed 733 tests. Exporter tests passed 55; `make ops-script-check` passed. Config validation reported zero errors and two existing warnings: ETH and EURC_LIQVID remain unlisted in enabled spokes.

All 11 configured markets returned valid prices without stale, deviation or error flags before unpause and after the live lending test. ETH pricing is verified; ETH borrowing remains unlisted.

Live account 53: supplied 6 USDC, borrowed 1 XLM, verified positive debt and health factor 31.965104141625914039, repaid fully using a 1.1-XLM cap with excess refunded, and withdrew all 6 USDC. Debt readback was zero; the account was closed. The wallet USDC balance returned exactly to 7.275943. XLM balance changes include transaction fees and loan interest. Every mutation was simulated before signing.

All submitted transactions, including uploads and proposals:

| Action | Ledger | Transaction |
| --- | --- | --- |
| Pause | 5198012 | [faa45f16c7f5…](https://stellar.expert/explorer/testnet/tx/faa45f16c7f579c7bea5b52dee5011febc61b01fbc8439afcc4e4dd833a6815c) |
| Upload price aggregator | 5198019 | [fb6367a5d204…](https://stellar.expert/explorer/testnet/tx/fb6367a5d204a3c130a78969d701b7e48f3155ef86fd3a427c5e8b60f927b169) |
| Propose price aggregator upgrade | 5198020 | [e2ead5898f6d…](https://stellar.expert/explorer/testnet/tx/e2ead5898f6d3952bcbe98f0ba23d2e19b31ee8075295d706e29ae4d88b58cbc) |
| Execute price aggregator upgrade | 5198033 | [8617ed4ef998…](https://stellar.expert/explorer/testnet/tx/8617ed4ef99806b3dd4365ed162ecadd87ac17f26c669365d3ef5294b5f4b591) |
| Upload governance | 5198035 | [1f4ec9bbc402…](https://stellar.expert/explorer/testnet/tx/1f4ec9bbc4027fd84ec0487446c332d05b07202432f2e5fb1fd899d8384eafc4) |
| Propose governance upgrade | 5198037 | [a7ae186fddb6…](https://stellar.expert/explorer/testnet/tx/a7ae186fddb659e9bf65f7ba17b2d333b55b44a732d994f7d3508f19a90abdf0) |
| Execute governance upgrade | 5198050 | [d992101aecab…](https://stellar.expert/explorer/testnet/tx/d992101aecabeb3e6063b595ce49dac33cc1a4c8d1c98aae28b93a7aeda27052) |
| Propose USDC oracle | 5198057 | [e75433f59177…](https://stellar.expert/explorer/testnet/tx/e75433f59177b8186fb1de4afafc84e750869bc546f2fc1f75edc5bcd691aef2) |
| Propose XLM oracle | 5198059 | [511b643270e9…](https://stellar.expert/explorer/testnet/tx/511b643270e93896e56db8b27da9d56446860f7eccfafbc882a60aec46a5c8ef) |
| Propose BTC oracle | 5198061 | [51c43bbe33c4…](https://stellar.expert/explorer/testnet/tx/51c43bbe33c411461c74a7bbfd4ab7275da3c2165d2169ab3140ea01793e7464) |
| Propose ETH oracle | 5198063 | [d36841a31722…](https://stellar.expert/explorer/testnet/tx/d36841a3172201e71185002e4243a30b984cb2a8623c8ac6ed60449db9624186) |
| Propose LP envelope | 5198101 | [1eac3da1ad7f…](https://stellar.expert/explorer/testnet/tx/1eac3da1ad7fdb4626226660d21a39239b9f142011b07b7b5907d240a2685262) |
| Execute LP envelope | 5198114 | [48eef789bbe0…](https://stellar.expert/explorer/testnet/tx/48eef789bbe0bc8d8c6faab3518e5c12a4664abdfbaae2a54aade1e9b65c9c7e) |
| Execute USDC oracle | 5198118 | [e473d52d3fdc…](https://stellar.expert/explorer/testnet/tx/e473d52d3fdcf72507a6738a2b15e301421213d0f5344ac588b9d807a335b0ed) |
| Execute XLM oracle | 5198120 | [03061852464a…](https://stellar.expert/explorer/testnet/tx/03061852464afc34d07a50d9a42c477ff818d0e28b9c50b794c77f017e51f488) |
| Execute BTC oracle | 5198122 | [51c896aa3fae…](https://stellar.expert/explorer/testnet/tx/51c896aa3fae3ba537d3cbc0d052e34db2ef619c2546c75f9ce7fc79fad6a8af) |
| Execute ETH oracle | 5198124 | [4764420b4f9e…](https://stellar.expert/explorer/testnet/tx/4764420b4f9e95a87fd2c98c3aedee4b3dbb4e478497e1bb28ee0bcd801f5fba) |
| Propose unpause | 5198141 | [28758130ffb1…](https://stellar.expert/explorer/testnet/tx/28758130ffb1fb8c1efa8041af1d484c10f8370c7b1d16260d6d8b870f6da6cd) |
| Execute unpause | 5198154 | [4acbb027ae28…](https://stellar.expert/explorer/testnet/tx/4acbb027ae28d93c98e4b6d6aa018f6003b388dd588fc30158f1e77230d0535d) |
| Smoke: supply 6 USDC | 5198168 | [476cff7d8d88…](https://stellar.expert/explorer/testnet/tx/476cff7d8d886ae663fda38f1b4714af07939882d247c99845efab6721baca3d) |
| Smoke: borrow 1 XLM | 5198169 | [cf692ac9bf87…](https://stellar.expert/explorer/testnet/tx/cf692ac9bf8790ba761a81513be387be0d00e27c9265404781033865179a9356) |
| Smoke: repay XLM | 5198171 | [e3f4627fdeec…](https://stellar.expert/explorer/testnet/tx/e3f4627fdeec1f6629ca313252028374b9c5f34fe12aa8fa942a61147e18c138) |
| Smoke: withdraw all USDC | 5198172 | [d6e53a29f108…](https://stellar.expert/explorer/testnet/tx/d6e53a29f108ac84afcea39d4a6534d635191d2de6c82ba7d828c357ba1c6bdc) |

The [machine-readable evidence](chainlink-testnet-2026-10-14.json) contains all transaction hashes, statuses, ledgers, contract hashes, stored oracle records, market quotes and loan results. Executed governance operation records are under `configs/ops/testnet/`.

For subsequent upgrades, pause first; upgrade price aggregator and governance; update dependent oracle freshness ceilings before loosening their underlying sources; apply token configs; verify stored values and prices; then unpause. `upgradeAll` unpauses automatically, so use individual upgrade targets for this ordering.
