use backgammon_contract::{LedgerState, LedgerStateDelta};
use backgammon_protocol::{
    build_next_game_action, replay_game, verify_typed_action_history, Action, ActionId,
    GameActionPayload, GameActionRecord,
};
use ciborium::{de::from_reader, ser::into_writer};

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

/// Constructs and canonically encodes a one-action contract delta.
///
/// The sequence number, previous-state hash, and resulting-state hash are
/// derived from the verified ledger and cannot be supplied by the caller.
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

    const ONE_ACTION_STATE: &[u8] = include_bytes!("../fixtures/expected-one-action-state.cbor");

    #[test]
    fn pinned_network_state_decodes_as_one_verified_action() {
        let ledger = decode_verified_ledger(ONE_ACTION_STATE).unwrap();

        assert_eq!(ledger.action_count(), 1);
        assert_eq!(ledger.storage_actions()[0].sequence, 0);
        assert_eq!(ledger.typed_actions()[0].sequence, 0);
    }

    #[test]
    fn dynamic_sequence_one_delta_round_trips() {
        let (record, encoded) = build_encoded_action_delta(
            ONE_ACTION_STATE,
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
        let ledger = decode_verified_ledger(ONE_ACTION_STATE).unwrap();
        let duplicate = ledger.typed_actions()[0].action_id;

        assert!(build_encoded_action_delta(
            ONE_ACTION_STATE,
            duplicate,
            GameActionPayload::Resign {
                player: Player::White,
            },
        )
        .is_err());
    }
}
