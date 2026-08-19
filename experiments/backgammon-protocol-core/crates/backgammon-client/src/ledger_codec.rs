use backgammon_contract::{LedgerState, LedgerStateDelta};
use backgammon_protocol::{
    build_next_game_action, encode_action_signing_message_v4, replay_game,
    verify_action_authentication_v4, verify_typed_action_history, Action, ActionAuthentication,
    ActionId, ActionSignature, ActionSigningBody, GameActionPayload, GameActionRecord,
    GameConfiguration, ReplayedGame,
};
use ciborium::{de::from_reader, ser::into_writer};
use ed25519_dalek::{Signer, SigningKey};

/// A decoded and independently verified replicated ledger.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedLedger {
    storage_actions: Vec<Action>,
    typed_actions: Vec<GameActionRecord>,
}

impl VerifiedLedger {
    pub fn storage_actions(&self) -> &[Action] {
        &self.storage_actions
    }

    pub fn typed_actions(&self) -> &[GameActionRecord] {
        &self.typed_actions
    }

    pub fn action_count(&self) -> usize {
        self.typed_actions.len()
    }
}

/// Decodes a complete contract state and verifies both the storage-level
/// action chain and the typed deterministic game replay.
pub fn decode_verified_ledger(bytes: &[u8]) -> Result<VerifiedLedger, String> {
    let state: LedgerState =
        from_reader(bytes).map_err(|error| format!("failed to decode ledger state: {error}"))?;

    verify_typed_action_history(&state.actions.0)
        .map_err(|error| format!("ledger history failed verification: {error}"))?;

    let mut storage_actions = state.actions.0;
    storage_actions.sort_unstable_by_key(|action| action.sequence);

    let typed_actions = storage_actions
        .iter()
        .map(Action::to_game_action_record)
        .collect::<Result<Vec<_>, _>>()?;

    /*
     * Replay once more using the typed representation. This rejects illegal
     * state transitions even if the outer CBOR structure was well formed.
     */
    replay_game(&typed_actions)
        .map_err(|error| format!("typed ledger replay failed: {error:?}"))?;

    Ok(VerifiedLedger {
        storage_actions,
        typed_actions,
    })
}

/// Decodes and independently verifies the replicated ledger, then returns
/// the canonical replay result that must drive the visible network game.
pub fn decode_verified_replay(bytes: &[u8]) -> Result<ReplayedGame, String> {
    let ledger = decode_verified_ledger(bytes)?;

    replay_game(ledger.typed_actions())
        .map_err(|error| format!("typed ledger replay failed: {error:?}"))
}

fn authenticate_player_action_v4(
    record: &GameActionRecord,
    configuration: &GameConfiguration,
    signing_key: &SigningKey,
) -> Result<Action, String> {
    let player = match &record.payload {
        GameActionPayload::RequestRoll { player, .. }
        | GameActionPayload::CommitDice { player, .. }
        | GameActionPayload::RevealDice { player, .. }
        | GameActionPayload::PlayTurn { player, .. }
        | GameActionPayload::Resign { player } => *player,

        _ => {
            return Err(
                "Only post-genesis player actions may use the single-player v4 signer.".to_owned(),
            );
        }
    };

    let expected_player_id = match player {
        backgammon_core::Player::White => configuration.white.id,
        backgammon_core::Player::Black => configuration.black.id,
    };

    let actual_player_id = signing_key.verifying_key().to_bytes();

    if actual_player_id != expected_player_id {
        return Err(format!(
            "Local signing identity does not match the authoritative {player:?} PlayerId."
        ));
    }

    let body = ActionSigningBody {
        protocol_version: record.protocol_version,
        game_id: record.game_id,
        action_id: record.action_id,
        sequence: record.sequence,
        previous_state_hash: record.previous_state_hash,
        resulting_state_hash: record.resulting_state_hash,
        payload: record.payload.clone(),
    };

    let message = encode_action_signing_message_v4(&body)?;

    let authentication = ActionAuthentication::Player {
        signature: ActionSignature::from_bytes(signing_key.sign(&message).to_bytes()),
    };

    /*
     * Verify locally through the same protocol policy used for hostile
     * replicated input before allowing the action onto the wire.
     */
    verify_action_authentication_v4(&body, &authentication, configuration)?;

    Action::from_authenticated_game_action_record(record, authentication)
}

