# XOXNO Lending addresses and ids

<!-- GENERATED FILE. Do not edit by hand. -->
<!-- Regenerate: python3 scripts/gen_skill_addresses.py   (check: --check) -->

Generated from `configs/networks.json` and `configs/<network>/*.json` in the protocol repository (github.com/XOXNO/rs-lending-xlm). Snapshot; resolve addresses from those files in code.

## Mainnet

- Network passphrase: `Public Global Stellar Network ; September 2015`
- RPC gateway used by XOXNO clients: `https://stellar-gateway.xoxno.com`
- Quote server (swap aggregator API): `https://stellar-swap.xoxno.com`
- XOXNO API (xoxno-api-v2): `https://api.xoxno.com`
- Deployment recorded at ledger `64140891`; timelock min delay `12` ledgers

### Contracts

| Role | Address |
|---|---|
| Governance (timelock; owns controller + price aggregator) | `CC44PEQW7HSEPKAZ5ZRPH2JS5M5KVJXCUBLJ2ZX4E3WCDKMNFILHC2AD` |
| Controller (the only user-facing lending contract) | `CAUCMIN5KSXEVZ7NMXR3LZATGD5EFIEUI5XWTFLYRO2R5OTXI22WE5JX` |
| Pool (custody + accounting; controller-only mutators) | `CBXRNDQMAJFG4VUKMKFEMFS75UUXE2SPSNV4LEEFCSNBCN66PYRWBKXO` |
| Position NFT (token id == account id) | `CAWCSG77AY2W24QZ6ZXLHZU4UXEFHZBM6EI4A4IF7JB5CTY4XND3TI6C` |
| Price aggregator | `CBGUF2G2Q7HCVCWYISDXBHPVBGMYNXA7PG2VET66YBZX6IKOOV27NSMV` |
| Swap aggregator router (`execute_strategy`) | `CCVENFSVCBYDHVOACFZXMNNYVOZ3LKXPZYU5LUI4N7KTXOKRVYD7F3TR` |
| XOXNO oracle adapter | `CDA3XS2HETVCW5GSRN3FH4X3YJ45IA6DNVSZTEOYDFGOCXQK4ZJG22JM` |
| RedStone adapter (external) | `CA526Y2NQWGWVVQ7RFFPGAZMU66PSYJ3UC2MTVAV4ZU7OM5BOPHDXUSG` |
| Reflector CEX oracle (external) | `CAFJZQWSED6YAWZU3GWRTOCNPPCGBN32L7QV43XX5LZLFTK6JLN34DLN` |
| Reflector DEX oracle (external) | `CALI2BYU2JE6WVRUFYTS6MSBNEHGJ35P4AVCZYF3B6QOE3QKOB2PLE6M` |
| Reflector FX oracle (external) | `CBKGPWGKSKZF52CFHMTRR23TBWTPMRDIYZ4O2P5VS65BMHYH4DXMCJZC` |
| Revenue accumulator (G-address) | `GAVWFZK5BGCGBWH4O2CXAHXVRIYVAMGCTZJ24IPLVNLT2WQ2LJBDEEBP` |

### Hubs

| On-chain `hub_id` | Name | Markets |
|---|---|---|
| 1 | Core | XLM, USDC, EURC, PYUSD, USDT0, SolvBTC, xSolvBTC |
| 2 | RWA | USST, SPIKOEUTBL, SPIKOUSTBL, SPIKOUKTBL, SPIKOSAFO, SPIKOEURSAFO, SPIKOGBPSAFO, DEJTRSY, DEJAAA, USTRY, CETES, USDY, XAUM |
| 3 | AMM | AQUA, XLMUSDC_LP, XLMSolvBTC_LP, xSolvBTCSolvBTC_LP, CETESUSDC_LP, USTRYUSDC_LP, USDYUSDC_LP, XAUMUSDC_LP, PYUSDUSDC_LP, XLMAQUA_LP, AQUAUSDC_LP |

### Markets (`HubAssetKey { hub_id, asset }`)

