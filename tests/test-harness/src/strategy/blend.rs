use soroban_sdk::{Address, Vec};

use crate::core::{AccountEntry, LendingTest};
use crate::helpers::{f64_to_i128, HARNESS_HUB, HARNESS_SPOKE};
use crate::mock_blend::{MockBlend, MockBlendClient, KIND_LIABILITY};

impl LendingTest {
    /// Registers a MockBlend pool and governance-approves it, reusing the
    /// same address for the rest of this world's lifetime.
    pub fn ensure_approved_blend(&mut self) -> Address {
        if let Some(addr) = self.blend_pool.clone() {
            return addr;
        }
        let addr = self.env.register(MockBlend, ());
        let admin = self.admin();
        self.gov_client().execute_immediate(
            &admin,
            &governance_interface::AdminOperation::ApproveBlendPool(addr.clone()),
        );
        self.blend_pool = Some(addr.clone());
        addr
    }

    /// Overwrites `user`'s mock Blend slot for `asset`/`kind`. Collateral and
    /// supply also mint that raw amount onto the mock so a later sweep can pay.
    pub fn seed_blend(&mut self, user: &str, asset: &str, kind: u32, amount: f64) {
        let blend = self.ensure_approved_blend();
        let caller = self.get_or_create_user(user);
        let market = self.resolve_market(asset);
        let raw = f64_to_i128(amount, market.decimals);
        MockBlendClient::new(&self.env, &blend).seed(&caller, &market.asset, &kind, &raw);
        if kind != KIND_LIABILITY {
            market.token_admin.mint(&blend, &raw);
        }
    }

    pub fn blend_position(&self, user: &str, asset: &str, kind: u32) -> i128 {
        let blend = self
            .blend_pool
            .as_ref()
            .expect("ensure_approved_blend before blend_position");
        let caller = self
            .users
            .get(user)
            .unwrap_or_else(|| panic!("user '{user}' not found"))
            .address
            .clone();
        let asset_addr = self.resolve_asset(asset);
        MockBlendClient::new(&self.env, blend).position(&caller, &asset_addr, &kind)
    }

    /// Migrates `user`'s mock Blend position into `account_id` (0 creates).
    /// Registers a newly minted account only when it still exists after
    /// finalize (empty Blend listings can mint-then-cleanup).
    pub fn try_migrate_from_blend(
        &mut self,
        user: &str,
        account_id: u64,
        collateral: &[&str],
        supply: &[&str],
        debt_caps: &[(&str, f64)],
    ) -> Result<u64, soroban_sdk::Error> {
        let blend = self.ensure_approved_blend();
        let caller = self.get_or_create_user(user);
        let mut coll = Vec::new(&self.env);
        for name in collateral {
            coll.push_back(self.resolve_asset(name));
        }
        let mut supp = Vec::new(&self.env);
        for name in supply {
            supp.push_back(self.resolve_asset(name));
        }
        let mut debt = Vec::new(&self.env);
        for (name, cap) in debt_caps {
            let decimals = self.resolve_market(name).decimals;
            debt.push_back((self.resolve_asset(name), f64_to_i128(*cap, decimals)));
        }
        let ctrl = self.ctrl_client();
        match ctrl.try_migrate_from_blend(
            &caller,
            &account_id,
            &HARNESS_SPOKE,
            &HARNESS_HUB,
            &blend,
            &coll,
            &supp,
            &debt,
        ) {
            Ok(Ok(id)) => {
                if account_id == 0 && ctrl.account_exists(&id) {
                    let attrs = ctrl.get_account_attributes(&id);
                    let user_state = self.users.get_mut(user).expect("user exists");
                    user_state.accounts.push(AccountEntry {
                        account_id: id,
                        spoke_id: attrs.spoke_id,
                        mode: attrs.mode,
                    });
                    if user_state.default_account_id.is_none() {
                        user_state.default_account_id = Some(id);
                    }
                }
                Ok(id)
            }
            Ok(Err(err)) => Err(err),
            Err(e) => Err(e.expect("expected contract error, got InvokeError")),
        }
    }
}