/// Constructs and canonically encodes a signed protocol-v4 post-genesis
/// one-action contract delta.
///
/// The authoritative ledger derives sequence number and both state hashes.
/// The supplied signing key must match the PlayerId assigned to the player
/// named by the action payload.
pub fn build_encoded_signed_action_delta(
    state_bytes: &[u8],
    action_id: ActionId,
    payload: GameActionPayload,
    signing_key: &SigningKey,
) -> Result<(GameActionRecord, Vec<u8>), String> {
    let ledger = decode_verified_ledger(state_bytes)?;

    let configuration = ledger
        .typed_actions()
        .first()
        .and_then(|record| match &record.payload {
            GameActionPayload::CreateGame(configuration) => Some(configuration),
            _ => None,
        })
        .ok_or_else(|| {
            "Verified ledger does not contain an authoritative genesis configuration.".to_owned()
        })?;

    let record = build_next_game_action(ledger.typed_actions(), action_id, payload)
        .map_err(|error| format!("could not build next action: {error:?}"))?;

    let action = authenticate_player_action_v4(&record, configuration, signing_key)?;

    let delta = LedgerStateDelta {
        actions: Some(vec![action]),
    };

    let mut encoded = Vec::new();

    into_writer(&delta, &mut encoded)
        .map_err(|error| format!("failed to encode signed ledger delta: {error}"))?;

    let decoded: LedgerStateDelta = from_reader(encoded.as_slice())
        .map_err(|error| format!("encoded signed ledger delta did not decode: {error}"))?;

    if decoded != delta {
        return Err("Encoded signed ledger delta did not round-trip exactly.".to_owned());
    }

    Ok((record, encoded))
}

/// Constructs and canonically encodes an unsigned one-action contract delta.
///
/// Test-only helper for historical and malformed-wire regression cases.
/// Protocol-v4 production submission must use
/// `build_encoded_signed_action_delta`.
#[cfg(test)]
pub fn build_encoded_action_delta(
    state_bytes: &[u8],
    action_id: ActionId,
    payload: GameActionPayload,
) -> Result<(GameActionRecord, Vec<u8>), String> {
    let ledger = decode_verified_ledger(state_bytes)?;

    let record = build_next_game_action(ledger.typed_actions(), action_id, payload)
        .map_err(|error| format!("could not build next action: {error:?}"))?;

    let action = Action::from_game_action_record(&record)?;

    let delta = LedgerStateDelta {
        actions: Some(vec![action]),
    };

    let mut encoded = Vec::new();

    into_writer(&delta, &mut encoded)
        .map_err(|error| format!("failed to encode ledger delta: {error}"))?;

    /*
     * Require our own encoded output to decode identically before it can be
     * handed to the transport.
     */
    let decoded: LedgerStateDelta = from_reader(encoded.as_slice())
        .map_err(|error| format!("encoded ledger delta did not decode: {error}"))?;

    if decoded != delta {
        return Err("encoded ledger delta did not round-trip exactly".to_owned());
    }

    Ok((record, encoded))
}

#[cfg(test)]
mod tests {
    use super::*;
    use backgammon_core::Player;

