//! Minimal ScVal readers.
//!
//! Structs → `Map` by field symbol; enums → `Vec[Symbol(tag), payload...]`.
//! Returns `Option` so shape mismatch is "missing", not a panic.

use stellar_xdr::curr::{ScAddress, ScVal};

pub fn map_field<'a>(value: &'a ScVal, name: &str) -> Option<&'a ScVal> {
    let ScVal::Map(Some(map)) = value else {
        return None;
    };
    map.0
        .iter()
        .find(|entry| symbol_text(&entry.key).as_deref() == Some(name))
        .map(|entry| &entry.val)
}

pub fn field_i128(value: &ScVal, name: &str) -> Option<i128> {
    map_field(value, name).and_then(as_i128)
}

pub fn field_u64(value: &ScVal, name: &str) -> Option<u64> {
    map_field(value, name).and_then(as_u64)
}

pub fn field_u32(value: &ScVal, name: &str) -> Option<u32> {
    map_field(value, name).and_then(as_u32)
}

pub fn field_bool(value: &ScVal, name: &str) -> Option<bool> {
    map_field(value, name).and_then(as_bool)
}

pub fn as_i128(value: &ScVal) -> Option<i128> {
    match value {
        ScVal::I128(parts) => Some((i128::from(parts.hi) << 64) | i128::from(parts.lo)),
        ScVal::U128(parts) => Some((i128::from(parts.hi) << 64) | i128::from(parts.lo)),
        ScVal::I64(v) => Some(i128::from(*v)),
        ScVal::U64(v) => Some(i128::from(*v)),
        ScVal::I32(v) => Some(i128::from(*v)),
        ScVal::U32(v) => Some(i128::from(*v)),
        _ => None,
    }
}

pub fn as_u64(value: &ScVal) -> Option<u64> {
    match value {
        ScVal::U64(v) => Some(*v),
        ScVal::U32(v) => Some(u64::from(*v)),
        ScVal::I64(v) if *v >= 0 => Some(*v as u64),
        _ => None,
    }
}

pub fn as_u32(value: &ScVal) -> Option<u32> {
    match value {
        ScVal::U32(v) => Some(*v),
        _ => None,
    }
}

pub fn as_bool(value: &ScVal) -> Option<bool> {
    match value {
        ScVal::Bool(v) => Some(*v),
        _ => None,
    }
}

pub fn symbol_text(value: &ScVal) -> Option<String> {
    match value {
        ScVal::Symbol(s) => Some(s.0.to_utf8_string_lossy()),
        _ => None,
    }
}

pub fn string_text(value: &ScVal) -> Option<String> {
    match value {
        ScVal::String(s) => Some(s.0.to_utf8_string_lossy()),
        _ => None,
    }
}

pub fn as_address(value: &ScVal) -> Option<ScAddress> {
    match value {
        ScVal::Address(a) => Some(a.clone()),
        _ => None,
    }
}

pub fn as_contract_id(value: &ScVal) -> Option<[u8; 32]> {
    match value {
        ScVal::Address(ScAddress::Contract(c)) => Some(c.0 .0),
        _ => None,
    }
}

pub fn address_strkey(value: &ScVal) -> Option<String> {
    // Display → std::String (inherent to_string is heapless).
    match value {
        ScVal::Address(ScAddress::Contract(c)) => Some(format!("{}", stellar_strkey::Contract(c.0 .0))),
        ScVal::Address(ScAddress::Account(a)) => {
            let stellar_xdr::curr::PublicKey::PublicKeyTypeEd25519(k) = &a.0;
            Some(format!("{}", stellar_strkey::ed25519::PublicKey(k.0)))
        }
        _ => None,
    }
}

pub fn enum_variant(value: &ScVal) -> Option<(String, &[ScVal])> {
    let ScVal::Vec(Some(items)) = value else {
        return None;
    };
    let (head, rest) = items.0.split_first()?;
    let tag = symbol_text(head)?;
    Some((tag, rest))
}

pub fn vec_items(value: &ScVal) -> Option<&[ScVal]> {
    match value {
        ScVal::Vec(Some(items)) => Some(items.0.as_slice()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use stellar_xdr::curr::{Int128Parts, ScMap, ScMapEntry, ScVec};

    fn sym(text: &str) -> ScVal {
        ScVal::Symbol(crate::keys::symbol(text).unwrap())
    }

    fn i128_val(v: i128) -> ScVal {
        ScVal::I128(Int128Parts {
            hi: (v >> 64) as i64,
            lo: v as u64,
        })
    }

    #[test]
    fn decodes_i128_roundtrip_including_negative() {
        assert_eq!(as_i128(&i128_val(0)), Some(0));
        assert_eq!(as_i128(&i128_val(1_000_000_000_000_000_000)), Some(1e18 as i128));
        assert_eq!(as_i128(&i128_val(-42)), Some(-42));
        assert_eq!(as_i128(&i128_val(i128::MAX)), Some(i128::MAX));
    }

    #[test]
    fn reads_struct_field_by_symbol() {
        let map = ScVal::Map(Some(ScMap(
            vec![
                ScMapEntry {
                    key: sym("cash"),
                    val: i128_val(500),
                },
                ScMapEntry {
                    key: sym("last_timestamp"),
                    val: ScVal::U64(1_700_000_000),
                },
            ]
            .try_into()
            .unwrap(),
        )));
        assert_eq!(field_i128(&map, "cash"), Some(500));
        assert_eq!(field_u64(&map, "last_timestamp"), Some(1_700_000_000));
        assert_eq!(field_i128(&map, "missing"), None);
    }

    #[test]
    fn splits_enum_variant_tag_and_payload() {
        let v = ScVal::Vec(Some(ScVec(
            vec![sym("Reflector"), ScVal::U32(3)].try_into().unwrap(),
        )));
        let (tag, payload) = enum_variant(&v).unwrap();
        assert_eq!(tag, "Reflector");
        assert_eq!(payload.len(), 1);
        assert!(matches!(payload[0], ScVal::U32(3)));
    }
}
