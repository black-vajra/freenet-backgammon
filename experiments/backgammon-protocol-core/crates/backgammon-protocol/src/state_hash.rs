use backgammon_core::{GameState, Player};
use serde::{Deserialize, Serialize};

use crate::{DiceRoundState, GameConfiguration, GameId, ReplayStatus, StateHash, PROTOCOL_VERSION};

const STATE_HASH_DOMAIN: &[u8] = b"freenet-backgammon-replay-state-v3\0";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct CanonicalReplayState {
    pub protocol_version: u16,
    pub game_id: GameId,
    pub configuration: GameConfiguration,
    pub state: GameState,
    pub next_turn: u32,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roll_requested_by: Option<Player>,

    pub dice_round: DiceRoundState,
    pub status: ReplayStatus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StateHashError {
    CanonicalEncodingFailed,
}

impl CanonicalReplayState {
    pub fn new(
        game_id: GameId,
        configuration: GameConfiguration,
        state: GameState,
        next_turn: u32,
        dice_round: DiceRoundState,
        status: ReplayStatus,
    ) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            game_id,
            configuration,
            state,
            next_turn,
            roll_requested_by: None,
            dice_round,
            status,
        }
    }

    pub fn encode_canonical(&self) -> Result<Vec<u8>, StateHashError> {
        /*
         * This representation intentionally contains only structs,
         * enums, arrays, integers, strings, and vectors. It contains no
         * unordered maps, so field and element order are fixed by the
         * protocol data types and serde representation.
         */
        let mut encoded = Vec::new();

        ciborium::ser::into_writer(self, &mut encoded)
            .map_err(|_| StateHashError::CanonicalEncodingFailed)?;

        Ok(encoded)
    }

    pub fn hash(&self) -> Result<StateHash, StateHashError> {
        let encoded = self.encode_canonical()?;

        let mut hasher = blake3::Hasher::new();
        hasher.update(STATE_HASH_DOMAIN);
        hasher.update(&encoded);

        Ok(*hasher.finalize().as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PlayerDescriptor;
    use backgammon_core::Player;

    fn snapshot() -> CanonicalReplayState {
        CanonicalReplayState::new(
            [9; 32],
            GameConfiguration {
                white: PlayerDescriptor {
                    id: [1; 32],
                    display_name: "White".to_owned(),
                },
                black: PlayerDescriptor {
                    id: [2; 32],
                    display_name: "Black".to_owned(),
                },
                match_length: 1,
            },
            GameState::standard_start(),
            0,
            DiceRoundState::default(),
            ReplayStatus::InProgress,
        )
    }

    #[test]
    fn identical_snapshots_have_identical_encoding_and_hash() {
        let first = snapshot();
        let second = snapshot();

        assert_eq!(first.encode_canonical(), second.encode_canonical());
        assert_eq!(first.hash(), second.hash());
    }

    #[test]
    fn canonical_encoding_matches_v3_golden_fixture() {
        let expected = include_bytes!("../tests/fixtures/canonical-replay-state-v3.cbor");

        let encoded = snapshot().encode_canonical().unwrap();

        assert_eq!(encoded.as_slice(), expected);
    }

    #[test]
    fn canonical_hash_matches_v3_golden_fixture() {
        let expected = include_bytes!("../tests/fixtures/canonical-replay-state-v3.blake3");

        assert_eq!(expected.len(), 32);
        assert_eq!(snapshot().hash().unwrap().as_slice(), expected);
    }

    #[test]
    fn roll_request_changes_canonical_hash() {
        let first = snapshot();
        let mut second = snapshot();

        second.roll_requested_by = Some(Player::White);

        assert_ne!(first.hash().unwrap(), second.hash().unwrap());
    }

    #[test]
    fn meaningful_state_change_changes_hash() {
        let first = snapshot();
        let mut second = snapshot();
        second.state.active_player = Player::Black;

        assert_ne!(first.hash().unwrap(), second.hash().unwrap());
    }
}