    fn one_action_state() -> &'static [u8] {
        crate::test_support::one_action_state()
    }

    fn test_signing_configuration() -> (GameConfiguration, SigningKey, SigningKey) {
        let white = SigningKey::from_bytes(&[41; 32]);
        let black = SigningKey::from_bytes(&[42; 32]);

        let configuration = GameConfiguration {
            white: backgammon_protocol::PlayerDescriptor {
                id: white.verifying_key().to_bytes(),
                display_name: "White".to_owned(),
            },
            black: backgammon_protocol::PlayerDescriptor {
                id: black.verifying_key().to_bytes(),
                display_name: "Black".to_owned(),
            },
            match_length: 1,
        };

        (configuration, white, black)
    }

    #[test]
    fn v4_player_action_is_signed_by_declared_identity() {
        let (configuration, white, _) = test_signing_configuration();

        let record = GameActionRecord {
            protocol_version: backgammon_protocol::PROTOCOL_VERSION,
            game_id: [7; 32],
            action_id: [8; 32],
            sequence: 1,
            previous_state_hash: [9; 32],
            resulting_state_hash: [10; 32],
            payload: GameActionPayload::Resign {
                player: Player::White,
            },
        };

        let action = authenticate_player_action_v4(&record, &configuration, &white).unwrap();

        assert!(matches!(
            action.authentication,
            Some(ActionAuthentication::Player { .. })
        ));

        assert_eq!(action.to_game_action_record().unwrap(), record);
    }

    #[test]
    fn v4_player_action_rejects_opponent_signing_key() {
        let (configuration, _, black) = test_signing_configuration();

        let record = GameActionRecord {
            protocol_version: backgammon_protocol::PROTOCOL_VERSION,
            game_id: [7; 32],
            action_id: [8; 32],
            sequence: 1,
            previous_state_hash: [9; 32],
            resulting_state_hash: [10; 32],
            payload: GameActionPayload::Resign {
                player: Player::White,
            },
        };

        let error = authenticate_player_action_v4(&record, &configuration, &black).unwrap_err();

        assert!(error.contains("does not match the authoritative White PlayerId"));
    }

    #[test]
    fn pinned_network_state_decodes_as_one_verified_action() {
        let ledger = decode_verified_ledger(one_action_state()).unwrap();

        assert_eq!(ledger.action_count(), 1);
        assert_eq!(ledger.storage_actions()[0].sequence, 0);
        assert_eq!(ledger.typed_actions()[0].sequence, 0);
    }

    #[test]
    fn pinned_network_state_returns_verified_authoritative_replay() {
        let replay = decode_verified_replay(one_action_state()).unwrap();

        assert_eq!(replay.next_sequence, 1);
        assert_eq!(replay.next_turn, 0);
        assert_eq!(replay.state, backgammon_core::GameState::standard_start());
        assert_eq!(replay.state.active_player, backgammon_core::Player::White);
        assert_eq!(
            replay.state.turn_phase,
            backgammon_core::TurnPhase::AwaitingRoll
        );
        assert_eq!(replay.state.dice, None);
    }

    #[test]
    fn malformed_state_cannot_produce_authoritative_replay() {
        assert!(decode_verified_replay(&[0xff, 0x00]).is_err());
    }

    #[test]
    fn dynamic_sequence_one_delta_round_trips() {
        let (record, encoded) = build_encoded_action_delta(
            one_action_state(),
            [42; 32],
            GameActionPayload::Resign {
                player: Player::White,
            },
        )
        .unwrap();

        assert_eq!(record.sequence, 1);
        assert_eq!(record.action_id, [42; 32]);

        let delta: LedgerStateDelta = from_reader(encoded.as_slice()).unwrap();

        let actions = delta.actions.expect("delta must contain one action");

        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].sequence, 1);
        assert_eq!(actions[0].id, [42; 32]);

        let decoded_record = actions[0].to_game_action_record().unwrap();

        assert_eq!(decoded_record, record);
    }

    #[test]
    fn malformed_state_is_rejected_without_building_delta() {
        assert!(build_encoded_action_delta(
            &[0xff, 0x00],
            [43; 32],
            GameActionPayload::Resign {
                player: Player::White,
            },
        )
        .is_err());
    }

    #[test]
    fn duplicate_action_id_is_rejected() {
        let ledger = decode_verified_ledger(one_action_state()).unwrap();
        let duplicate = ledger.typed_actions()[0].action_id;

        assert!(build_encoded_action_delta(
            one_action_state(),
            duplicate,
            GameActionPayload::Resign {
                player: Player::White,
            },
        )
        .is_err());
    }
}