| Symbol | `hub_id` | `asset` | Decimals (oracle config) |
|---|---|---|---|
| XLM | 1 | `CAS3J7GYLGXMF6TDJBBYYSE3HQ6BBSMLNUQ34T6TZMYMW2EVH34XOWMA` | 7 |
| USDC | 1 | `CCW67TSZV3SSS2HXMBQ5JFGCKJNXKZM7UQUWUZPUTHXSTZLEO7SJMI75` | 7 |
| EURC | 1 | `CDTKPWPLOURQA2SGTKTUQOWRCBZEORB4BWBOMJ3D3ZTQQSGE5F6JBQLV` | 7 |
| PYUSD | 1 | `CCCRWH6Q3FNP3I2I57BDLM5AFAT7O6OF6GKQOC6SSJNDAVRZ57SPHGU2` | 7 |
| USDT0 | 1 | `CBSJZEIO5C7KC2SF3MKSNXXJSW5G3VTNBX4ATMKUI3B2MR4JKM4R26YF` | 7 |
| USST | 2 | `CBZ4DCE7PYMUTOAKKUTRSUPT3FJFVOWCSKWUM5A72D6SAVMUJE5JN2PJ` | 18 |
| SPIKOEUTBL | 2 | `CBGV2QFQBBGEQRUKUMCPO3SZOHDDYO6SCP5CH6TW7EALKVHCXTMWDDOF` | 5 |
| SPIKOUSTBL | 2 | `CARUUX2FZNPH6DGJOEUFSIUQWYHNL5AVDV7PMVSHWL7OBYIBFC76F4TO` | 5 |
| SPIKOUKTBL | 2 | `CDT3KU6TQZNOHKNOHNAFFDQZDURVC3MSTL4ML7TUTZGNOPBZCLABP4FR` | 5 |
| SPIKOSAFO | 2 | `CDGSC6BA4TCAOVSFQCUEHDMOIIHYYVNYBT6YEARS4MX3ITAHUINVGQHX` | 5 |
| SPIKOEURSAFO | 2 | `CBOOCGZSVRSZFRE4U2NWR2B4RXYVJWRCBTGOUD2JPI2TDJPWMTJX7FZP` | 5 |
| SPIKOGBPSAFO | 2 | `CAGYRRKPFSWKM6SJOE4QAAVYMOSHMDS5WOQ4T5A2E6XNCU7LZZKUNQKP` | 5 |
| DEJTRSY | 2 | `CBI7UCH5KGSVQRO5H4SUCZUTZABCITZLRHQQZTWL2TK4RZ72TAR6IHRV` | 18 |
| DEJAAA | 2 | `CC64WBDGS6QQP22QTTIACYIXT3WF7BBQEYOQPLTP7GTKYY7PZ74QYGSL` | 18 |
| USTRY | 2 | `CBLV4ATSIWU67CFSQU2NVRKINQIKUZ2ODSZBUJTJ43VJVRSBTZYOPNUR` | 7 |
| CETES | 2 | `CAL6ER2TI6CTRAY6BFXWNWA7WTYXUXTQCHUBCIBU5O6KM3HJFG6Z6VXV` | 7 |
| USDY | 2 | `CB3YA656OYIHU57657I5KGSBRHE5I3OZU4VFC22PYAOANFZHEWNYGAGP` | 7 |
| XAUM | 2 | `CC2RBGYNCFBCVENIDL5BFBWPH4OUZM2UA3OD2K2N54GLMWCC4KWPVAGO` | 9 |
| SolvBTC | 1 | `CBIJBDNZNF4X35BJ4FFZWCDBSCKOP5NB4PLG4SNENRMLAPYG4P5FM6VN` | 8 |
| xSolvBTC | 1 | `CAUP7NFABXE5TJRL3FKTPMWRLC7IAXYDCTHQRFSCLR5TMGKHOOQO772J` | 8 |
| AQUA | 3 | `CAUIKL3IYGMERDRUN6YSCLWVAKIFG5Q4YJHUKM4S4NJZQIA3BAS6OJPK` | 7 |
| XLMUSDC_LP | 3 | `CAVKLYY4RWFQBRA2YI5GTGGXKUKJQI3JLAHDGXMS7L5RDH6X6A47NMOZ` | 7 |
| XLMSolvBTC_LP | 3 | `CAPPL34NF6X7M3EQFIPDXZHQX2PT3EOE2LYCDORH3PF2LTRDSFGWV6NL` | 7 |
| xSolvBTCSolvBTC_LP | 3 | `CDZ4QRNY63YKJY7FLBTJUVPX3Z3U56JY5KHNHQEYI5F5MYLMC5YNRIXE` | 7 |
| CETESUSDC_LP | 3 | `CB2WUVUP2SKZVFZFNU33EZCISH65DKHQP4WBEHJ4WE3BF7QOABYGCQMS` | 7 |
| USTRYUSDC_LP | 3 | `CCLJBNY2KMUOSAW7URIZHREO5ORQREDX7NIHLFRAT6KAZ5GO7652OA4I` | 7 |
| USDYUSDC_LP | 3 | `CCYD4C2WFWIIDF235SCDXEWL5RT6S53WQWLDHUTQ5USZ5VNF5NHJGVW7` | 7 |
| XAUMUSDC_LP | 3 | `CACO4IS27ANIDC45UN5LWTYZYEELSJWA3WVWGI6QAF6Q2ZY4ZMZFSJRW` | 7 |
| PYUSDUSDC_LP | 3 | `CD3EQXQGRYUKM7LZ6MZVTXSW74ROXU6YI2SXHJ6KTB5BQ425E5KXRDBC` | 7 |
| XLMAQUA_LP | 3 | `CBOHAVUYKQD4C7FIVXEDJCVLUZYUO6RN3VIKEDOTIJGDDV3QN33Y4T4D` | 7 |
| AQUAUSDC_LP | 3 | `CDOY7ILRR7PDGLBXZUPSENB6XOET77PR2JY3HXDGQS3TS4T764OYBUGO` | 7 |

