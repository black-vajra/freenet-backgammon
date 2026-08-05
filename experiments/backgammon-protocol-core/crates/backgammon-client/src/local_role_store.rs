use backgammon_core::Player;

const STORAGE_PREFIX: &str = "freenet-backgammon.local-role.v1";
const MAX_CONTRACT_ID_BYTES: usize = 128;

pub fn local_role_storage_key(contract_id: &str) -> Result<String, String> {
    validate_contract_id(contract_id)?;

    Ok(format!("{STORAGE_PREFIX}.{contract_id}"))
}

pub fn encode_local_role(player: Player) -> &'static str {
    match player {
        Player::White => "white",
        Player::Black => "black",
    }
}

pub fn decode_local_role(encoded: &str) -> Result<Player, String> {
    match encoded {
        "white" => Ok(Player::White),
        "black" => Ok(Player::Black),
        _ => Err("Stored local role is invalid or noncanonical.".to_owned()),
    }
}

#[cfg(target_arch = "wasm32")]
pub fn store_local_role(contract_id: &str, player: Player) -> Result<(), String> {
    let storage = browser_storage()?;
    let key = local_role_storage_key(contract_id)?;
    let encoded = encode_local_role(player);

    storage
        .set_item(&key, encoded)
        .map_err(|error| format!("Could not persist the local player role: {error:?}"))?;

    let persisted = storage
        .get_item(&key)
        .map_err(|error| format!("Could not verify the persisted local role: {error:?}"))?
        .ok_or_else(|| "Browser storage did not retain the local role.".to_owned())?;

    let decoded = decode_local_role(&persisted)?;

    if decoded != player || encode_local_role(decoded) != persisted {
        let _ = storage.remove_item(&key);

        return Err("Persisted local role failed exact round-trip verification.".to_owned());
    }

    Ok(())
}

#[cfg(target_arch = "wasm32")]
pub fn load_local_role(contract_id: &str) -> Result<Option<Player>, String> {
    let storage = browser_storage()?;
    let key = local_role_storage_key(contract_id)?;

    let Some(encoded) = storage
        .get_item(&key)
        .map_err(|error| format!("Could not read the local player role: {error:?}"))?
    else {
        return Ok(None);
    };

    let player = decode_local_role(&encoded)?;

    if encode_local_role(player) != encoded {
        return Err("Stored local role is not canonically encoded.".to_owned());
    }

    Ok(Some(player))
}

#[cfg(target_arch = "wasm32")]
pub fn remove_local_role(contract_id: &str) -> Result<(), String> {
    let storage = browser_storage()?;
    let key = local_role_storage_key(contract_id)?;

    storage
        .remove_item(&key)
        .map_err(|error| format!("Could not remove the local player role: {error:?}"))?;

    if storage
        .get_item(&key)
        .map_err(|error| format!("Could not verify local-role removal: {error:?}"))?
        .is_some()
    {
        return Err("Browser storage retained the local role after removal.".to_owned());
    }

    Ok(())
}

fn validate_contract_id(contract_id: &str) -> Result<(), String> {
    if contract_id.is_empty() {
        return Err("Local-role contract ID is empty.".to_owned());
    }

    if contract_id.len() > MAX_CONTRACT_ID_BYTES {
        return Err(format!(
            "Local-role contract ID exceeds {MAX_CONTRACT_ID_BYTES} bytes."
        ));
    }

    if !contract_id
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err("Local-role contract ID contains unsupported characters.".to_owned());
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roles_have_stable_canonical_encodings() {
        assert_eq!(encode_local_role(Player::White), "white");
        assert_eq!(encode_local_role(Player::Black), "black");

        assert_eq!(decode_local_role("white"), Ok(Player::White));
        assert_eq!(decode_local_role("black"), Ok(Player::Black));
    }

    #[test]
    fn malformed_or_noncanonical_roles_are_rejected() {
        for invalid in ["", "White", "BLACK", "player-one", " white", "black "] {
            assert!(decode_local_role(invalid).is_err());
        }
    }

    #[test]
    fn storage_keys_are_contract_scoped() {
        let first = local_role_storage_key("contract-one").unwrap();
        let second = local_role_storage_key("contract-two").unwrap();

        assert_ne!(first, second);
        assert!(first.ends_with(".contract-one"));
        assert!(second.ends_with(".contract-two"));
    }

    #[test]
    fn invalid_contract_ids_are_rejected() {
        assert!(local_role_storage_key("").is_err());
        assert!(local_role_storage_key("contains space").is_err());
        assert!(local_role_storage_key("contains/slash").is_err());
        assert!(local_role_storage_key("contains.dot").is_err());
        assert!(local_role_storage_key(&"a".repeat(129)).is_err());
    }

    #[test]
    fn accepted_contract_id_characters_are_stable() {
        let key = local_role_storage_key("ABC_xyz-0123").unwrap();

        assert_eq!(key, "freenet-backgammon.local-role.v1.ABC_xyz-0123");
    }
}
