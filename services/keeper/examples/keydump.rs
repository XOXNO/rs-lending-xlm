fn main() -> anyhow::Result<()> {
    use keeper_bot::keys::ControllerPersistentKey;
    use stellar_xdr::curr::{Limits, WriteXdr};
    let id = stellar_strkey::Contract::from_string("CDZY676UCB5OC2DCABE5CD2USZS6Z3RC47R2QBSSOBCU76EK37A5PRLM").unwrap();
    let key = ControllerPersistentKey::AccountNonce.to_ledger_key(&id.0)?;
    println!("{}", key.to_xdr_base64(Limits::none())?);
    Ok(())
}