The decimals column is the oracle configuration value. The market unit is the listed decimals: the stored oracle's `asset_decimals` when the price aggregator holds an oracle for the token, otherwise the token's `decimals()` at first listing. An issuer relabel does not change it. The pool never reads decimals itself.

### Spokes

Live listing parameters: `get_spoke_asset(spoke_id, hub_asset)`.

| On-chain `spoke_id` | Name | Config key | Listed markets |
|---|---|---|---|
| 1 | Blue Chip | 1 | XLM@hub1, USDC@hub1, EURC@hub1, PYUSD@hub1, USDT0@hub1, SolvBTC@hub1, xSolvBTC@hub1 |
| 2 | Etherfuse RWA | 2 | USTRY@hub2, CETES@hub2, USDC@hub1, XLM@hub1, EURC@hub1, PYUSD@hub1 |
| 3 | Centrifuge RWA | 4 | DEJTRSY@hub2, DEJAAA@hub2, USDC@hub1, XLM@hub1, EURC@hub1, PYUSD@hub1 |
| 4 | Stables & FX | 5 | USDC@hub1, EURC@hub1, PYUSD@hub1, USDT0@hub1, USST@hub2, USDY@hub2, SPIKOEUTBL@hub2, SPIKOUSTBL@hub2, SPIKOUKTBL@hub2, SPIKOSAFO@hub2, SPIKOEURSAFO@hub2, SPIKOGBPSAFO@hub2 |
| 5 | AMM Collateral | 6 | XLMUSDC_LP@hub3, XLMSolvBTC_LP@hub3, xSolvBTCSolvBTC_LP@hub3, CETESUSDC_LP@hub3, USTRYUSDC_LP@hub3, USDYUSDC_LP@hub3, PYUSDUSDC_LP@hub3, XLMAQUA_LP@hub3, AQUAUSDC_LP@hub3, USDC@hub1, EURC@hub1, XLM@hub1, PYUSD@hub1 |
| 6 | Ondo RWA | 7 | USDY@hub2, USDYUSDC_LP@hub3, USDC@hub1, EURC@hub1, XLM@hub1, PYUSD@hub1 |
| 7 | Commodities | 8 | XAUM@hub2, XAUMUSDC_LP@hub3, USDC@hub1, EURC@hub1, XLM@hub1, PYUSD@hub1 |
| 8 | Aquarius Ecosystem | 9 | AQUA@hub3, XLMAQUA_LP@hub3, AQUAUSDC_LP@hub3, USDC@hub1, XLM@hub1, EURC@hub1, PYUSD@hub1 |

Configured but **not created on-chain** (absent from `spoke_ids` in `configs/networks.json`): config key 3 (Spiko RWA).

### Blend pools approved for `migrate_from_blend`

| Pool | Address |
|---|---|
| FixedV2 | `CAJJZSGMMM3PD7N33TAPHGBUGTB43OC73HVIK2L2G6BNGGGYOSSYBXBD` |
| YieldBloxV2 | `CCCCIQSDILITHMM7PBSLVDT5MISSY7R26MNZXCX4H7J5JQ5FPIYOGYFS` |

Blend pool factory v2: `CDSYOAVXFY7SM5S64IZPPPYB4GVGGLMQVFREPSQQEZVIWXX5R23G4QSU`. The controller is authoritative: `is_blend_pool_approved(pool)`.

## Testnet

- Network passphrase: `Test SDF Network ; September 2015`
- RPC gateway used by XOXNO clients: `https://stellar-testnet-gateway.xoxno.com`
- Quote server (swap aggregator API): `https://testnet-stellar-swap.xoxno.com`
- XOXNO API (xoxno-api-v2): `https://testnet-api.xoxno.com`
- Deployment recorded at ledger `4037973`; timelock min delay `12` ledgers

### Contracts

