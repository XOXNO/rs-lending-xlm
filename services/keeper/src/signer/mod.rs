//! Single-key Ed25519 signer used to authorize all keeper transactions.

pub mod mnemonic;
pub mod vault;

use anyhow::{anyhow, Result};
use ed25519_dalek::{Signer as DalekSigner, SigningKey, VerifyingKey};
use stellar_strkey::ed25519::PublicKey as StrKeyPublicKey;

#[derive(Clone)]
pub struct Ed25519Signer {
    signing: SigningKey,
    verifying: VerifyingKey,
}

impl std::fmt::Debug for Ed25519Signer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ed25519Signer")
            .field("public_key", &self.public_key_strkey())
            .finish()
    }
}

impl Ed25519Signer {
    pub fn from_seed_bytes(secret: [u8; 32]) -> Self {
        let signing = SigningKey::from_bytes(&secret);
        let verifying = signing.verifying_key();
        Self { signing, verifying }
    }

    pub fn public_key_bytes(&self) -> [u8; 32] {
        self.verifying.to_bytes()
    }

    pub fn public_key_strkey(&self) -> String {
        // Display → std::String (inherent to_string is heapless).
        format!("{}", StrKeyPublicKey(self.public_key_bytes()))
    }

    pub fn sign(&self, tx_hash: &[u8; 32]) -> [u8; 64] {
        self.signing.sign(tx_hash).to_bytes()
    }

    pub fn signature_hint(&self) -> [u8; 4] {
        let pk = self.public_key_bytes();
        [pk[28], pk[29], pk[30], pk[31]]
    }
}

pub fn signer_from_mnemonic(mnemonic: &str, derivation_path: &str) -> Result<Ed25519Signer> {
    let mn = bip39::Mnemonic::parse_normalized(mnemonic.trim())
        .map_err(|e| anyhow!("invalid BIP-39 mnemonic: {e}"))?;
    let seed = mn.to_seed("");
    let secret = mnemonic::derive_ed25519(&seed, derivation_path)?;
    Ok(Ed25519Signer::from_seed_bytes(secret))
}

#[cfg(test)]
mod tests {
    use super::signer_from_mnemonic;

    #[test]
    fn derives_sep5_test_vector() {
        let signer = signer_from_mnemonic(
            "illness spike retreat truth genius clock brain pass fit cave bargain toe",
            "m/44'/148'/0'",
        )
        .unwrap();
        assert_eq!(
            signer.public_key_strkey(),
            "GDRXE2BQUC3AZNPVFSCEZ76NJ3WWL25FYFK6RGZGIEKWE4SOOHSUJUJ6"
        );
    }
}
