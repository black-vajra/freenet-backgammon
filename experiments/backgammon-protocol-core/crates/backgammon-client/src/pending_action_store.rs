use crate::pending_action::{PendingAction, MAX_CONTRACT_ID_BYTES, MAX_PENDING_DELTA_BYTES};

const STORAGE_PREFIX: &str = "freenet-backgammon.pending-action.v1";

/*
 * The pending record contains metadata in addition to the action delta.
 * This upper bound prevents hostile or corrupted localStorage content from
 * causing an unbounded allocation before canonical decoding.
 */
const MAX_PENDING_RECORD_BYTES: usize = MAX_PENDING_DELTA_BYTES + MAX_CONTRACT_ID_BYTES + 1024;

pub fn pending_action_storage_key(contract_id: &str) -> Result<String, String> {
    validate_contract_id(contract_id)?;

    Ok(format!("{STORAGE_PREFIX}.{contract_id}"))
}

#[cfg(target_arch = "wasm32")]
pub fn store_pending_action(pending: &PendingAction) -> Result<(), String> {
    pending.verify()?;

    let storage = browser_storage()?;
    let key = pending_action_storage_key(&pending.contract_id)?;
    let encoded = pending.encode()?;

    if encoded.len() > MAX_PENDING_RECORD_BYTES {
        return Err(format!(
            "Pending-action record exceeds {} bytes.",
            MAX_PENDING_RECORD_BYTES,
        ));
    }

    let encoded_hex = encode_hex(&encoded);

    storage
        .set_item(&key, &encoded_hex)
        .map_err(|error| format!("Could not persist the pending action: {error:?}"))?;

    /*
     * Read the value back immediately. A submit must never begin unless the
     * exact retry unit survived browser storage byte-for-byte.
     */
    let persisted_hex = storage
        .get_item(&key)
        .map_err(|error| format!("Could not verify the persisted pending action: {error:?}"))?
        .ok_or_else(|| "Browser storage did not retain the pending action.".to_owned())?;

    let persisted_bytes = decode_hex_bounded(&persisted_hex, MAX_PENDING_RECORD_BYTES)?;

    let persisted = PendingAction::decode(&persisted_bytes)?;

    if persisted != *pending
        || persisted_bytes != encoded
        || encode_hex(&persisted_bytes) != persisted_hex
    {
        let _ = storage.remove_item(&key);

        return Err("Persisted pending action failed exact round-trip verification.".to_owned());
    }

    Ok(())
}

#[cfg(target_arch = "wasm32")]
pub fn load_pending_action(contract_id: &str) -> Result<Option<PendingAction>, String> {
    let storage = browser_storage()?;
    let key = pending_action_storage_key(contract_id)?;

    let Some(encoded_hex) = storage
        .get_item(&key)
        .map_err(|error| format!("Could not read the pending action: {error:?}"))?
    else {
        return Ok(None);
    };

    let encoded = decode_hex_bounded(&encoded_hex, MAX_PENDING_RECORD_BYTES)?;

    let pending = PendingAction::decode(&encoded)?;

    if pending.contract_id != contract_id {
        return Err("Stored pending action belongs to a different contract.".to_owned());
    }

    if encode_hex(&encoded) != encoded_hex {
        return Err("Stored pending action is not canonically encoded.".to_owned());
    }

    Ok(Some(pending))
}

#[cfg(target_arch = "wasm32")]
pub fn remove_pending_action(contract_id: &str) -> Result<(), String> {
    let storage = browser_storage()?;
    let key = pending_action_storage_key(contract_id)?;

    storage
        .remove_item(&key)
        .map_err(|error| format!("Could not remove the pending action: {error:?}"))?;

    /*
     * Confirm removal. Accepted actions must not remain available for an
     * accidental retry after cleanup reports success.
     */
    if storage
        .get_item(&key)
        .map_err(|error| format!("Could not verify pending-action removal: {error:?}"))?
        .is_some()
    {
        return Err("Browser storage retained the pending action after removal.".to_owned());
    }

    Ok(())
}

