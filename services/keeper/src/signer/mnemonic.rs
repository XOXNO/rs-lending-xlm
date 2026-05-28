//! SLIP-0010 Ed25519 hardened-only key derivation.
//!
//! Stellar follows SEP-0005 which mandates the path `m/44'/148'/account'`,
//! all segments hardened. Hand-rolled here so the keeper avoids a thin
//! third-party wrapper for ~30 lines of well-known HMAC chaining.

use anyhow::{anyhow, bail, Result};
use hmac::{Hmac, Mac};
use sha2::Sha512;

type HmacSha512 = Hmac<Sha512>;

const HARDENED_OFFSET: u32 = 0x8000_0000;
const SLIP10_ED25519_KEY: &[u8] = b"ed25519 seed";

/// Derive a 32-byte Ed25519 secret key from the BIP-39 seed and a BIP-32-style
/// hardened path like `m/44'/148'/0'`.
pub fn derive_ed25519(seed: &[u8], path: &str) -> Result<[u8; 32]> {
    let (mut key, mut chain_code) = master_key(seed)?;

    for index in parse_path(path)? {
        let (next_key, next_chain) = ckd_priv(&key, &chain_code, index)?;
        key = next_key;
        chain_code = next_chain;
    }
    Ok(key)
}

fn master_key(seed: &[u8]) -> Result<([u8; 32], [u8; 32])> {
    let mut mac =
        HmacSha512::new_from_slice(SLIP10_ED25519_KEY).map_err(|e| anyhow!("hmac init: {e}"))?;
    mac.update(seed);
    split(mac.finalize().into_bytes().as_slice())
}

fn ckd_priv(parent_key: &[u8; 32], chain_code: &[u8; 32], index: u32) -> Result<([u8; 32], [u8; 32])> {
    if index < HARDENED_OFFSET {
        bail!("SLIP-0010 ed25519 supports hardened derivation only (index {index} < 2^31)");
    }
    let mut data = [0u8; 1 + 32 + 4];
    data[0] = 0x00;
    data[1..33].copy_from_slice(parent_key);
    data[33..].copy_from_slice(&index.to_be_bytes());

    let mut mac =
        HmacSha512::new_from_slice(chain_code).map_err(|e| anyhow!("hmac init: {e}"))?;
    mac.update(&data);
    split(mac.finalize().into_bytes().as_slice())
}

fn split(bytes: &[u8]) -> Result<([u8; 32], [u8; 32])> {
    if bytes.len() != 64 {
        bail!("HMAC-SHA512 must return 64 bytes, got {}", bytes.len());
    }
    let mut key = [0u8; 32];
    let mut chain = [0u8; 32];
    key.copy_from_slice(&bytes[..32]);
    chain.copy_from_slice(&bytes[32..]);
    Ok((key, chain))
}

fn parse_path(path: &str) -> Result<Vec<u32>> {
    let trimmed = path.trim();
    if !trimmed.starts_with("m/") && trimmed != "m" {
        bail!("derivation path must start with 'm/' (got {trimmed:?})");
    }
    if trimmed == "m" {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for raw in trimmed[2..].split('/') {
        if raw.is_empty() {
            bail!("empty segment in derivation path {trimmed:?}");
        }
        let (digits, hardened) = if let Some(stripped) = raw.strip_suffix('\'') {
            (stripped, true)
        } else if let Some(stripped) = raw.strip_suffix('h') {
            (stripped, true)
        } else {
            (raw, false)
        };
        let n: u32 = digits
            .parse()
            .map_err(|_| anyhow!("non-numeric path segment {raw:?}"))?;
        if !hardened {
            bail!("SEP-0005 path {trimmed:?} requires all segments hardened (segment {raw} is not)");
        }
        let with_hardened = n
            .checked_add(HARDENED_OFFSET)
            .ok_or_else(|| anyhow!("derivation index {n} overflows u32 when hardened"))?;
        out.push(with_hardened);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Test vector from SEP-0005 appendix.
    // mnemonic = "illness spike retreat truth genius clock brain pass fit cave bargain toe"
    // path = m/44'/148'/0'
    // expected pubkey = GDRXE2BQUC3AZNPVFSCEZ76NJ3WWL25FYFK6RGZGIEKWE4SOOHSUJUJ6
    // (We don't import the full bip39 wordlist test here; the parse_path tests
    //  cover the format machinery. Integration with bip39 + ed25519 is exercised
    //  by signer::tests in a smoke test below.)

    #[test]
    fn parses_canonical_sep5_path() {
        let path = parse_path("m/44'/148'/0'").unwrap();
        assert_eq!(path, vec![44 | HARDENED_OFFSET, 148 | HARDENED_OFFSET, HARDENED_OFFSET]);
    }

    #[test]
    fn rejects_non_hardened_segment() {
        assert!(parse_path("m/44'/148/0'").is_err());
    }

    #[test]
    fn rejects_invalid_prefix() {
        assert!(parse_path("44'/148'/0'").is_err());
    }

    #[test]
    fn derives_deterministically() {
        let seed = [0x42u8; 64];
        let a = derive_ed25519(&seed, "m/44'/148'/0'").unwrap();
        let b = derive_ed25519(&seed, "m/44'/148'/0'").unwrap();
        assert_eq!(a, b);
        let c = derive_ed25519(&seed, "m/44'/148'/1'").unwrap();
        assert_ne!(a, c, "different account index must derive different key");
    }
}
