use ciborium::{de::from_reader, ser::into_writer};
use backgammon_core::Player;
use serde::{Deserialize, Serialize};
use std::io::Cursor;

use crate::GameId;

/// Keep pending lobby negotiations separate across lobby instances without
/// changing the signed player identity stored inside their payloads.
/// Game recovery slots retain their existing scope and exact secret keys.
pub fn scope_lobby_durable_request(mut request: DurableRequest, lobby_id: &str) -> DurableRequest {
    if matches!(request.slot, DurableSlot::OutboundChallenge | DurableSlot::IncomingAcceptance) {
        let mut hash = blake3::Hasher::new();
        hash.update(b"freenet-backgammon:durable-lobby-player:v1\0");
        hash.update(&(lobby_id.len() as u64).to_be_bytes());
        hash.update(lobby_id.as_bytes());
        hash.update(&request.scope);
        request.scope = *hash.finalize().as_bytes();
    }
    request
}

/// Versioned, bounded messages for local Freenet delegate secrets. The scope
/// identifies a player or game; the slot prevents unrelated records sharing a
/// key. Client code must also validate the record's authenticated contents.
pub const DURABLE_STATE_VERSION: u16 = 1;
// Pending action deltas may contain up to 64 KiB, plus bounded metadata.
pub const MAX_DURABLE_VALUE_BYTES: usize = 72 * 1024;
pub const MAX_DURABLE_MESSAGE_BYTES: usize = 2 * MAX_DURABLE_VALUE_BYTES + 1024;

/// A separate secret slot for each dice round and local player. The domain
/// prevents a game ID from sharing a scope with a round-zero secret.
pub fn dice_secret_durable_scope(game_id: &GameId, turn: u32, player: Player) -> [u8; 32] {
    let mut hash = blake3::Hasher::new();
    hash.update(b"freenet-backgammon:durable-dice-secret:v1\0");
    hash.update(game_id);
    hash.update(&turn.to_be_bytes());
    hash.update(&[match player { Player::White => 1, Player::Black => 2 }]);
    *hash.finalize().as_bytes()
}

