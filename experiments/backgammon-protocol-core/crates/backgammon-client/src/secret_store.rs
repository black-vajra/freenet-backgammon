use backgammon_core::Player;
use backgammon_protocol::{DiceCommit, DiceCommitment, DiceSecret, GameId};

const STORAGE_PREFIX: &str = "freenet-backgammon.dice-secret.v1";

pub fn dice_secret_storage_key(game_id: &GameId, turn: u32, player: Player) -> String {
    format!(
        "{STORAGE_PREFIX}.{}.{}.{}",
        encode_hex(game_id),
        turn,
        player_label(player),
    )
}

pub fn verify_dice_secret_commitment(
    game_id: &GameId,
    turn: u32,
    player: Player,
    expected_commitment: &DiceCommitment,
    secret: &DiceSecret,
) -> Result<(), String> {
    let derived = DiceCommit::new(game_id, turn, player, secret);

    if derived.commitment != *expected_commitment {
        return Err(
            "Stored dice secret does not match the accepted network commitment.".to_owned(),
        );
    }

    Ok(())
}

#[cfg(target_arch = "wasm32")]
pub fn store_dice_secret(
    game_id: &GameId,
    turn: u32,
    player: Player,
    secret: &DiceSecret,
) -> Result<(), String> {
    let storage = browser_storage()?;
    let key = dice_secret_storage_key(game_id, turn, player);
    let encoded = encode_hex(secret);

    storage
        .set_item(&key, &encoded)
        .map_err(|error| format!("Could not persist the dice secret: {error:?}"))?;

    let persisted = storage
        .get_item(&key)
        .map_err(|error| format!("Could not verify the persisted dice secret: {error:?}"))?
        .ok_or_else(|| "Browser storage did not retain the dice secret.".to_owned())?;

    let decoded = decode_secret_hex(&persisted)?;

    if decoded != *secret || encode_hex(&decoded) != persisted {
        let _ = storage.remove_item(&key);

        return Err("Persisted dice secret failed exact round-trip verification.".to_owned());
    }

    Ok(())
}

#[cfg(target_arch = "wasm32")]
pub fn load_dice_secret(
    game_id: &GameId,
    turn: u32,
    player: Player,
) -> Result<Option<DiceSecret>, String> {
    let storage = browser_storage()?;
    let key = dice_secret_storage_key(game_id, turn, player);

    let Some(encoded) = storage
        .get_item(&key)
        .map_err(|error| format!("Could not read the dice secret: {error:?}"))?
    else {
        return Ok(None);
    };

    let secret = decode_secret_hex(&encoded)?;

    if encode_hex(&secret) != encoded {
        return Err("Stored dice secret is not canonically encoded.".to_owned());
    }

    Ok(Some(secret))
}

#[cfg(target_arch = "wasm32")]
pub fn remove_dice_secret(game_id: &GameId, turn: u32, player: Player) -> Result<(), String> {
    let storage = browser_storage()?;
    let key = dice_secret_storage_key(game_id, turn, player);

    storage
        .remove_item(&key)
        .map_err(|error| format!("Could not remove the dice secret: {error:?}"))
}

#[cfg(target_arch = "wasm32")]
fn browser_storage() -> Result<web_sys::Storage, String> {
    let window = web_sys::window().ok_or_else(|| "Browser window is unavailable.".to_owned())?;

    window
        .local_storage()
        .map_err(|error| format!("Browser storage is unavailable: {error:?}"))?
        .ok_or_else(|| "Browser local storage is disabled.".to_owned())
}

fn player_label(player: Player) -> &'static str {
    match player {
        Player::White => "white",
        Player::Black => "black",
    }
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

fn decode_secret_hex(encoded: &str) -> Result<DiceSecret, String> {
    if encoded.len() != 64 {
        return Err(format!(
            "Stored dice secret has invalid length {}; expected 64.",
            encoded.len()
        ));
    }

    let bytes = encoded.as_bytes();
    let mut secret = [0_u8; 32];

    for (index, pair) in bytes.chunks_exact(2).enumerate() {
        let high = decode_hex_nibble(pair[0])?;
        let low = decode_hex_nibble(pair[1])?;

        secret[index] = (high << 4) | low;
    }

    Ok(secret)
}

fn decode_hex_nibble(value: u8) -> Result<u8, String> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        _ => Err("Stored dice secret contains noncanonical hexadecimal.".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matching_secret_verifies_against_commitment() {
        let game_id = [7_u8; 32];
        let secret = [11_u8; 32];

        let commitment = DiceCommit::new(&game_id, 0, Player::White, &secret);

        assert_eq!(
            verify_dice_secret_commitment(
                &game_id,
                0,
                Player::White,
                &commitment.commitment,
                &secret,
            ),
            Ok(()),
        );
    }

    #[test]
    fn wrong_secret_is_rejected_against_commitment() {
        let game_id = [7_u8; 32];

        let commitment = DiceCommit::new(&game_id, 0, Player::White, &[11_u8; 32]);

        assert!(verify_dice_secret_commitment(
            &game_id,
            0,
            Player::White,
            &commitment.commitment,
            &[12_u8; 32],
        )
        .is_err());
    }

    #[test]
    fn secret_encoding_round_trips_exactly() {
        let secret = [0xabu8; 32];
        let encoded = encode_hex(&secret);

        assert_eq!(encoded.len(), 64);
        assert_eq!(decode_secret_hex(&encoded), Ok(secret));
    }

    #[test]
    fn malformed_or_noncanonical_secrets_are_rejected() {
        assert!(decode_secret_hex("abcd").is_err());
        assert!(decode_secret_hex(&"G".repeat(64)).is_err());
        assert!(decode_secret_hex(&"A".repeat(64)).is_err());
    }

    #[test]
    fn storage_keys_are_context_bound() {
        let game = [7_u8; 32];

        let white_turn_zero = dice_secret_storage_key(&game, 0, Player::White);

        assert_ne!(
            white_turn_zero,
            dice_secret_storage_key(&game, 1, Player::White),
        );

        assert_ne!(
            white_turn_zero,
            dice_secret_storage_key(&game, 0, Player::Black),
        );

        assert_ne!(
            white_turn_zero,
            dice_secret_storage_key(&[8_u8; 32], 0, Player::White),
        );
    }
}