| Role | Address |
|---|---|
| Governance (timelock; owns controller + price aggregator) | `CDS33JDOYH3F3FL4QUQ6DV4WKHML2AKHF4LADTZ57FRUAEBTE7NMY5FQ` |
| Controller (the only user-facing lending contract) | `CCXRWJ6SIU2WPFEGLFGJVITPL57QAYIMIO6OAM2NBGNDQSSCK2FFV3F3` |
| Pool (custody + accounting; controller-only mutators) | `CBSGF6QOQAMPFBEVSYPEQHSZRIHJ6RCGUPCRDMUX36DEKRWFAO2PZB5A` |
| Position NFT (token id == account id) | `CDVN5JU675MEDPVRPCYC45AHFC275UH57WEU5OTFE4WFGZBNN7HTLPSY` |
| Price aggregator | `CAALOOTIDXCX7D7FMQIBSSAJLPKOM3GMXS4UUSTDIG42JCRRJHOPUHOP` |
| Swap aggregator router (`execute_strategy`) | `CDNTWMWW2WGYTKIZTJYNGNVQQZI4KTC5BQRZ3275KESRX5T4O3AYECL5` |
| XOXNO oracle adapter | `CDYX4ZEO556YZDYDJLUE5XQUE2DLWVFJDTBJJGF7HYQP5HK5NICNTQ6F` |
| RedStone adapter (external) | `CBIHT4HVRIT5OMVLSXZ44J2ZAXYBDDGOSCN3LTN2DOC6SWHDS5IP6BK3` |
| Reflector CEX oracle (external) | `CCYOZJCOPG34LLQQ7N24YXBM7LL62R7ONMZ3G6WZAAYPB5OYKOMJRN63` |
| Reflector DEX oracle (external) | `CAVLP5DH2GJPZMVO7IJY4CVOD5MWEFTJFVPD2YY2FQXOQHRGHK4D6HLP` |
| Reflector FX oracle (external) | `CCSSOHTBL3LEWUCBBEB5NJFC2OKFRC74OWEIJIZLRJBGAAU4VMU5NV4W` |
| Revenue accumulator (G-address) | `GDBBOILYIJBSUQKC3Z3USAW3DGPFHIGVKYA5T4ZUZBO56HBUPHJEN3FV` |

### Hubs

| On-chain `hub_id` | Name | Markets |
|---|---|---|
| 1 | Main | USDC, XLM, EURC, BTC, ETH |
| 2 | Secondary | USDC_HUB2 |
| 3 | Aquarius | XLMUSDC_LP |

### Markets (`HubAssetKey { hub_id, asset }`)

| Symbol | `hub_id` | `asset` | Decimals (oracle config) |
|---|---|---|---|
| USDC | 1 | `CBIELTK6YBZJU5UP2WWQEUCYKLPU6AUNZ2BQ4WWFEIE3USCIHMXQDAMA` | 7 |
| XLM | 1 | `CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC` | 7 |
| EURC | 1 | `CCUUDM434BMZMYWYDITHFXHDMIVTGGD6T2I5UKNX5BSLXLW7HVR4MCGZ` | 7 |
| BTC | 1 | `CBK3FNAM3C54674OSOCQLDNW4EXMNUY6ZO3C3ZI5S5DGBIIPX4GJ7WHW` | 7 |
| ETH | 1 | `CBFNIHC2B7WMAH2CKNKQJOB3CWBUXXNNRQXYISJ7VZONM77YBMHCOULJ` | 7 |
| USDC_HUB2 | 2 | `CBIELTK6YBZJU5UP2WWQEUCYKLPU6AUNZ2BQ4WWFEIE3USCIHMXQDAMA` | 7 |
| XLMUSDC_LP | 3 | `CDEUHPEUQAQNLCHFVBX3ZOSIR2FUWD2COYTSHUPQPJWK2BCLLQCW66FY` | 7 |

The decimals column is the oracle configuration value. The market unit is the listed decimals: the stored oracle's `asset_decimals` when the price aggregator holds an oracle for the token, otherwise the token's `decimals()` at first listing. An issuer relabel does not change it. The pool never reads decimals itself.

### Spokes

Live listing parameters: `get_spoke_asset(spoke_id, hub_asset)`.

| On-chain `spoke_id` | Name | Config key | Listed markets |
|---|---|---|---|
| 1 | Main (USDC/EURC/XLM/BTC) | 1 | USDC@hub1, EURC@hub1, XLM@hub1, BTC@hub1 |
| 2 | XLM + USDC | 2 | XLM@hub1, USDC@hub1 |
| 3 | Full (BTC/USDC/XLM/EURC/LP + Dual USDC) | 3 | BTC@hub1, XLM@hub1, EURC@hub1, XLMUSDC_LP@hub3, USDC@hub1, USDC_HUB2@hub2 |
| 4 | LP Tokens | 4 | XLMUSDC_LP@hub3 |

### Blend pools approved for `migrate_from_blend`

| Pool | Address |
|---|---|
| TestnetV2 | `CCEBVDYM32YNYCVNRXQKDFFPISJJCV557CDZEIRBEE4NCV4KHPQ44HGF` |

Blend pool factory v2: `CDV6RX4CGPCOKGTBFS52V3LMWQGZN3LCQTXF5RVPOOCG4XVMHXQ4NTF6`. The controller is authoritative: `is_blend_pool_approved(pool)`.
