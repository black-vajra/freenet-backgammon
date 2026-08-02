use std::collections::BTreeSet;

use backgammon_core::{GameState, GameStatus, Player, TurnError, TurnPhase};
use serde::{Deserialize, Serialize};

use crate::{
    CanonicalReplayState, GameActionError, GameActionPayload, GameActionRecord, GameConfiguration,
    GameId, StateHash, StateHashError, GENESIS_STATE_HASH,
};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum ReplayStatus {
    InProgress,
    Completed { winner: Player, points: u8 },
    Resigned { resigned: Player, winner: Player },
    Abandoned { player: Player },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplayedGame {
    pub game_id: GameId,
    pub configuration: GameConfiguration,
    pub state: GameState,
    pub next_sequence: u64,
    pub next_turn: u32,
    pub status: ReplayStatus,
    pub latest_state_hash: StateHash,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReplayError {
    EmptyHistory,
    InvalidAction {
        sequence: u64,
        error: GameActionError,
    },
    SequenceMustStartAtZero {
        found: u64,
    },
    SequenceGap {
        expected: u64,
        found: u64,
    },
    DuplicateActionId,
    MixedGameIds,
    GenesisHashMismatch,
    BrokenStateHashChain {
        sequence: u64,
    },
    ResultingStateHashMismatch {
        sequence: u64,
        expected: StateHash,
        found: StateHash,
    },
    StateHash {
        sequence: u64,
        error: StateHashError,
    },
    FirstActionMustCreateGame,
    DuplicateGameCreation,
    ActionAfterTerminalState {
        sequence: u64,
    },
    RollAlreadyPending,
    RollExpected,
    WrongTurnNumber {
        expected: u32,
        found: u32,
    },
    WrongPlayer {
        expected: Player,
        found: Player,
    },
    InvalidTurn {
        sequence: u64,
        error: TurnError,
    },
    TurnNumberOverflow,
    SequenceNumberOverflow,
}

impl ReplayedGame {
    pub fn canonical_state(&self) -> CanonicalReplayState {
        CanonicalReplayState::new(
            self.game_id,
            self.configuration.clone(),
            self.state.clone(),
            self.next_turn,
            self.status.clone(),
        )
    }

    pub fn canonical_hash(&self) -> Result<StateHash, StateHashError> {
        self.canonical_state().hash()
    }

    fn refresh_status_from_board(&mut self) {
        if let GameStatus::Completed { winner, points } = self.state.status {
            self.status = ReplayStatus::Completed { winner, points };
        }
    }

    fn ensure_in_progress(&self, sequence: u64) -> Result<(), ReplayError> {
        if self.status != ReplayStatus::InProgress {
            return Err(ReplayError::ActionAfterTerminalState { sequence });
        }

        Ok(())
    }

    fn apply_record(&mut self, record: &GameActionRecord) -> Result<(), ReplayError> {
        self.ensure_in_progress(record.sequence)?;

        match &record.payload {
            GameActionPayload::CreateGame(_) => Err(ReplayError::DuplicateGameCreation),

            GameActionPayload::RecordRoll { turn, player, dice } => {
                if *turn != self.next_turn {
                    return Err(ReplayError::WrongTurnNumber {
                        expected: self.next_turn,
                        found: *turn,
                    });
                }

                if *player != self.state.active_player {
                    return Err(ReplayError::WrongPlayer {
                        expected: self.state.active_player,
                        found: *player,
                    });
                }

                if self.state.turn_phase != TurnPhase::AwaitingRoll || self.state.dice.is_some() {
                    return Err(ReplayError::RollAlreadyPending);
                }

                self.state.dice = Some(*dice);
                self.state.turn_phase = TurnPhase::Moving;

                Ok(())
            }

            GameActionPayload::PlayTurn {
                turn,
                player,
                sequence,
            } => {
                if *turn != self.next_turn {
                    return Err(ReplayError::WrongTurnNumber {
                        expected: self.next_turn,
                        found: *turn,
                    });
                }

                if *player != self.state.active_player {
                    return Err(ReplayError::WrongPlayer {
                        expected: self.state.active_player,
                        found: *player,
                    });
                }

                if self.state.turn_phase != TurnPhase::Moving || self.state.dice.is_none() {
                    return Err(ReplayError::RollExpected);
                }

                self.state.apply_turn_sequence(sequence).map_err(|error| {
                    ReplayError::InvalidTurn {
                        sequence: record.sequence,
                        error,
                    }
                })?;

                self.next_turn = self
                    .next_turn
                    .checked_add(1)
                    .ok_or(ReplayError::TurnNumberOverflow)?;

                self.refresh_status_from_board();

                Ok(())
            }

            GameActionPayload::Resign { player } => {
                self.status = ReplayStatus::Resigned {
                    resigned: *player,
                    winner: player.opponent(),
                };

                Ok(())
            }

            GameActionPayload::Abandon { player } => {
                self.status = ReplayStatus::Abandoned { player: *player };

                Ok(())
            }
        }
    }
}

pub fn replay_game(records: &[GameActionRecord]) -> Result<ReplayedGame, ReplayError> {
    let first = records.first().ok_or(ReplayError::EmptyHistory)?;

    if first.sequence != 0 {
        return Err(ReplayError::SequenceMustStartAtZero {
            found: first.sequence,
        });
    }

    first.verify().map_err(|error| ReplayError::InvalidAction {
        sequence: first.sequence,
        error,
    })?;

    if first.previous_state_hash != GENESIS_STATE_HASH {
        return Err(ReplayError::GenesisHashMismatch);
    }

    let configuration = match &first.payload {
        GameActionPayload::CreateGame(configuration) => configuration.clone(),
        _ => return Err(ReplayError::FirstActionMustCreateGame),
    };

    let mut replay = ReplayedGame {
        game_id: first.game_id,
        configuration,
        state: GameState::standard_start(),
        next_sequence: 1,
        next_turn: 0,
        status: ReplayStatus::InProgress,
        latest_state_hash: first.resulting_state_hash,
    };

    let expected_first_hash = replay
        .canonical_hash()
        .map_err(|error| ReplayError::StateHash {
            sequence: first.sequence,
            error,
        })?;

    if first.resulting_state_hash != expected_first_hash {
        return Err(ReplayError::ResultingStateHashMismatch {
            sequence: first.sequence,
            expected: expected_first_hash,
            found: first.resulting_state_hash,
        });
    }

    replay.latest_state_hash = expected_first_hash;

    let mut action_ids = BTreeSet::new();
    action_ids.insert(first.action_id);

    for record in &records[1..] {
        record
            .verify()
            .map_err(|error| ReplayError::InvalidAction {
                sequence: record.sequence,
                error,
            })?;

        if record.game_id != replay.game_id {
            return Err(ReplayError::MixedGameIds);
        }

        if record.sequence != replay.next_sequence {
            return Err(ReplayError::SequenceGap {
                expected: replay.next_sequence,
                found: record.sequence,
            });
        }

        if !action_ids.insert(record.action_id) {
            return Err(ReplayError::DuplicateActionId);
        }

        if record.previous_state_hash != replay.latest_state_hash {
            return Err(ReplayError::BrokenStateHashChain {
                sequence: record.sequence,
            });
        }

        replay.apply_record(record)?;

        let expected_hash = replay
            .canonical_hash()
            .map_err(|error| ReplayError::StateHash {
                sequence: record.sequence,
                error,
            })?;

        if record.resulting_state_hash != expected_hash {
            return Err(ReplayError::ResultingStateHashMismatch {
                sequence: record.sequence,
                expected: expected_hash,
                found: record.resulting_state_hash,
            });
        }

        replay.latest_state_hash = expected_hash;
        replay.next_sequence = replay
            .next_sequence
            .checked_add(1)
            .ok_or(ReplayError::SequenceNumberOverflow)?;
    }

    Ok(replay)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ActionId, PlayerDescriptor, PROTOCOL_VERSION};
    use backgammon_core::{Dice, TurnSequence};

    fn configuration() -> GameConfiguration {
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
        }
    }

    fn bare_record(
        sequence: u64,
        action_id: u8,
        previous_state_hash: StateHash,
        resulting_state_hash: StateHash,
        payload: GameActionPayload,
    ) -> GameActionRecord {
        GameActionRecord {
            protocol_version: PROTOCOL_VERSION,
            game_id: [9; 32],
            action_id: [action_id; 32] as ActionId,
            sequence,
            previous_state_hash,
            resulting_state_hash,
            payload,
        }
    }

    fn create_record() -> GameActionRecord {
        let replay = ReplayedGame {
            game_id: [9; 32],
            configuration: configuration(),
            state: GameState::standard_start(),
            next_sequence: 1,
            next_turn: 0,
            status: ReplayStatus::InProgress,
            latest_state_hash: GENESIS_STATE_HASH,
        };

        bare_record(
            0,
            1,
            GENESIS_STATE_HASH,
            replay.canonical_hash().unwrap(),
            GameActionPayload::CreateGame(configuration()),
        )
    }

    fn append_valid(records: &mut Vec<GameActionRecord>, payload: GameActionPayload) {
        let current = replay_game(records).unwrap();
        let sequence = current.next_sequence;
        let mut next = current.clone();

        let mut record = bare_record(
            sequence,
            u8::try_from(sequence + 1).unwrap(),
            current.latest_state_hash,
            [0; 32],
            payload,
        );

        next.apply_record(&record).unwrap();
        record.resulting_state_hash = next.canonical_hash().unwrap();
        records.push(record);
    }

    fn legal_opening_sequence(dice: Dice) -> TurnSequence {
        let mut state = GameState::standard_start();
        state.dice = Some(dice);
        state.turn_phase = TurnPhase::Moving;
        state.legal_turn_sequences().unwrap()[0].clone()
    }

    #[test]
    fn create_game_replays_from_genesis() {
        let create = create_record();
        let replay = replay_game(&[create.clone()]).unwrap();

        assert_eq!(replay.game_id, [9; 32]);
        assert_eq!(replay.next_sequence, 1);
        assert_eq!(replay.next_turn, 0);
        assert_eq!(replay.status, ReplayStatus::InProgress);
        assert_eq!(replay.state, GameState::standard_start());
        assert_eq!(replay.latest_state_hash, create.resulting_state_hash);
    }

    #[test]
    fn roll_and_complete_turn_replay_deterministically() {
        let dice = Dice {
            first: 1,
            second: 2,
        };
        let sequence = legal_opening_sequence(dice);

        let mut actions = vec![create_record()];

        append_valid(
            &mut actions,
            GameActionPayload::RecordRoll {
                turn: 0,
                player: Player::White,
                dice,
            },
        );

        append_valid(
            &mut actions,
            GameActionPayload::PlayTurn {
                turn: 0,
                player: Player::White,
                sequence,
            },
        );

        let first = replay_game(&actions).unwrap();
        let second = replay_game(&actions).unwrap();

        assert_eq!(first, second);
        assert_eq!(first.next_sequence, 3);
        assert_eq!(first.next_turn, 1);
        assert_eq!(first.state.active_player, Player::Black);
        assert_eq!(first.state.turn_phase, TurnPhase::AwaitingRoll);
        assert_eq!(first.state.dice, None);
    }

    #[test]
    fn forged_create_resulting_hash_is_rejected() {
        let mut create = create_record();
        let expected = create.resulting_state_hash;
        create.resulting_state_hash = [99; 32];

        assert_eq!(
            replay_game(&[create]),
            Err(ReplayError::ResultingStateHashMismatch {
                sequence: 0,
                expected,
                found: [99; 32],
            })
        );
    }

    #[test]
    fn forged_post_action_resulting_hash_is_rejected() {
        let mut actions = vec![create_record()];

        append_valid(
            &mut actions,
            GameActionPayload::Resign {
                player: Player::White,
            },
        );

        let expected = actions[1].resulting_state_hash;
        actions[1].resulting_state_hash = [88; 32];

        assert_eq!(
            replay_game(&actions),
            Err(ReplayError::ResultingStateHashMismatch {
                sequence: 1,
                expected,
                found: [88; 32],
            })
        );
    }

    #[test]
    fn first_action_must_create_game() {
        let action = bare_record(
            0,
            1,
            GENESIS_STATE_HASH,
            [10; 32],
            GameActionPayload::RecordRoll {
                turn: 0,
                player: Player::White,
                dice: Dice {
                    first: 1,
                    second: 2,
                },
            },
        );

        assert_eq!(
            replay_game(&[action]),
            Err(ReplayError::FirstActionMustCreateGame)
        );
    }

    #[test]
    fn sequence_gap_is_rejected() {
        let create = create_record();

        let second = bare_record(
            2,
            2,
            create.resulting_state_hash,
            [11; 32],
            GameActionPayload::Resign {
                player: Player::White,
            },
        );

        assert_eq!(
            replay_game(&[create, second]),
            Err(ReplayError::SequenceGap {
                expected: 1,
                found: 2,
            })
        );
    }

    #[test]
    fn mixed_game_ids_are_rejected() {
        let create = create_record();

        let mut second = bare_record(
            1,
            2,
            create.resulting_state_hash,
            [11; 32],
            GameActionPayload::Resign {
                player: Player::White,
            },
        );
        second.game_id = [8; 32];

        assert_eq!(
            replay_game(&[create, second]),
            Err(ReplayError::MixedGameIds)
        );
    }

    #[test]
    fn broken_hash_chain_is_rejected() {
        let actions = vec![
            create_record(),
            bare_record(
                1,
                2,
                [99; 32],
                [11; 32],
                GameActionPayload::Resign {
                    player: Player::White,
                },
            ),
        ];

        assert_eq!(
            replay_game(&actions),
            Err(ReplayError::BrokenStateHashChain { sequence: 1 })
        );
    }

    #[test]
    fn wrong_turn_number_is_rejected() {
        let mut actions = vec![create_record()];
        let previous = actions[0].resulting_state_hash;

        actions.push(bare_record(
            1,
            2,
            previous,
            [11; 32],
            GameActionPayload::RecordRoll {
                turn: 1,
                player: Player::White,
                dice: Dice {
                    first: 1,
                    second: 2,
                },
            },
        ));

        assert_eq!(
            replay_game(&actions),
            Err(ReplayError::WrongTurnNumber {
                expected: 0,
                found: 1,
            })
        );
    }

    #[test]
    fn wrong_player_roll_is_rejected() {
        let mut actions = vec![create_record()];
        let previous = actions[0].resulting_state_hash;

        actions.push(bare_record(
            1,
            2,
            previous,
            [11; 32],
            GameActionPayload::RecordRoll {
                turn: 0,
                player: Player::Black,
                dice: Dice {
                    first: 1,
                    second: 2,
                },
            },
        ));

        assert_eq!(
            replay_game(&actions),
            Err(ReplayError::WrongPlayer {
                expected: Player::White,
                found: Player::Black,
            })
        );
    }

    #[test]
    fn turn_without_roll_is_rejected() {
        let mut actions = vec![create_record()];
        let previous = actions[0].resulting_state_hash;

        actions.push(bare_record(
            1,
            2,
            previous,
            [11; 32],
            GameActionPayload::PlayTurn {
                turn: 0,
                player: Player::White,
                sequence: TurnSequence::default(),
            },
        ));

        assert_eq!(replay_game(&actions), Err(ReplayError::RollExpected));
    }

    #[test]
    fn second_roll_before_turn_is_rejected() {
        let dice = Dice {
            first: 1,
            second: 2,
        };
        let mut actions = vec![create_record()];

        append_valid(
            &mut actions,
            GameActionPayload::RecordRoll {
                turn: 0,
                player: Player::White,
                dice,
            },
        );

        let current = replay_game(&actions).unwrap();

        actions.push(bare_record(
            current.next_sequence,
            3,
            current.latest_state_hash,
            [12; 32],
            GameActionPayload::RecordRoll {
                turn: 0,
                player: Player::White,
                dice,
            },
        ));

        assert_eq!(replay_game(&actions), Err(ReplayError::RollAlreadyPending));
    }

    #[test]
    fn action_after_resignation_is_rejected() {
        let mut actions = vec![create_record()];

        append_valid(
            &mut actions,
            GameActionPayload::Resign {
                player: Player::White,
            },
        );

        let current = replay_game(&actions).unwrap();

        actions.push(bare_record(
            current.next_sequence,
            3,
            current.latest_state_hash,
            [12; 32],
            GameActionPayload::Abandon {
                player: Player::Black,
            },
        ));

        assert_eq!(
            replay_game(&actions),
            Err(ReplayError::ActionAfterTerminalState { sequence: 2 })
        );
    }

    #[test]
    fn duplicate_action_id_is_rejected() {
        let create = create_record();

        let second = bare_record(
            1,
            1,
            create.resulting_state_hash,
            [11; 32],
            GameActionPayload::Resign {
                player: Player::White,
            },
        );

        assert_eq!(
            replay_game(&[create, second]),
            Err(ReplayError::DuplicateActionId)
        );
    }
}
