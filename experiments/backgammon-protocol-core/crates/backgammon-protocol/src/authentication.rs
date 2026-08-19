use ciborium::ser::into_writer;
use ed25519_dalek::{Signature, VerifyingKey};
use serde::Serialize;

use crate::{ActionId, GameActionPayload, GameId, PlayerId, StateHash};

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

/// Raw byte length of an Ed25519 signature.
pub const ED25519_SIGNATURE_BYTES: usize = 64;

/// Serialized Ed25519 signature carried by a protocol-v4 action.
///
/// The byte vector is intentionally verified explicitly so malformed network
/// input can be rejected cleanly rather than being assumed to have a valid
/// length.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct ActionSignature(pub Vec<u8>);

impl ActionSignature {
    pub fn from_bytes(bytes: [u8; ED25519_SIGNATURE_BYTES]) -> Self {
        Self(bytes.to_vec())
    }

    pub fn verify(&self) -> Result<(), String> {
        if self.0.len() != ED25519_SIGNATURE_BYTES {
            return Err(format!(
                "invalid Ed25519 signature length: expected {ED25519_SIGNATURE_BYTES} bytes, got {}",
                self.0.len()
            ));
        }

        Ok(())
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

/// Authentication material attached to a finalized protocol-v4 game action.
///
/// Genesis is jointly authorized by both configured players. Every supported
/// post-genesis action carries exactly one player signature.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, serde::Deserialize)]
pub enum ActionAuthentication {
    Genesis {
        white_signature: ActionSignature,
        black_signature: ActionSignature,
    },
    Player {
        signature: ActionSignature,
    },
}

impl ActionAuthentication {
    pub fn verify_structure(&self) -> Result<(), String> {
        match self {
            Self::Genesis {
                white_signature,
                black_signature,
            } => {
                white_signature.verify()?;
                black_signature.verify()?;
            }
            Self::Player { signature } => {
                signature.verify()?;
            }
        }

        Ok(())
    }
}

/// Verifies one protocol-v4 action signature against the player's
/// authoritative Ed25519 public identity.
///
/// `PlayerId` is interpreted directly as the 32-byte Ed25519 verifying key.
/// The signed message is reconstructed locally from the finalized action body;
/// peer-supplied signing bytes are never trusted.
pub fn verify_action_signature_v4(
    player_id: &PlayerId,
    signature_bytes: &[u8],
    body: &ActionSigningBody,
) -> Result<(), String> {
    let signature_array: [u8; ED25519_SIGNATURE_BYTES] = signature_bytes
        .try_into()
        .map_err(|_| {
            format!(
                "invalid Ed25519 signature length: expected {ED25519_SIGNATURE_BYTES} bytes, got {}",
                signature_bytes.len()
            )
        })?;

    let verifying_key = VerifyingKey::from_bytes(player_id)
        .map_err(|error| format!("invalid Ed25519 player identity: {error}"))?;

    let signature = Signature::from_bytes(&signature_array);
    let message = encode_action_signing_message_v4(body)?;

    verifying_key
        .verify_strict(&message, &signature)
        .map_err(|error| format!("invalid protocol-v4 action signature: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use backgammon_core::Player;
    use ed25519_dalek::{Signer, SigningKey};

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

    fn deterministic_signing_key(seed_byte: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed_byte; 32])
    }

    #[test]
    fn action_signature_requires_exact_ed25519_length() {
        let valid = ActionSignature(vec![7; ED25519_SIGNATURE_BYTES]);
        valid.verify().unwrap();

        assert!(ActionSignature(vec![7; 63]).verify().is_err());
        assert!(ActionSignature(vec![7; 65]).verify().is_err());
    }

    #[test]
    fn genesis_authentication_requires_two_well_formed_signatures() {
        let valid = ActionAuthentication::Genesis {
            white_signature: ActionSignature(vec![1; ED25519_SIGNATURE_BYTES]),
            black_signature: ActionSignature(vec![2; ED25519_SIGNATURE_BYTES]),
        };

        valid.verify_structure().unwrap();

        let invalid_white = ActionAuthentication::Genesis {
            white_signature: ActionSignature(vec![1; 63]),
            black_signature: ActionSignature(vec![2; ED25519_SIGNATURE_BYTES]),
        };

        assert!(invalid_white.verify_structure().is_err());

        let invalid_black = ActionAuthentication::Genesis {
            white_signature: ActionSignature(vec![1; ED25519_SIGNATURE_BYTES]),
            black_signature: ActionSignature(vec![2; 65]),
        };

        assert!(invalid_black.verify_structure().is_err());
    }

    #[test]
    fn player_authentication_requires_one_well_formed_signature() {
        let valid = ActionAuthentication::Player {
            signature: ActionSignature(vec![3; ED25519_SIGNATURE_BYTES]),
        };

        valid.verify_structure().unwrap();

        let invalid = ActionAuthentication::Player {
            signature: ActionSignature(vec![3; 63]),
        };

        assert!(invalid.verify_structure().is_err());
    }

    #[test]
    fn valid_action_signature_is_accepted() {
        let body = signing_body();
        let signing_key = deterministic_signing_key(17);
        let player_id = signing_key.verifying_key().to_bytes();
        let message = encode_action_signing_message_v4(&body).unwrap();
        let signature = signing_key.sign(&message).to_bytes();

        verify_action_signature_v4(&player_id, &signature, &body).unwrap();
    }

    #[test]
    fn signature_from_wrong_player_is_rejected() {
        let body = signing_body();
        let signer = deterministic_signing_key(17);
        let wrong_player = deterministic_signing_key(23);
        let message = encode_action_signing_message_v4(&body).unwrap();
        let signature = signer.sign(&message).to_bytes();

        assert!(verify_action_signature_v4(
            &wrong_player.verifying_key().to_bytes(),
            &signature,
            &body
        )
        .is_err());
    }

    #[test]
    fn signature_does_not_survive_action_mutation() {
        let body = signing_body();
        let signing_key = deterministic_signing_key(17);
        let player_id = signing_key.verifying_key().to_bytes();
        let message = encode_action_signing_message_v4(&body).unwrap();
        let signature = signing_key.sign(&message).to_bytes();

        let mut mutated = body;
        mutated.sequence += 1;

        assert!(verify_action_signature_v4(&player_id, &signature, &mutated).is_err());
    }

    #[test]
    fn malformed_signature_lengths_are_rejected() {
        let body = signing_body();
        let signing_key = deterministic_signing_key(17);
        let player_id = signing_key.verifying_key().to_bytes();

        assert!(verify_action_signature_v4(&player_id, &[0; 63], &body).is_err());

        assert!(verify_action_signature_v4(&player_id, &[0; 65], &body).is_err());
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
