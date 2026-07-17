//! Mnemonic resolution from Azure Key Vault via mx-keyvault.

use anyhow::{anyhow, Context, Result};
use mx_keyvault::KeyVaultClient;
use tracing::info;

use super::{signer_from_mnemonic, Ed25519Signer};
use crate::config::{KeyVaultConfig, SignerConfig};

pub async fn load_signer(
    vault: &KeyVaultConfig,
    signer_cfg: &SignerConfig,
) -> Result<Ed25519Signer> {
    let client = KeyVaultClient::new(&vault.url)
        .with_context(|| format!("create KeyVault client for {}", vault.url))?;

    let mnemonic = client
        .fetch_secret(&vault.secret_name)
        .await
        .with_context(|| format!("fetch secret {} from KeyVault", vault.secret_name))?
        .ok_or_else(|| {
            anyhow!(
                "KeyVault secret {} is empty; cannot derive signer",
                vault.secret_name
            )
        })?;

    let signer = signer_from_mnemonic(&mnemonic, &signer_cfg.derivation_path)?;
    info!(
        target: "keeper.signer",
        public_key = %signer.public_key_strkey(),
        derivation = %signer_cfg.derivation_path,
        "keeper signer loaded from KeyVault"
    );
    Ok(signer)
}
