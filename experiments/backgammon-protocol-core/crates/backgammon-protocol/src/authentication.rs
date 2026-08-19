use ciborium::ser::into_writer;
use serde::Serialize;

use crate::{ActionId, GameActionPayload, GameId, StateHash};

/// Domain separating protocol-v4 game-action signatures from every other
/// signature produced by the same player identity.
pub const ACTION_SIGNATURE_DOMAIN_V4: &[u8] = b"freenet-backgammon/action/v4";

/// Final immutable action fields covered by protocol-v4 authentication.
///
/// Authentication data itself is deliberately excluded. State hashes describe
/// deterministic game state; signatures authenticate this exact transition.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct ActionSigningBody {
    pub protocol_version: u16,
    pub game_id: GameId,
    pub action_id: ActionId,
    pub sequence: u64,
    pub previous_state_hash: StateHash,
    pub resulting_state_hash: StateHash,
    pub payload: GameActionPayload,
}

/// Encodes the finalized action body using the protocol's deterministic CBOR
/// representation.
///
/// The returned bytes do not include the signature domain.
pub fn encode_action_signing_body_v4(body: &ActionSigningBody) -> Result<Vec<u8>, String> {
    body.payload
        .verify()
        .map_err(|error| format!("invalid signing-body payload: {error:?}"))?;

    let mut encoded = Vec::new();

    into_writer(body, &mut encoded)
        .map_err(|error| format!("failed to encode action signing body: {error}"))?;

    Ok(encoded)
}

/// Builds the exact message signed by protocol-v4 Ed25519 game actions.
///
/// ```text
/// ASCII("freenet-backgammon/action/v4")
/// || 0x00
/// || CBOR(finalized action body)
/// ```
pub fn encode_action_signing_message_v4(body: &ActionSigningBody) -> Result<Vec<u8>, String> {
    let encoded_body = encode_action_signing_body_v4(body)?;

    let mut message = Vec::with_capacity(ACTION_SIGNATURE_DOMAIN_V4.len() + 1 + encoded_body.len());

    message.extend_from_slice(ACTION_SIGNATURE_DOMAIN_V4);
    message.push(0);
    message.extend_from_slice(&encoded_body);

    Ok(message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use backgammon_core::Player;

    fn signing_body() -> ActionSigningBody {
        ActionSigningBody {
            protocol_version: 4,
            game_id: [1; 32],
            action_id: [2; 32],
            sequence: 7,
            previous_state_hash: [3; 32],
            resulting_state_hash: [4; 32],
            payload: GameActionPayload::RequestRoll {
                turn: 5,
                player: Player::Black,
            },
        }
    }

    #[test]
    fn signing_message_is_deterministic() {
        let body = signing_body();

        assert_eq!(
            encode_action_signing_message_v4(&body).unwrap(),
            encode_action_signing_message_v4(&body).unwrap()
        );
    }

    #[test]
    fn signing_message_has_v4_domain_separator() {
        let message = encode_action_signing_message_v4(&signing_body()).unwrap();

        assert!(message.starts_with(ACTION_SIGNATURE_DOMAIN_V4));

        assert_eq!(
            message[ACTION_SIGNATURE_DOMAIN_V4.len()],
            0,
            "domain must be followed by a zero-byte separator"
        );
    }

    #[test]
    fn every_authoritative_field_changes_signing_message() {
        let original = signing_body();
        let expected = encode_action_signing_message_v4(&original).unwrap();

        let mut changed = original.clone();
        changed.protocol_version = 5;
        assert_ne!(
            encode_action_signing_message_v4(&changed).unwrap(),
            expected
        );

        let mut changed = original.clone();
        changed.game_id = [9; 32];
        assert_ne!(
            encode_action_signing_message_v4(&changed).unwrap(),
            expected
        );

        let mut changed = original.clone();
        changed.action_id = [9; 32];
        assert_ne!(
            encode_action_signing_message_v4(&changed).unwrap(),
            expected
        );

        let mut changed = original.clone();
        changed.sequence += 1;
        assert_ne!(
            encode_action_signing_message_v4(&changed).unwrap(),
            expected
        );

        let mut changed = original.clone();
        changed.previous_state_hash = [9; 32];
        assert_ne!(
            encode_action_signing_message_v4(&changed).unwrap(),
            expected
        );

        let mut changed = original.clone();
        changed.resulting_state_hash = [9; 32];
        assert_ne!(
            encode_action_signing_message_v4(&changed).unwrap(),
            expected
        );

        let mut changed = original;
        changed.payload = GameActionPayload::RequestRoll {
            turn: 6,
            player: Player::Black,
        };
        assert_ne!(
            encode_action_signing_message_v4(&changed).unwrap(),
            expected
        );
    }

    #[test]
    fn invalid_payload_is_rejected_before_signing() {
        let mut body = signing_body();

        body.payload = GameActionPayload::CreateGame(crate::GameConfiguration {
            white: crate::PlayerDescriptor {
                id: [1; 32],
                display_name: String::new(),
            },
            black: crate::PlayerDescriptor {
                id: [2; 32],
                display_name: "Black".to_owned(),
            },
            match_length: 1,
        });

        assert!(encode_action_signing_message_v4(&body).is_err());
    }
}
