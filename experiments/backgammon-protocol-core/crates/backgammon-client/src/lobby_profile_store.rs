use backgammon_protocol::PlayerId;

#[cfg(target_arch = "wasm32")]
use crate::lobby::validate_display_name;

const STORAGE_PREFIX: &str = "freenet-backgammon.lobby-profile.v1";

/// Display-name storage is scoped to the persistent cryptographic identity.
///
/// A new PlayerId therefore receives a fresh lobby profile instead of silently
/// inheriting presentation metadata from a different identity.
pub fn lobby_profile_storage_key(player_id: &PlayerId) -> String {
    format!("{STORAGE_PREFIX}.{}", encode_player_id(player_id))
}

#[cfg(target_arch = "wasm32")]
pub fn load_lobby_display_name(player_id: &PlayerId) -> Result<Option<String>, String> {
    let storage = browser_storage()?;
    let key = lobby_profile_storage_key(player_id);

    let Some(display_name) = storage
        .get_item(&key)
        .map_err(|error| format!("Could not read the lobby display name: {error:?}"))?
    else {
        return Ok(None);
    };

    /*
     * Browser storage is untrusted local input. Apply exactly the same
     * display-name rule used by signed lobby presence and game configuration.
     */
    validate_display_name(&display_name)?;

    Ok(Some(display_name))
}

#[cfg(target_arch = "wasm32")]
pub fn store_lobby_display_name(player_id: &PlayerId, display_name: &str) -> Result<(), String> {
    validate_display_name(display_name)?;

    let storage = browser_storage()?;
    let key = lobby_profile_storage_key(player_id);

    storage
        .set_item(&key, display_name)
        .map_err(|error| format!("Could not persist the lobby display name: {error:?}"))?;

    /*
     * Match the project's existing durable-browser-store convention:
     * immediately read back and require the exact value before reporting
     * success.
     */
    let persisted = storage
        .get_item(&key)
        .map_err(|error| format!("Could not verify the persisted lobby display name: {error:?}"))?
        .ok_or_else(|| "Browser storage did not retain the lobby display name.".to_owned())?;

    validate_display_name(&persisted)?;

    if persisted != display_name {
        return Err(
            "Persisted lobby display name failed exact round-trip verification.".to_owned(),
        );
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

fn encode_player_id(player_id: &PlayerId) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";

    let mut encoded = String::with_capacity(player_id.len() * 2);

    for byte in player_id {
        encoded.push(HEX[usize::from(byte >> 4)] as char);
        encoded.push(HEX[usize::from(byte & 0x0f)] as char);
    }

    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_keys_are_player_identity_scoped() {
        let first = lobby_profile_storage_key(&[0x11; 32]);
        let second = lobby_profile_storage_key(&[0x22; 32]);

        assert_ne!(first, second);
        assert!(first.starts_with(STORAGE_PREFIX));
        assert!(second.starts_with(STORAGE_PREFIX));
    }

    #[test]
    fn storage_keys_use_fixed_width_canonical_player_ids() {
        let mut player_id = [0_u8; 32];
        player_id[0] = 0xab;
        player_id[31] = 0xcd;

        let key = lobby_profile_storage_key(&player_id);
        let encoded = key.strip_prefix(&format!("{STORAGE_PREFIX}.")).unwrap();

        assert_eq!(encoded.len(), 64);
        assert!(encoded.starts_with("ab"));
        assert!(encoded.ends_with("cd"));

        assert!(encoded
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)));
    }
}
