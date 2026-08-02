#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u16 = 1;
pub const GENESIS_STATE_HASH: StateHash = [0_u8; 32];

pub type GameId = [u8; 32];
pub type ActionId = [u8; 32];
pub type StateHash = [u8; 32];

#[derive(Serialize, Deserialize, Clone, Default, PartialEq, Eq, Debug)]
pub struct LedgerParameters {
    pub protocol_version: u16,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct Action {
    pub game_id: GameId,
    pub id: ActionId,
    pub sequence: u32,
    pub previous_state_hash: StateHash,
    pub resulting_state_hash: StateHash,
    pub payload: Vec<u8>,
}

impl LedgerParameters {
    pub const fn current() -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
        }
    }

    pub fn verify(&self) -> Result<(), String> {
        if self.protocol_version != PROTOCOL_VERSION {
            return Err("unsupported protocol version".into());
        }

        Ok(())
    }
}

pub fn verify_action_sequences(actions: &[Action]) -> Result<(), String> {
    let mut sequences: Vec<u32> = actions.iter().map(|action| action.sequence).collect();
    sequences.sort_unstable();

    for (expected, actual) in sequences.into_iter().enumerate() {
        let expected =
            u32::try_from(expected).map_err(|_| "action sequence exceeds supported range")?;

        if actual < expected {
            return Err("duplicate action sequence".into());
        }

        if actual > expected {
            return Err("action sequence gap".into());
        }
    }

    Ok(())
}

pub fn verify_action_history(actions: &[Action]) -> Result<(), String> {
    verify_action_sequences(actions)?;

    if actions.is_empty() {
        return Ok(());
    }

    let expected_game_id = actions[0].game_id;

    if actions
        .iter()
        .any(|action| action.game_id != expected_game_id)
    {
        return Err("actions belong to different games".into());
    }

    let mut ordered: Vec<&Action> = actions.iter().collect();
    ordered.sort_unstable_by_key(|action| action.sequence);

    if ordered[0].previous_state_hash != GENESIS_STATE_HASH {
        return Err("genesis action has invalid previous-state hash".into());
    }

    for pair in ordered.windows(2) {
        let previous = pair[0];
        let current = pair[1];

        if current.previous_state_hash != previous.resulting_state_hash {
            return Err("action state-hash chain is broken".into());
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game(id: u8) -> GameId {
        [id; 32]
    }

    fn state_hash(id: u8) -> StateHash {
        [id; 32]
    }

    fn action(
        game_id: u8,
        id: u8,
        sequence: u32,
        previous_state_hash: StateHash,
        resulting_state_hash: StateHash,
    ) -> Action {
        Action {
            game_id: game(game_id),
            id: [id; 32],
            sequence,
            previous_state_hash,
            resulting_state_hash,
            payload: vec![id],
        }
    }

    #[test]
    fn current_parameters_use_supported_version() {
        let parameters = LedgerParameters::current();

        assert_eq!(parameters.protocol_version, PROTOCOL_VERSION);
        assert_eq!(parameters.verify(), Ok(()));
    }

    #[test]
    fn unsupported_version_is_rejected() {
        let parameters = LedgerParameters {
            protocol_version: PROTOCOL_VERSION + 1,
        };

        assert_eq!(
            parameters.verify(),
            Err("unsupported protocol version".into())
        );
    }

    #[test]
    fn contiguous_sequences_are_valid_regardless_of_storage_order() {
        let actions = vec![
            action(7, 3, 2, state_hash(2), state_hash(3)),
            action(7, 1, 0, GENESIS_STATE_HASH, state_hash(1)),
            action(7, 2, 1, state_hash(1), state_hash(2)),
        ];

        assert_eq!(verify_action_sequences(&actions), Ok(()));
    }

    #[test]
    fn empty_history_is_valid() {
        assert_eq!(verify_action_history(&[]), Ok(()));
    }

    #[test]
    fn valid_hash_chain_is_accepted_regardless_of_storage_order() {
        let actions = vec![
            action(7, 3, 2, state_hash(2), state_hash(3)),
            action(7, 1, 0, GENESIS_STATE_HASH, state_hash(1)),
            action(7, 2, 1, state_hash(1), state_hash(2)),
        ];

        assert_eq!(verify_action_history(&actions), Ok(()));
    }

    #[test]
    fn sequence_must_start_at_zero() {
        let actions = vec![action(7, 1, 1, GENESIS_STATE_HASH, state_hash(1))];

        assert_eq!(
            verify_action_history(&actions),
            Err("action sequence gap".into())
        );
    }

    #[test]
    fn duplicate_sequence_is_rejected() {
        let actions = vec![
            action(7, 1, 0, GENESIS_STATE_HASH, state_hash(1)),
            action(7, 2, 0, GENESIS_STATE_HASH, state_hash(2)),
        ];

        assert_eq!(
            verify_action_history(&actions),
            Err("duplicate action sequence".into())
        );
    }

    #[test]
    fn sequence_gap_is_rejected() {
        let actions = vec![
            action(7, 1, 0, GENESIS_STATE_HASH, state_hash(1)),
            action(7, 3, 2, state_hash(1), state_hash(3)),
        ];

        assert_eq!(
            verify_action_history(&actions),
            Err("action sequence gap".into())
        );
    }

    #[test]
    fn mixed_game_ids_are_rejected() {
        let actions = vec![
            action(7, 1, 0, GENESIS_STATE_HASH, state_hash(1)),
            action(8, 2, 1, state_hash(1), state_hash(2)),
        ];

        assert_eq!(
            verify_action_history(&actions),
            Err("actions belong to different games".into())
        );
    }

    #[test]
    fn genesis_action_must_reference_genesis_hash() {
        let actions = vec![action(7, 1, 0, state_hash(9), state_hash(1))];

        assert_eq!(
            verify_action_history(&actions),
            Err("genesis action has invalid previous-state hash".into())
        );
    }

    #[test]
    fn broken_state_hash_chain_is_rejected() {
        let actions = vec![
            action(7, 1, 0, GENESIS_STATE_HASH, state_hash(1)),
            action(7, 2, 1, state_hash(9), state_hash(2)),
        ];

        assert_eq!(
            verify_action_history(&actions),
            Err("action state-hash chain is broken".into())
        );
    }
}