fn validate_contract_id(contract_id: &str) -> Result<(), String> {
    if contract_id.is_empty() {
        return Err("Pending-action contract ID is empty.".to_owned());
    }

    if contract_id.len() > MAX_CONTRACT_ID_BYTES {
        return Err(format!(
            "Pending-action contract ID exceeds {} bytes.",
            MAX_CONTRACT_ID_BYTES,
        ));
    }

    if !contract_id
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err("Pending-action contract ID contains unsupported characters.".to_owned());
    }

    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn browser_storage() -> Result<web_sys::Storage, String> {
    let window = web_sys::window().ok_or_else(|| "Browser window is unavailable.".to_owned())?;

    window
        .local_storage()
        .map_err(|error| format!("Browser storage is unavailable: {error:?}"))?
        .ok_or_else(|| "Browser local storage is disabled.".to_owned())
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";

    let mut encoded = String::with_capacity(bytes.len() * 2);

    for byte in bytes {
        encoded.push(HEX[usize::from(byte >> 4)] as char);
        encoded.push(HEX[usize::from(byte & 0x0f)] as char);
    }

    encoded
}

fn decode_hex_bounded(encoded: &str, maximum_bytes: usize) -> Result<Vec<u8>, String> {
    if encoded.len() % 2 != 0 {
        return Err("Stored pending action has an odd hexadecimal length.".to_owned());
    }

    let decoded_len = encoded.len() / 2;

    if decoded_len > maximum_bytes {
        return Err(format!(
            "Stored pending action exceeds {} decoded bytes.",
            maximum_bytes,
        ));
    }

    let bytes = encoded.as_bytes();
    let mut decoded = Vec::with_capacity(decoded_len);

    for pair in bytes.chunks_exact(2) {
        let high = decode_hex_nibble(pair[0])?;
        let low = decode_hex_nibble(pair[1])?;

        decoded.push((high << 4) | low);
    }

    Ok(decoded)
}

fn decode_hex_nibble(value: u8) -> Result<u8, String> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        _ => Err("Stored pending action contains noncanonical hexadecimal.".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use backgammon_core::Player;
    use backgammon_protocol::{ActionId, GameActionPayload};

    use crate::test_support::build_encoded_action_delta;

    fn one_action_state() -> &'static [u8] {
        crate::test_support::one_action_state()
    }

    fn pending_action(action_id: ActionId) -> PendingAction {
        let (record, delta) = build_encoded_action_delta(
            one_action_state(),
            action_id,
            GameActionPayload::Resign {
                player: Player::White,
            },
        )
        .unwrap();

        PendingAction::new(
            "5fyAKtPnwDEPdT3Ey9qryJTRZ7E6ztofRPxDHRtbL1S5",
            &record,
            delta,
        )
        .unwrap()
    }

    #[test]
    fn storage_keys_are_contract_bound() {
        let first = pending_action_storage_key("contract-one").unwrap();
        let second = pending_action_storage_key("contract-two").unwrap();

        assert_ne!(first, second);
        assert!(first.starts_with(STORAGE_PREFIX));
        assert!(second.starts_with(STORAGE_PREFIX));
    }

    #[test]
    fn invalid_contract_ids_are_rejected() {
        assert!(pending_action_storage_key("").is_err());
        assert!(pending_action_storage_key("contains space").is_err());
        assert!(pending_action_storage_key("contains/slash").is_err());

        assert!(pending_action_storage_key(&"a".repeat(MAX_CONTRACT_ID_BYTES + 1),).is_err());
    }

    #[test]
    fn canonical_pending_record_hex_round_trips() {
        let pending = pending_action([42_u8; 32]);
        let encoded = pending.encode().unwrap();
        let encoded_hex = encode_hex(&encoded);

        let decoded = decode_hex_bounded(&encoded_hex, MAX_PENDING_RECORD_BYTES).unwrap();

        assert_eq!(decoded, encoded);
        assert_eq!(PendingAction::decode(&decoded).unwrap(), pending,);
        assert_eq!(encode_hex(&decoded), encoded_hex);
    }

    #[test]
    fn malformed_or_noncanonical_hex_is_rejected() {
        assert!(decode_hex_bounded("0", 16).is_err());
        assert!(decode_hex_bounded("GG", 16).is_err());
        assert!(decode_hex_bounded("AA", 16).is_err());
        assert!(decode_hex_bounded("0/", 16).is_err());
    }

    #[test]
    fn oversized_stored_record_is_rejected_before_allocation() {
        let encoded = "00".repeat(MAX_PENDING_RECORD_BYTES + 1);

        assert!(decode_hex_bounded(&encoded, MAX_PENDING_RECORD_BYTES,).is_err());
    }

    #[test]
    fn decoded_record_must_still_pass_pending_validation() {
        let pending = pending_action([42_u8; 32]);
        let mut encoded = pending.encode().unwrap();

        encoded.push(0);

        assert!(PendingAction::decode(&encoded).is_err());
    }
}
