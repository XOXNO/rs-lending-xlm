//! Grouping of discovered ledger entries into the `(contract, group)` pairs the
//! metrics surface and the TTL inspector both report against.
//!
//! Cardinality is the reason this aggregates rather than labelling per key.
//! Account ids are never reused, so a series per ledger key would add a
//! permanent new label value for every account ever opened. Grouping holds the
//! series count flat — roughly contracts times groups — no matter how far the
//! protocol grows.

use stellar_xdr::{ContractId, Hash, LedgerKey, ScAddress, ScSymbol, ScVal};

use crate::discovery::ContractIds;
use crate::stellar::client::LedgerEntryQuery;

/// The class of protocol state an entry holds. Mirrors the sections the TTL
/// inspector prints, so its output and the dashboard agree.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum KeyClass {
    PerAsset,
    Spoke,
    PerUser,
    Roles,
    Governance,
    /// Price-aggregator `Oracle(PriceKey)` rows — the per-asset and `Ref`
    /// oracle configs.
    Oracle,
    /// Xoxno-oracle-adapter feed state: registries, aggregates, history and
    /// per-signer submissions.
    OracleFeed,
    Instance,
    WasmCode,
    Other,
}

impl KeyClass {
    /// Section heading used by the inspector.
    pub fn title(self) -> &'static str {
        match self {
            Self::PerAsset => "PER-ASSET",
            Self::Spoke => "HUB & SPOKE",
            Self::PerUser => "PER-USER",
            Self::Roles => "ROLES",
            Self::Governance => "GOVERNANCE",
            Self::Oracle => "ORACLE",
            Self::OracleFeed => "ORACLE FEED",
            Self::Instance => "INSTANCE",
            Self::WasmCode => "WASM CODE",
            Self::Other => "OTHER",
        }
    }

    /// Prometheus label value. Lower-case and stable — renaming one silently
    /// breaks every saved dashboard query and alert that selects on it.
    pub fn label(self) -> &'static str {
        match self {
            Self::PerAsset => "per_asset",
            Self::Spoke => "hub_spoke",
            Self::PerUser => "per_user",
            Self::Roles => "roles",
            Self::Governance => "governance",
            Self::Oracle => "oracle",
            Self::OracleFeed => "oracle_feed",
            Self::Instance => "instance",
            Self::WasmCode => "wasm_code",
            Self::Other => "other",
        }
    }

    /// Every class, so the metrics surface can publish a zero for groups that
    /// hold no entries this tick. Without that, a group whose entries all
    /// vanish leaves its last value on the dashboard forever.
    pub const ALL: [KeyClass; 10] = [
        Self::PerAsset,
        Self::Spoke,
        Self::PerUser,
        Self::Roles,
        Self::Governance,
        Self::Oracle,
        Self::OracleFeed,
        Self::Instance,
        Self::WasmCode,
        Self::Other,
    ];
}

/// Returns the contract an entry lives on, as a stable label. Falls back to the
/// strkey-free hex prefix so an unrecognised contract is still distinguishable
/// rather than collapsing into one bucket.
pub fn contract_label(
    key: &LedgerKey,
    ids: &ContractIds,
    pool_id: Option<&[u8; 32]>,
    position_nft_id: Option<&[u8; 32]>,
) -> String {
    let LedgerKey::ContractData(cd) = key else {
        return "wasm_code".to_string();
    };
    let ScAddress::Contract(ContractId(Hash(id))) = &cd.contract else {
        return "unknown".to_string();
    };
    let named = [
        (Some(ids.controller), "controller"),
        (ids.governance, "governance"),
        (ids.price_aggregator, "price_aggregator"),
        (ids.xoxno_oracle_adapter, "xoxno_oracle_adapter"),
        (ids.flash_receiver, "flash_loan_receiver"),
        (pool_id.copied(), "pool"),
        (position_nft_id.copied(), "position_nft"),
    ];
    for (candidate, name) in named {
        if candidate.as_ref() == Some(id) {
            return name.to_string();
        }
    }
    // Anything still unnamed keeps a distinct label rather than collapsing into
    // one bucket, so an unexpected contract is visible on the dashboard.
    format!("other_{:02x}{:02x}{:02x}{:02x}", id[0], id[1], id[2], id[3])
}

