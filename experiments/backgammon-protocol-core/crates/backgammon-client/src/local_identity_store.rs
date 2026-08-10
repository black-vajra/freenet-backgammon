use backgammon_core::Player;
use backgammon_protocol::{GameConfiguration, PlayerId};
use ed25519_dalek::SigningKey;

const STORAGE_KEY: &str = "freenet-backgammon.local-identity.v1";
const ENCODED_SEED_BYTES: usize = 64;

pub fn signing_key_from_seed(seed: [u8; 32]) -> SigningKey {
    SigningKey::from_bytes(&seed)
}

pub fn player_id_for_signing_key(signing_key: &SigningKey) -> PlayerId {
    signing_key.verifying_key().to_bytes()
}

pub fn role_for_player_id(
    configuration: &GameConfiguration,
    player_id: &PlayerId,
) -> Option<Player> {
    if configuration.white.id == *player_id {
        Some(Player::White)
    } else if configuration.black.id == *player_id {
        Some(Player::Black)
    } else {
        None
    }
}

fn encode_identity_seed(seed: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";

    let mut encoded = String::with_capacity(ENCODED_SEED_BYTES);

    for byte in seed {
        encoded.push(HEX[usize::from(byte >> 4)] as char);
        encoded.push(HEX[usize::from(byte & 0x0f)] as char);
    }

    encoded
}

fn decode_identity_seed(encoded: &str) -> Result<[u8; 32], String> {
    if encoded.len() != ENCODED_SEED_BYTES {
        return Err(format!(
            "Stored local identity has invalid length {}; expected {ENCODED_SEED_BYTES}.",
            encoded.len()
        ));
    }

    let bytes = encoded.as_bytes();
    let mut seed = [0_u8; 32];

    for (index, slot) in seed.iter_mut().enumerate() {
        let high = decode_lower_hex_nibble(bytes[index * 2])?;
        let low = decode_lower_hex_nibble(bytes[index * 2 + 1])?;
        *slot = (high << 4) | low;
    }

    if encode_identity_seed(&seed) != encoded {
        return Err("Stored local identity is not canonically encoded.".to_owned());
    }

    Ok(seed)
}

fn decode_lower_hex_nibble(value: u8) -> Result<u8, String> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        _ => Err("Stored local identity contains noncanonical hexadecimal data.".to_owned()),
    }
}

#[cfg(target_arch = "wasm32")]
pub fn store_local_identity(signing_key: &SigningKey) -> Result<(), String> {
    let storage = browser_storage()?;
    let encoded = encode_identity_seed(&signing_key.to_bytes());

    storage
        .set_item(STORAGE_KEY, &encoded)
        .map_err(|error| format!("Could not persist the local identity: {error:?}"))?;

    let persisted = storage
        .get_item(STORAGE_KEY)
        .map_err(|error| format!("Could not verify the persisted local identity: {error:?}"))?
        .ok_or_else(|| "Browser storage did not retain the local identity.".to_owned())?;

    let decoded = decode_identity_seed(&persisted)?;

    if decoded != signing_key.to_bytes() || encode_identity_seed(&decoded) != persisted {
        let _ = storage.remove_item(STORAGE_KEY);

        return Err("Persisted local identity failed exact round-trip verification.".to_owned());
    }

    Ok(())
}

#[cfg(target_arch = "wasm32")]
pub fn load_local_identity() -> Result<Option<SigningKey>, String> {
    let storage = browser_storage()?;

    let Some(encoded) = storage
        .get_item(STORAGE_KEY)
        .map_err(|error| format!("Could not read the local identity: {error:?}"))?
    else {
        return Ok(None);
    };

    let seed = decode_identity_seed(&encoded)?;

    if encode_identity_seed(&seed) != encoded {
        return Err("Stored local identity is not canonically encoded.".to_owned());
    }

    Ok(Some(signing_key_from_seed(seed)))
}

#[cfg(target_arch = "wasm32")]
pub fn load_or_create_local_identity(candidate_seed: [u8; 32]) -> Result<SigningKey, String> {
    if let Some(existing) = load_local_identity()? {
        return Ok(existing);
    }

    let signing_key = signing_key_from_seed(candidate_seed);
    store_local_identity(&signing_key)?;

    Ok(signing_key)
}

#[cfg(target_arch = "wasm32")]
pub fn remove_local_identity() -> Result<(), String> {
    let storage = browser_storage()?;

    storage
        .remove_item(STORAGE_KEY)
        .map_err(|error| format!("Could not remove the local identity: {error:?}"))?;

    if storage
        .get_item(STORAGE_KEY)
        .map_err(|error| format!("Could not verify local-identity removal: {error:?}"))?
        .is_some()
    {
        return Err("Browser storage retained the local identity after removal.".to_owned());
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
    use backgammon_protocol::PlayerDescriptor;

    fn descriptor(seed_byte: u8, display_name: &str) -> PlayerDescriptor {
        let signing_key = signing_key_from_seed([seed_byte; 32]);

        PlayerDescriptor {
            id: player_id_for_signing_key(&signing_key),
            display_name: display_name.to_owned(),
        }
    }

    fn configuration() -> GameConfiguration {
        GameConfiguration {
            white: descriptor(1, "White"),
            black: descriptor(2, "Black"),
            match_length: 1,
        }
    }

    #[test]
    fn fixed_seed_derives_stable_player_id() {
        let first = signing_key_from_seed([7; 32]);
        let second = signing_key_from_seed([7; 32]);
        let different = signing_key_from_seed([8; 32]);

        assert_eq!(
            player_id_for_signing_key(&first),
            player_id_for_signing_key(&second)
        );
        assert_ne!(
            player_id_for_signing_key(&first),
            player_id_for_signing_key(&different)
        );
    }

    #[test]
    fn authoritative_configuration_resolves_player_role() {
        let configuration = configuration();

        assert_eq!(
            role_for_player_id(&configuration, &configuration.white.id),
            Some(Player::White)
        );
        assert_eq!(
            role_for_player_id(&configuration, &configuration.black.id),
            Some(Player::Black)
        );
        assert_eq!(role_for_player_id(&configuration, &[99; 32]), None);
    }

    #[test]
    fn identity_seed_hex_round_trip_is_canonical() {
        let seed = [0xab; 32];
        let encoded = encode_identity_seed(&seed);

        assert_eq!(encoded.len(), ENCODED_SEED_BYTES);
        assert_eq!(encoded, "ab".repeat(32));
        assert_eq!(decode_identity_seed(&encoded), Ok(seed));
    }

    #[test]
    fn malformed_identity_seed_encodings_are_rejected() {
        let invalid = [
            String::new(),
            "a".repeat(63),
            "a".repeat(65),
            "AB".repeat(32),
            "gg".repeat(32),
        ];

        for encoded in invalid {
            assert!(decode_identity_seed(&encoded).is_err());
        }
    }
}