/// One pending action and one local role per canonical game-contract ID.
/// The authenticated record is checked against the contract ID after reading.
pub fn contract_durable_scope(contract_id: &str) -> [u8; 32] {
    let mut hash = blake3::Hasher::new();
    hash.update(b"freenet-backgammon:durable-contract:v1\0");
    hash.update(contract_id.as_bytes());
    *hash.finalize().as_bytes()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DurableSlot {
    OutboundChallenge,
    IncomingAcceptance,
    GenesisHandshake,
    DiceSecret,
    PendingAction,
    LocalRole,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DurableAction {
    Get,
    Create(Vec<u8>),
    Replace { expected: Vec<u8>, replacement: Vec<u8> },
    Delete { expected: Vec<u8> },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DurableRequest {
    pub version: u16,
    pub request_id: [u8; 32],
    pub scope: [u8; 32],
    pub slot: DurableSlot,
    pub action: DurableAction,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DurableResult {
    Value(Option<Vec<u8>>),
    Stored,
    Deleted,
    Conflict,
    Error(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DurableResponse {
    pub version: u16,
    pub request_id: [u8; 32],
    pub scope: [u8; 32],
    pub slot: DurableSlot,
    pub result: DurableResult,
}

fn check_value(value: &[u8]) -> Result<(), String> {
    if value.is_empty() || value.len() > MAX_DURABLE_VALUE_BYTES {
        return Err("durable value has invalid length".to_owned());
    }
    Ok(())
}

fn check_action(action: &DurableAction) -> Result<(), String> {
    match action {
        DurableAction::Get => Ok(()),
        DurableAction::Create(value) => check_value(value),
        DurableAction::Replace { expected, replacement } => {
            check_value(expected)?;
            check_value(replacement)
        }
        DurableAction::Delete { expected } => check_value(expected),
    }
}

fn encode<T: Serialize>(message: &T) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    into_writer(message, &mut bytes).map_err(|error| format!("durable message encoding: {error}"))?;
    if bytes.len() > MAX_DURABLE_MESSAGE_BYTES {
        return Err("durable message exceeds maximum length".to_owned());
    }
    Ok(bytes)
}

fn decode<T: for<'de> Deserialize<'de>>(bytes: &[u8]) -> Result<T, String> {
    if bytes.len() > MAX_DURABLE_MESSAGE_BYTES {
        return Err("durable message exceeds maximum length".to_owned());
    }
    let mut cursor = Cursor::new(bytes);
    let decoded = from_reader(&mut cursor)
        .map_err(|error| format!("durable message decoding: {error}"))?;
    if cursor.position() != bytes.len() as u64 {
        return Err("durable message has trailing bytes".to_owned());
    }
    Ok(decoded)
}

pub fn encode_durable_request(request: &DurableRequest) -> Result<Vec<u8>, String> {
    if request.version != DURABLE_STATE_VERSION {
        return Err("unsupported durable request version".to_owned());
    }
    check_action(&request.action)?;
    encode(request)
}

pub fn decode_durable_request(bytes: &[u8]) -> Result<DurableRequest, String> {
    let request: DurableRequest = decode(bytes)?;
    if encode_durable_request(&request)? != bytes {
        return Err("durable request is not canonical".to_owned());
    }
    Ok(request)
}

pub fn encode_durable_response(response: &DurableResponse) -> Result<Vec<u8>, String> {
    if response.version != DURABLE_STATE_VERSION {
        return Err("unsupported durable response version".to_owned());
    }
    if let DurableResult::Value(Some(value)) = &response.result {
        check_value(value)?;
    }
    encode(response)
}

pub fn decode_durable_response(bytes: &[u8]) -> Result<DurableResponse, String> {
    let response: DurableResponse = decode(bytes)?;
    if encode_durable_response(&response)? != bytes {
        return Err("durable response is not canonical".to_owned());
    }
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dice_secret_scopes_separate_games_turns_and_players() {
        let scope = dice_secret_durable_scope(&[1; 32], 3, Player::White);
        assert_eq!(scope, dice_secret_durable_scope(&[1; 32], 3, Player::White));
        assert_ne!(scope, dice_secret_durable_scope(&[2; 32], 3, Player::White));
        assert_ne!(scope, dice_secret_durable_scope(&[1; 32], 4, Player::White));
        assert_ne!(scope, dice_secret_durable_scope(&[1; 32], 3, Player::Black));
        assert_ne!(scope, [1; 32]);
    }

    #[test]
    fn contract_scopes_separate_contracts_and_dice() {
        assert_eq!(contract_durable_scope("contract-A"), contract_durable_scope("contract-A"));
        assert_ne!(contract_durable_scope("contract-A"), contract_durable_scope("contract-B"));
        assert_ne!(contract_durable_scope("contract-A"), dice_secret_durable_scope(&[0; 32], 0, Player::White));
    }

    #[test]
    fn pending_lobby_records_are_separated_by_lobby_and_player() {
        let request = DurableRequest {
            version: DURABLE_STATE_VERSION, request_id: [1; 32], scope: [2; 32],
            slot: DurableSlot::OutboundChallenge, action: DurableAction::Get,
        };
        let scoped = scope_lobby_durable_request(request.clone(), "lobby-A");
        assert_eq!(scoped, scope_lobby_durable_request(request.clone(), "lobby-A"));
        assert_ne!(scoped.scope, request.scope);
        assert_ne!(scoped.scope, scope_lobby_durable_request(request.clone(), "lobby-B").scope);
        let mut other_player = request.clone();
        other_player.scope = [3; 32];
        assert_ne!(scoped.scope, scope_lobby_durable_request(other_player, "lobby-A").scope);
        for slot in [DurableSlot::OutboundChallenge, DurableSlot::IncomingAcceptance] {
            let mut pending = request.clone();
            pending.slot = slot;
            pending.action = DurableAction::Delete { expected: vec![7, 8] };
            let namespaced = scope_lobby_durable_request(pending.clone(), "lobby-A");
            assert_eq!(namespaced.scope, scoped.scope);
            assert_eq!(namespaced.slot, pending.slot);
            assert_eq!(namespaced.action, pending.action);
            assert_eq!(namespaced.request_id, pending.request_id);
        }
    }

    #[test]
    fn lobby_rotation_preserves_game_recovery_scopes() {
        for slot in [DurableSlot::GenesisHandshake, DurableSlot::DiceSecret,
            DurableSlot::PendingAction, DurableSlot::LocalRole] {
            let request = DurableRequest {
                version: DURABLE_STATE_VERSION, request_id: [1; 32], scope: [2; 32],
                slot, action: DurableAction::Get,
            };
            assert_eq!(scope_lobby_durable_request(request.clone(), "lobby-A"), request);
            assert_eq!(scope_lobby_durable_request(request.clone(), "lobby-B"), request);
        }
    }

    #[test]
    fn request_response_round_trip_and_trailing_bytes() {
        let request = DurableRequest {
            version: DURABLE_STATE_VERSION,
            request_id: [1; 32],
            scope: [2; 32],
            slot: DurableSlot::OutboundChallenge,
            action: DurableAction::Replace {
                expected: vec![3],
                replacement: vec![4],
            },
        };
        let bytes = encode_durable_request(&request).unwrap();
        assert_eq!(decode_durable_request(&bytes), Ok(request));
        let mut invalid = bytes;
        invalid.push(0);
        assert!(decode_durable_request(&invalid).is_err());
        let response = DurableResponse {
            version: DURABLE_STATE_VERSION,
            request_id: [1; 32],
            scope: [2; 32],
            slot: DurableSlot::OutboundChallenge,
            result: DurableResult::Value(Some(vec![5])),
        };
        assert_eq!(decode_durable_response(&encode_durable_response(&response).unwrap()), Ok(response));
    }

    #[test]
    fn rejects_oversized_or_empty_values() {
        let mut request = DurableRequest {
            version: DURABLE_STATE_VERSION,
            request_id: [1; 32],
            scope: [2; 32],
            slot: DurableSlot::PendingAction,
            action: DurableAction::Create(Vec::new()),
        };
        assert!(encode_durable_request(&request).is_err());
        request.action = DurableAction::Create(vec![1; MAX_DURABLE_VALUE_BYTES + 1]);
        assert!(encode_durable_request(&request).is_err());
    }
}
