# 0015. Caps are literal asset limits

**Status:** Accepted

**Implemented by:** contracts/controller/src/spoke_usage.rs (`apply_entry`, `apply_exit`, `enforce_spoke_cap`, `SpokeSupplyCapReached`), common/src/rates/scaling.rs (`calculate_scaled_cap`), contracts/controller/src/config/asset.rs (`load_market_and_validate_caps`, `remove_asset_from_spoke`), contracts/controller/src/positions/liquidation/apply.rs (`assert_credit_usage_is_neutral`).

## Decision

Supply and borrow caps are configured in native asset units and converted to
scaled shares at the live index when exposure grows. A cap of zero permits no
new exposure; it is not an unlimited sentinel. Exits reduce usage and are
never blocked by the cap.

## Guarantees

- Governance configures a human-scale value, independent of share precision.
- Interest-index changes cannot make a configured cap ambiguous.
- Cap checks cannot trap a user attempting to reduce exposure.

## Auditor focus

Test zero, boundary, index-change, multi-leg, and exit cases. Check that every
path that grows a spoke's exposure consumes the same usage accounting and that
no exit underflows it.

Liquidation's share credit is the one exemption. It moves shares between two
accounts of the same spoke, so it calls neither `apply_entry` nor `apply_exit`
for the account-to-account half; `assert_credit_usage_is_neutral` asserts that
the debit and the credit cancel instead. Only the protocol fee books a real
`apply_exit`. The credit is deliberately outside the cap check so that a spoke
sitting at its supply cap still stays liquidatable. See
[ADR-0019](0019-share-credit-liquidation.md).
