use std::collections::BTreeSet;

use backgammon_core::{GameState, GameStatus, Player, TurnError, TurnPhase};
use serde::{Deserialize, Serialize};

use crate::{
    verify_and_derive_dice, CanonicalReplayState, DiceCommit, DiceReveal, DiceRoundState,
    FairDiceError, GameActionError, GameActionPayload, GameActionRecord, GameConfiguration, GameId,
    StateHash, StateHashError, GENESIS_STATE_HASH,
};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum ReplayStatus {
    InProgress,
    Completed { winner: Player, points: u8 },
    Resigned { resigned: Player, winner: Player },
    Abandoned { player: Player },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplayedTurn {
    pub turn: u32,
    pub player: Player,
    pub dice: backgammon_core::Dice,
    pub sequence: backgammon_core::TurnSequence,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplayedGame {
    pub game_id: GameId,
    pub configuration: GameConfiguration,
    pub state: GameState,
    pub next_sequence: u64,
    pub next_turn: u32,
    pub completed_turns: Vec<ReplayedTurn>,
    pub roll_requested_by: Option<Player>,
    pub dice_round: DiceRoundState,
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
    RollAlreadyRequested,
    RollNotRequested,
    DiceCommitmentAlreadyPresent {
        player: Player,
    },
    DiceRevealBeforeBothCommitments,
    DiceRevealAlreadyPresent {
        player: Player,
    },
    FairDice {
        sequence: u64,
        error: FairDiceError,
    },
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
        let mut canonical = CanonicalReplayState::new(
            self.game_id,
            self.configuration.clone(),
            self.state.clone(),
            self.next_turn,
            self.dice_round.clone(),
            self.status.clone(),
        );

        canonical.roll_requested_by = self.roll_requested_by;
        canonical
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

            GameActionPayload::RequestRoll { turn, player } => {
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

                if self.roll_requested_by.is_some() || !self.dice_round.is_empty() {
                    return Err(ReplayError::RollAlreadyRequested);
                }

                self.roll_requested_by = Some(*player);

                Ok(())
            }

            GameActionPayload::CommitDice {
                turn,
                player,
                commitment,
            } => {
                if *turn != self.next_turn {
                    return Err(ReplayError::WrongTurnNumber {
                        expected: self.next_turn,
                        found: *turn,
                    });
                }

                if self.state.turn_phase != TurnPhase::AwaitingRoll || self.state.dice.is_some() {
                    return Err(ReplayError::RollAlreadyPending);
                }

                if self.roll_requested_by != Some(self.state.active_player) {
                    return Err(ReplayError::RollNotRequested);
                }

                let slot = match player {
                    Player::White => &mut self.dice_round.white_commitment,
                    Player::Black => &mut self.dice_round.black_commitment,
                };

                if slot.is_some() {
                    return Err(ReplayError::DiceCommitmentAlreadyPresent { player: *player });
                }

                *slot = Some(*commitment);

                Ok(())
            }

            GameActionPayload::RevealDice {
                turn,
                player,
                secret,
            } => {
                if *turn != self.next_turn {
                    return Err(ReplayError::WrongTurnNumber {
                        expected: self.next_turn,
                        found: *turn,
                    });
                }

                if self.state.turn_phase != TurnPhase::AwaitingRoll || self.state.dice.is_some() {
                    return Err(ReplayError::RollAlreadyPending);
                }

                if self.roll_requested_by != Some(self.state.active_player) {
                    return Err(ReplayError::RollNotRequested);
                }

                let (white_commitment, black_commitment) = match (
                    self.dice_round.white_commitment,
                    self.dice_round.black_commitment,
                ) {
                    (Some(white), Some(black)) => (white, black),
                    _ => {
                        return Err(ReplayError::DiceRevealBeforeBothCommitments);
                    }
                };

                let already_revealed = match player {
                    Player::White => self.dice_round.white_reveal.is_some(),
                    Player::Black => self.dice_round.black_reveal.is_some(),
                };

                if already_revealed {
                    return Err(ReplayError::DiceRevealAlreadyPresent { player: *player });
                }

                let player_commitment = match player {
                    Player::White => white_commitment,
                    Player::Black => black_commitment,
                };

                let commitment = DiceCommit {
                    turn: *turn,
                    player: *player,
                    commitment: player_commitment,
                };

                let reveal = DiceReveal {
                    turn: *turn,
                    player: *player,
                    secret: *secret,
                };

                commitment
                    .verify_reveal(&self.game_id, &reveal)
                    .map_err(|error| ReplayError::FairDice {
                        sequence: record.sequence,
                        error,
                    })?;

                match player {
                    Player::White => {
                        self.dice_round.white_reveal = Some(*secret);
                    }
                    Player::Black => {
                        self.dice_round.black_reveal = Some(*secret);
                    }
                }

                if let (Some(white_secret), Some(black_secret)) =
                    (self.dice_round.white_reveal, self.dice_round.black_reveal)
                {
                    let white_commit = DiceCommit {
                        turn: *turn,
                        player: Player::White,
                        commitment: white_commitment,
                    };

                    let black_commit = DiceCommit {
                        turn: *turn,
                        player: Player::Black,
                        commitment: black_commitment,
                    };

                    let white_reveal = DiceReveal {
                        turn: *turn,
                        player: Player::White,
                        secret: white_secret,
                    };

                    let black_reveal = DiceReveal {
                        turn: *turn,
                        player: Player::Black,
                        secret: black_secret,
                    };

                    let dice = verify_and_derive_dice(
                        &self.game_id,
                        *turn,
                        &white_commit,
                        &black_commit,
                        &white_reveal,
                        &black_reveal,
                    )
                    .map_err(|error| ReplayError::FairDice {
                        sequence: record.sequence,
                        error,
                    })?;

                    self.state.dice = Some(dice);
                    self.state.turn_phase = TurnPhase::Moving;
                }

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

                let dice = self.state.dice.ok_or(ReplayError::RollExpected)?;

                self.state.apply_turn_sequence(sequence).map_err(|error| {
                    ReplayError::InvalidTurn {
                        sequence: record.sequence,
                        error,
                    }
                })?;

                self.completed_turns.push(ReplayedTurn {
                    turn: *turn,
                    player: *player,
                    dice,
                    sequence: sequence.clone(),
                });

                self.dice_round.clear();
                self.roll_requested_by = None;

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

/// Builds one canonical action extending an already verified game history.
///
/// The existing history is replayed first. The candidate payload is then
/// applied to a cloned replay state so that its sequence number,
/// previous-state hash, and resulting-state hash are derived rather than
/// trusted from the caller.
///
/// This function does not mutate the supplied history.
pub fn build_next_game_action(
    records: &[GameActionRecord],
    action_id: crate::ActionId,
    payload: GameActionPayload,
) -> Result<GameActionRecord, ReplayError> {
    let current = replay_game(records)?;

    if records.iter().any(|record| record.action_id == action_id) {
        return Err(ReplayError::DuplicateActionId);
    }

    let sequence = current.next_sequence;

    /*
     * The replicated ledger currently stores its sequence as u32 even
     * though the typed action envelope exposes u64. Reject an action that
     * could not subsequently be represented by the ledger.
     */
    u32::try_from(sequence).map_err(|_| ReplayError::SequenceNumberOverflow)?;

    let mut record = GameActionRecord {
        protocol_version: crate::PROTOCOL_VERSION,
        game_id: current.game_id,
        action_id,
        sequence,
        previous_state_hash: current.latest_state_hash,
        resulting_state_hash: [0_u8; 32],
        payload,
    };

    record
        .verify()
        .map_err(|error| ReplayError::InvalidAction { sequence, error })?;

    let mut next = current;
    next.apply_record(&record)?;

    record.resulting_state_hash = next
        .canonical_hash()
        .map_err(|error| ReplayError::StateHash { sequence, error })?;

    /*
     * Replay the complete candidate history once more. This independently
     * verifies sequence ordering, action-ID uniqueness, the hash chain,
     * payload legality, and the derived resulting-state hash.
     */
    let mut candidate_history = records.to_vec();
    candidate_history.push(record.clone());
    replay_game(&candidate_history)?;

    Ok(record)
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
        completed_turns: Vec::new(),
        roll_requested_by: None,
        dice_round: DiceRoundState::default(),
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
            completed_turns: Vec::new(),
            roll_requested_by: None,
            dice_round: DiceRoundState::default(),
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

    fn append_fair_roll(records: &mut Vec<GameActionRecord>, turn: u32) -> Dice {
        let game_id = records[0].game_id;
        let white_secret = [11; 32];
        let black_secret = [22; 32];

        let white_commit = DiceCommit::new(&game_id, turn, Player::White, &white_secret);

        let black_commit = DiceCommit::new(&game_id, turn, Player::Black, &black_secret);

        let active_player = replay_game(records).unwrap().state.active_player;

        append_valid(
            records,
            GameActionPayload::RequestRoll {
                turn,
                player: active_player,
            },
        );

        append_valid(
            records,
            GameActionPayload::CommitDice {
                turn,
                player: Player::White,
                commitment: white_commit.commitment,
            },
        );

        append_valid(
            records,
            GameActionPayload::CommitDice {
                turn,
                player: Player::Black,
                commitment: black_commit.commitment,
            },
        );

        append_valid(
            records,
            GameActionPayload::RevealDice {
                turn,
                player: Player::White,
                secret: white_secret,
            },
        );

        append_valid(
            records,
            GameActionPayload::RevealDice {
                turn,
                player: Player::Black,
                secret: black_secret,
            },
        );

        crate::derive_dice(&game_id, turn, &white_secret, &black_secret).unwrap()
    }

    #[test]
    fn next_action_builder_derives_sequence_and_hash() {
        let create = create_record();

        let resign = build_next_game_action(
            std::slice::from_ref(&create),
            [42; 32],
            GameActionPayload::Resign {
                player: Player::White,
            },
        )
        .unwrap();

        assert_eq!(resign.game_id, create.game_id);
        assert_eq!(resign.sequence, 1);
        assert_eq!(resign.previous_state_hash, create.resulting_state_hash);

        let replay = replay_game(&[create, resign.clone()]).unwrap();

        assert_eq!(
            replay.status,
            ReplayStatus::Resigned {
                resigned: Player::White,
                winner: Player::Black,
            }
        );
        assert_eq!(resign.resulting_state_hash, replay.latest_state_hash);
        assert_eq!(replay.next_sequence, 2);
    }

    #[test]
    fn next_action_builder_rejects_duplicate_action_id() {
        let create = create_record();

        assert_eq!(
            build_next_game_action(
                std::slice::from_ref(&create),
                create.action_id,
                GameActionPayload::Resign {
                    player: Player::White,
                },
            ),
            Err(ReplayError::DuplicateActionId)
        );
    }

    #[test]
    fn next_action_builder_rejects_illegal_transition() {
        let create = create_record();

        assert_eq!(
            build_next_game_action(
                std::slice::from_ref(&create),
                [43; 32],
                GameActionPayload::PlayTurn {
                    turn: 0,
                    player: Player::White,
                    sequence: Default::default(),
                },
            ),
            Err(ReplayError::RollExpected)
        );
    }

    #[test]
    fn active_player_can_request_roll() {
        let create = create_record();

        let request = build_next_game_action(
            std::slice::from_ref(&create),
            [44; 32],
            GameActionPayload::RequestRoll {
                turn: 0,
                player: Player::White,
            },
        )
        .unwrap();

        let replay = replay_game(&[create, request]).unwrap();

        assert_eq!(replay.roll_requested_by, Some(Player::White));
        assert_eq!(replay.next_turn, 0);
        assert_eq!(replay.state.turn_phase, TurnPhase::AwaitingRoll);
        assert_eq!(replay.state.dice, None);
    }

    #[test]
    fn inactive_player_cannot_request_roll() {
        let create = create_record();

        assert_eq!(
            build_next_game_action(
                std::slice::from_ref(&create),
                [45; 32],
                GameActionPayload::RequestRoll {
                    turn: 0,
                    player: Player::Black,
                },
            ),
            Err(ReplayError::WrongPlayer {
                expected: Player::White,
                found: Player::Black,
            })
        );
    }

    #[test]
    fn commitment_before_roll_request_is_rejected() {
        let create = create_record();

        let commitment = DiceCommit::new(&create.game_id, 0, Player::White, &[11; 32]);

        assert_eq!(
            build_next_game_action(
                std::slice::from_ref(&create),
                [46; 32],
                GameActionPayload::CommitDice {
                    turn: 0,
                    player: Player::White,
                    commitment: commitment.commitment,
                },
            ),
            Err(ReplayError::RollNotRequested)
        );
    }

    #[test]
    fn duplicate_roll_request_is_rejected() {
        let create = create_record();

        let request = build_next_game_action(
            std::slice::from_ref(&create),
            [47; 32],
            GameActionPayload::RequestRoll {
                turn: 0,
                player: Player::White,
            },
        )
        .unwrap();

        assert_eq!(
            build_next_game_action(
                &[create, request],
                [48; 32],
                GameActionPayload::RequestRoll {
                    turn: 0,
                    player: Player::White,
                },
            ),
            Err(ReplayError::RollAlreadyRequested)
        );
    }

    #[test]
    fn fair_dice_actions_derive_roll_deterministically() {
        let mut actions = vec![create_record()];
        let expected = append_fair_roll(&mut actions, 0);

        let replay = replay_game(&actions).unwrap();

        assert_eq!(replay.next_sequence, 6);
        assert_eq!(replay.next_turn, 0);
        assert_eq!(replay.state.dice, Some(expected));
        assert_eq!(replay.state.turn_phase, TurnPhase::Moving);
        assert!(!replay.dice_round.is_empty());
    }

    #[test]
    fn reveal_before_both_commitments_is_rejected() {
        let mut actions = vec![create_record()];
        let game_id = actions[0].game_id;
        let secret = [11; 32];

        let commitment = DiceCommit::new(&game_id, 0, Player::White, &secret);

        append_valid(
            &mut actions,
            GameActionPayload::RequestRoll {
                turn: 0,
                player: Player::White,
            },
        );

        append_valid(
            &mut actions,
            GameActionPayload::CommitDice {
                turn: 0,
                player: Player::White,
                commitment: commitment.commitment,
            },
        );

        let current = replay_game(&actions).unwrap();

        actions.push(bare_record(
            current.next_sequence,
            9,
            current.latest_state_hash,
            [0; 32],
            GameActionPayload::RevealDice {
                turn: 0,
                player: Player::White,
                secret,
            },
        ));

        assert_eq!(
            replay_game(&actions),
            Err(ReplayError::DiceRevealBeforeBothCommitments)
        );
    }

    #[test]
    fn mismatched_dice_reveal_is_rejected() {
        let mut actions = vec![create_record()];
        let game_id = actions[0].game_id;

        let white_commit = DiceCommit::new(&game_id, 0, Player::White, &[11; 32]);

        let black_commit = DiceCommit::new(&game_id, 0, Player::Black, &[22; 32]);

        append_valid(
            &mut actions,
            GameActionPayload::RequestRoll {
                turn: 0,
                player: Player::White,
            },
        );

        append_valid(
            &mut actions,
            GameActionPayload::CommitDice {
                turn: 0,
                player: Player::White,
                commitment: white_commit.commitment,
            },
        );

        append_valid(
            &mut actions,
            GameActionPayload::CommitDice {
                turn: 0,
                player: Player::Black,
                commitment: black_commit.commitment,
            },
        );

        let current = replay_game(&actions).unwrap();

        actions.push(bare_record(
            current.next_sequence,
            9,
            current.latest_state_hash,
            [0; 32],
            GameActionPayload::RevealDice {
                turn: 0,
                player: Player::White,
                secret: [99; 32],
            },
        ));

        assert!(matches!(
            replay_game(&actions),
            Err(ReplayError::FairDice {
                sequence: 4,
                error: FairDiceError::CommitmentMismatch(Player::White),
            })
        ));
    }

    #[test]
    fn duplicate_dice_commitment_is_rejected() {
        let mut actions = vec![create_record()];
        let game_id = actions[0].game_id;
        let secret = [11; 32];

        let commitment = DiceCommit::new(&game_id, 0, Player::White, &secret);

        append_valid(
            &mut actions,
            GameActionPayload::RequestRoll {
                turn: 0,
                player: Player::White,
            },
        );

        append_valid(
            &mut actions,
            GameActionPayload::CommitDice {
                turn: 0,
                player: Player::White,
                commitment: commitment.commitment,
            },
        );

        let current = replay_game(&actions).unwrap();

        actions.push(bare_record(
            current.next_sequence,
            9,
            current.latest_state_hash,
            [0; 32],
            GameActionPayload::CommitDice {
                turn: 0,
                player: Player::White,
                commitment: commitment.commitment,
            },
        ));

        assert_eq!(
            replay_game(&actions),
            Err(ReplayError::DiceCommitmentAlreadyPresent {
                player: Player::White,
            })
        );
    }

    #[test]
    fn completed_turn_clears_fair_dice_round() {
        let mut actions = vec![create_record()];
        append_fair_roll(&mut actions, 0);

        let current = replay_game(&actions).unwrap();

        let sequence = current.state.legal_turn_sequences().unwrap()[0].clone();

        append_valid(
            &mut actions,
            GameActionPayload::PlayTurn {
                turn: 0,
                player: Player::White,
                sequence,
            },
        );

        let replay = replay_game(&actions).unwrap();

        assert_eq!(replay.next_sequence, 7);
        assert_eq!(replay.next_turn, 1);
        assert!(replay.dice_round.is_empty());
        assert_eq!(replay.roll_requested_by, None);
        assert_eq!(replay.state.dice, None);
        assert_eq!(replay.state.turn_phase, TurnPhase::AwaitingRoll);
    }

    #[test]
    fn completed_turn_history_is_derived_from_verified_replay() {
        let mut actions = vec![create_record()];
        let dice = append_fair_roll(&mut actions, 0);
        let current = replay_game(&actions).unwrap();
        let sequence = current.state.legal_turn_sequences().unwrap()[0].clone();

        append_valid(
            &mut actions,
            GameActionPayload::PlayTurn {
                turn: 0,
                player: Player::White,
                sequence: sequence.clone(),
            },
        );

        let replay = replay_game(&actions).unwrap();

        assert_eq!(replay.completed_turns.len(), 1);
        assert_eq!(replay.completed_turns[0].turn, 0);
        assert_eq!(replay.completed_turns[0].player, Player::White);
        assert_eq!(replay.completed_turns[0].dice, dice);
        assert_eq!(replay.completed_turns[0].sequence, sequence);
    }

    #[test]
    fn completed_turn_history_does_not_change_canonical_hash() {
        let mut actions = vec![create_record()];
        append_fair_roll(&mut actions, 0);
        let current = replay_game(&actions).unwrap();
        let sequence = current.state.legal_turn_sequences().unwrap()[0].clone();

        append_valid(
            &mut actions,
            GameActionPayload::PlayTurn {
                turn: 0,
                player: Player::White,
                sequence,
            },
        );

        let replay = replay_game(&actions).unwrap();
        let expected = replay.canonical_hash().unwrap();
        let mut metadata_changed = replay.clone();

        metadata_changed.completed_turns.clear();

        assert_eq!(metadata_changed.canonical_hash().unwrap(), expected);
    }

    #[test]
    fn create_game_replays_from_genesis() {
        let create = create_record();
        let replay = replay_game(&[create.clone()]).unwrap();

        assert_eq!(replay.game_id, [9; 32]);
        assert_eq!(replay.next_sequence, 1);
        assert_eq!(replay.next_turn, 0);
        assert_eq!(replay.status, ReplayStatus::InProgress);
        assert_eq!(replay.roll_requested_by, None);
        assert_eq!(replay.state, GameState::standard_start());
        assert_eq!(replay.latest_state_hash, create.resulting_state_hash);
    }

    #[test]
    fn fair_dice_and_complete_turn_replay_deterministically() {
        let mut actions = vec![create_record()];
        let dice = append_fair_roll(&mut actions, 0);
        let sequence = legal_opening_sequence(dice);

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
        assert_eq!(first.next_sequence, 7);
        assert_eq!(first.next_turn, 1);
        assert_eq!(first.state.active_player, Player::Black);
        assert_eq!(first.state.turn_phase, TurnPhase::AwaitingRoll);
        assert_eq!(first.state.dice, None);
        assert!(first.dice_round.is_empty());
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
        let first = bare_record(
            0,
            1,
            GENESIS_STATE_HASH,
            [10; 32],
            GameActionPayload::CommitDice {
                turn: 0,
                player: Player::White,
                commitment: [7; 32],
            },
        );

        assert_eq!(
            replay_game(&[first]),
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
    fn wrong_dice_commitment_turn_is_rejected() {
        let mut actions = vec![create_record()];
        let game_id = actions[0].game_id;
        let previous = actions[0].resulting_state_hash;
        let secret = [11; 32];

        let commitment = DiceCommit::new(&game_id, 1, Player::White, &secret);

        actions.push(bare_record(
            1,
            2,
            previous,
            [11; 32],
            GameActionPayload::CommitDice {
                turn: 1,
                player: Player::White,
                commitment: commitment.commitment,
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
    fn wrong_player_turn_is_rejected() {
        let mut actions = vec![create_record()];
        append_fair_roll(&mut actions, 0);

        let current = replay_game(&actions).unwrap();

        actions.push(bare_record(
            current.next_sequence,
            9,
            current.latest_state_hash,
            [12; 32],
            GameActionPayload::PlayTurn {
                turn: 0,
                player: Player::Black,
                sequence: TurnSequence { moves: Vec::new() },
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
    fn fair_dice_action_after_roll_is_rejected() {
        let mut actions = vec![create_record()];
        append_fair_roll(&mut actions, 0);

        let current = replay_game(&actions).unwrap();
        let secret = [33; 32];

        let commitment = DiceCommit::new(&current.game_id, 0, Player::White, &secret);

        actions.push(bare_record(
            current.next_sequence,
            9,
            current.latest_state_hash,
            [12; 32],
            GameActionPayload::CommitDice {
                turn: 0,
                player: Player::White,
                commitment: commitment.commitment,
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