/// Classifies a persistent entry by the storage-key variant it carries.
pub fn classify_persistent(
    row: &LedgerEntryQuery,
    controller_id: &[u8; 32],
    governance_id: Option<&[u8; 32]>,
) -> KeyClass {
    let LedgerKey::ContractData(cd) = &row.key else {
        return KeyClass::Other;
    };
    let on_governance = matches!(
        &cd.contract,
        ScAddress::Contract(ContractId(Hash(b))) if Some(b) == governance_id
    );
    let on_controller = matches!(
        &cd.contract,
        ScAddress::Contract(ContractId(Hash(b))) if b == controller_id
    );
    let variant = match &cd.key {
        ScVal::Vec(Some(v)) => v.0.first().and_then(|s| match s {
            ScVal::Symbol(ScSymbol(s)) => Some(s.to_utf8_string_lossy()),
            _ => None,
        }),
        _ => None,
    };
    let role_variants = [
        "ExistingRoles",
        "RoleAccountsCount",
        "RoleAccounts",
        "HasRole",
        "RoleAdmin",
    ];
    match variant.as_deref() {
        Some(v) if role_variants.contains(&v) => {
            if on_governance {
                KeyClass::Governance
            } else {
                KeyClass::Roles
            }
        }
        Some("AccountMeta" | "SupplyPositions" | "BorrowPositions" | "Delegates")
            if on_controller =>
        {
            KeyClass::PerUser
        }
        // `Owner` lives on the position NFT, not the controller. OpenZeppelin
        // extends it by 30 days against the controller's 120, so it is the
        // entry that archives first, and an archived `Owner` makes the account
        // unusable. It belongs with the rest of the per-account state rather
        // than in `other`, which is the group nobody reads.
        Some("Owner") => KeyClass::PerUser,
        Some("Oracle") => KeyClass::Oracle,
        Some(
            "AssetCount" | "FeedCount" | "AssetAt" | "FeedAt" | "AssetIndex" | "FeedMapping"
            | "FeedOwner" | "FeedIndex" | "CurrentAggregate" | "History" | "LatestSubmission"
            | "SignerFeeds",
        ) => KeyClass::OracleFeed,
        Some("Market" | "Params" | "State") => KeyClass::PerAsset,
        Some("Hub" | "Spoke") => KeyClass::Spoke,
        _ if on_governance => KeyClass::Governance,
        _ => KeyClass::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::{AggregatorPriceKey, HubAssetKey};
    use crate::keys::{ControllerUserKey, PoolPersistentKey, PriceAggregatorPersistentKey};

    const CTRL: [u8; 32] = [1u8; 32];
    const GOV: [u8; 32] = [2u8; 32];
    const AGG: [u8; 32] = [3u8; 32];

    fn row(key: LedgerKey) -> LedgerEntryQuery {
        LedgerEntryQuery {
            key,
            value: None,
            live_until_ledger: None,
        }
    }

    fn ids() -> ContractIds {
        ContractIds {
            controller: CTRL,
            pool_wasm_hash: [0u8; 32],
            flash_receiver: None,
            governance: Some(GOV),
            xoxno_oracle_adapter: None,
            price_aggregator: Some(AGG),
        }
    }

    #[test]
    fn account_keys_group_as_per_user() {
        let r = row(ControllerUserKey::AccountMeta(1)
            .to_ledger_key(&CTRL)
            .unwrap());
        assert_eq!(
            classify_persistent(&r, &CTRL, Some(&GOV)),
            KeyClass::PerUser
        );
    }

    /// The same variant name on a different contract must not be read as a
    /// controller account key; grouping keys by name alone would mislabel it.
    #[test]
    fn pool_market_keys_group_as_per_asset() {
        let hub_asset = HubAssetKey {
            hub_id: 1,
            asset: [7u8; 32],
        };
        let r = row(PoolPersistentKey::Params(hub_asset)
            .to_ledger_key(&[9u8; 32])
            .unwrap());
        assert_eq!(
            classify_persistent(&r, &CTRL, Some(&GOV)),
            KeyClass::PerAsset
        );
    }

    /// Role variants exist on both the controller and governance; the contract
    /// they sit on is what separates them.
    #[test]
    fn role_keys_split_by_owning_contract() {
        use crate::keys::AccessControlPersistentKey;
        let on_gov = row(AccessControlPersistentKey::ExistingRoles
            .to_ledger_key(&GOV)
            .unwrap());
        let on_ctrl = row(AccessControlPersistentKey::ExistingRoles
            .to_ledger_key(&CTRL)
            .unwrap());
        assert_eq!(
            classify_persistent(&on_gov, &CTRL, Some(&GOV)),
            KeyClass::Governance
        );
        assert_eq!(
            classify_persistent(&on_ctrl, &CTRL, Some(&GOV)),
            KeyClass::Roles
        );
    }

    /// `Owner` is the shortest-lived entry in the protocol and an archived one
    /// makes an account unusable, so it must group with the other per-account
    /// state and not disappear into `other`.
    #[test]
    fn position_nft_owner_groups_as_per_user() {
        use crate::keys::PositionNftUserKey;
        let nft = [0xCCu8; 32];
        let r = row(PositionNftUserKey::Owner(1).to_ledger_key(&nft).unwrap());
        assert_eq!(
            classify_persistent(&r, &CTRL, Some(&GOV)),
            KeyClass::PerUser
        );
    }

    #[test]
    fn hub_and_spoke_share_a_group() {
        use crate::keys::ControllerPersistentKey;
        for k in [
            ControllerPersistentKey::Hub(1),
            ControllerPersistentKey::Spoke(1),
        ] {
            let r = row(k.to_ledger_key(&CTRL).unwrap());
            assert_eq!(classify_persistent(&r, &CTRL, Some(&GOV)), KeyClass::Spoke);
        }
    }

    #[test]
    fn contract_labels_resolve_configured_ids() {
        let agg_key = PriceAggregatorPersistentKey::Oracle(AggregatorPriceKey::Ref("BTC".into()))
            .to_ledger_key(&AGG)
            .unwrap();
        assert_eq!(
            contract_label(&agg_key, &ids(), None, None),
            "price_aggregator"
        );
    }

    /// An unconfigured contract (pool and position-NFT are resolved at runtime)
    /// must stay distinguishable rather than collapsing into one bucket.
    /// Pool and position-NFT are read from the controller instance, not config.
    /// Without threading them through they render as an opaque hex prefix, which
    /// is unreadable on a per-contract dashboard.
    #[test]
    fn runtime_resolved_contracts_are_named() {
        let pool = [0xEEu8; 32];
        let key = PoolPersistentKey::Params(HubAssetKey {
            hub_id: 1,
            asset: [7u8; 32],
        })
        .to_ledger_key(&pool)
        .unwrap();
        assert_eq!(contract_label(&key, &ids(), Some(&pool), None), "pool");
        assert!(contract_label(&key, &ids(), None, None).starts_with("other_"));
    }

    #[test]
    fn unknown_contracts_keep_a_distinct_label() {
        let a = row(ControllerUserKey::AccountMeta(1)
            .to_ledger_key(&[0xAAu8; 32])
            .unwrap());
        let b = row(ControllerUserKey::AccountMeta(1)
            .to_ledger_key(&[0xBBu8; 32])
            .unwrap());
        let la = contract_label(&a.key, &ids(), None, None);
        let lb = contract_label(&b.key, &ids(), None, None);
        assert_ne!(la, lb);
        assert!(la.starts_with("other_"));
    }
}
