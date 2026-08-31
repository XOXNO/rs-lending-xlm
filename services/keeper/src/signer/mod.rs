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
    // The shared `oracle-key` KeyVault entry stores the phrase comma-separated.
    // bip39 splits on whitespace, so a comma makes the whole phrase one unknown
    // word. mx-bridge normalises the same way in its Stellar signer.
    let normalized = mnemonic.replace(',', " ");
    let mn = bip39::Mnemonic::parse_normalized(normalized.trim())
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

    #[test]
    fn accepts_a_comma_separated_phrase() {
        // Same SEP-0005 vector, stored the way the shared KeyVault entry holds it.
        let signer = signer_from_mnemonic(
            "illness,spike,retreat,truth,genius,clock,brain,pass,fit,cave,bargain,toe",
            "m/44'/148'/0'",
        )
        .unwrap();
        assert_eq!(
            signer.public_key_strkey(),
            "GDRXE2BQUC3AZNPVFSCEZ76NJ3WWL25FYFK6RGZGIEKWE4SOOHSUJUJ6"
        );
    }

    #[test]
    fn index_one_is_a_distinct_account() {
        // SEP-0005 test vector 1, m/44'/148'/1' — the index the keeper uses so it
        // never shares a sequence number with the mx-bridge relayer at index 0.
        let signer = signer_from_mnemonic(
            "illness spike retreat truth genius clock brain pass fit cave bargain toe",
            "m/44'/148'/1'",
        )
        .unwrap();
        assert_eq!(
            signer.public_key_strkey(),
            "GBAW5XGWORWVFE2XTJYDTLDHXTY2Q2MO73HYCGB3XMFMQ562Q2W2GJQX"
        );
    }
}
