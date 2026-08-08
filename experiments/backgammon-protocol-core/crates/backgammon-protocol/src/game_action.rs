use backgammon_core::{CheckerMove, Player, TurnSequence};
use serde::{Deserialize, Serialize};

use crate::{ActionId, DiceCommitment, DiceSecret, GameId, StateHash, PROTOCOL_VERSION};

pub const MAX_DISPLAY_NAME_BYTES: usize = 48;
pub const MAX_MATCH_LENGTH: u16 = 25;

pub type PlayerId = [u8; 32];

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct PlayerDescriptor {
    pub id: PlayerId,
    pub display_name: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct GameConfiguration {
    pub white: PlayerDescriptor,
    pub black: PlayerDescriptor,
    pub match_length: u16,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum GameActionPayload {
    CreateGame(GameConfiguration),

    RequestRoll {
        turn: u32,
        player: Player,
    },

    CommitDice {
        turn: u32,
        player: Player,
        commitment: DiceCommitment,
    },

    RevealDice {
        turn: u32,
        player: Player,
        secret: DiceSecret,
    },

    PlayTurn {
        turn: u32,
        player: Player,
        sequence: TurnSequence,
    },

    Resign {
        player: Player,
    },

    Abandon {
        player: Player,
    },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct GameActionRecord {
    pub protocol_version: u16,
    pub game_id: GameId,
    pub action_id: ActionId,
    pub sequence: u64,
    pub previous_state_hash: StateHash,
    pub resulting_state_hash: StateHash,
    pub payload: GameActionPayload,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GameActionError {
    UnsupportedProtocolVersion(u16),
    EmptyDisplayName(Player),
    DisplayNameTooLong(Player),
    DuplicatePlayerIdentity,
    InvalidMatchLength,
    MoveOwnedByWrongPlayer {
        expected: Player,
        checker_move: CheckerMove,
    },
}

impl PlayerDescriptor {
    fn verify_for_player(&self, player: Player) -> Result<(), GameActionError> {
        if self.display_name.trim().is_empty() {
            return Err(GameActionError::EmptyDisplayName(player));
        }

        if self.display_name.len() > MAX_DISPLAY_NAME_BYTES {
            return Err(GameActionError::DisplayNameTooLong(player));
        }

        Ok(())
    }
}

impl GameConfiguration {
    pub fn verify(&self) -> Result<(), GameActionError> {
        self.white.verify_for_player(Player::White)?;
        self.black.verify_for_player(Player::Black)?;

        if self.white.id == self.black.id {
            return Err(GameActionError::DuplicatePlayerIdentity);
        }

        if self.match_length == 0 || self.match_length > MAX_MATCH_LENGTH {
            return Err(GameActionError::InvalidMatchLength);
        }

        Ok(())
    }
}

impl GameActionPayload {
    pub fn verify(&self) -> Result<(), GameActionError> {
        match self {
            Self::CreateGame(configuration) => configuration.verify(),

            Self::RequestRoll { .. } | Self::CommitDice { .. } | Self::RevealDice { .. } => Ok(()),

            Self::PlayTurn {
                player, sequence, ..
            } => {
                for checker_move in &sequence.moves {
                    if checker_move.player != *player {
                        return Err(GameActionError::MoveOwnedByWrongPlayer {
                            expected: *player,
                            checker_move: *checker_move,
                        });
                    }
                }

                Ok(())
            }

            Self::Resign { .. } | Self::Abandon { .. } => Ok(()),
        }
    }
}

impl GameActionRecord {
    pub fn verify(&self) -> Result<(), GameActionError> {
        if self.protocol_version != PROTOCOL_VERSION {
            return Err(GameActionError::UnsupportedProtocolVersion(
                self.protocol_version,
            ));
        }

        self.payload.verify()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use backgammon_core::MoveSource;

    fn player(id_byte: u8, display_name: &str) -> PlayerDescriptor {
        PlayerDescriptor {
            id: [id_byte; 32],
            display_name: display_name.to_owned(),
        }
    }

    fn create_payload() -> GameActionPayload {
        GameActionPayload::CreateGame(GameConfiguration {
            white: player(1, "White Player"),
            black: player(2, "Black Player"),
            match_length: 5,
        })
    }

    fn record(payload: GameActionPayload) -> GameActionRecord {
        GameActionRecord {
            protocol_version: PROTOCOL_VERSION,
            game_id: [3; 32],
            action_id: [4; 32],
            sequence: 0,
            previous_state_hash: [0; 32],
            resulting_state_hash: [5; 32],
            payload,
        }
    }

    #[test]
    fn valid_game_creation_is_accepted() {
        assert_eq!(record(create_payload()).verify(), Ok(()));
    }

    #[test]
    fn unsupported_record_version_is_rejected() {
        let mut action = record(create_payload());
        action.protocol_version = PROTOCOL_VERSION + 1;

        assert_eq!(
            action.verify(),
            Err(GameActionError::UnsupportedProtocolVersion(
                PROTOCOL_VERSION + 1
            ))
        );
    }

    #[test]
    fn empty_display_name_is_rejected() {
        let payload = GameActionPayload::CreateGame(GameConfiguration {
            white: player(1, "   "),
            black: player(2, "Black"),
            match_length: 1,
        });

        assert_eq!(
            payload.verify(),
            Err(GameActionError::EmptyDisplayName(Player::White))
        );
    }

    #[test]
    fn oversized_display_name_is_rejected() {
        let payload = GameActionPayload::CreateGame(GameConfiguration {
            white: player(1, &"x".repeat(MAX_DISPLAY_NAME_BYTES + 1)),
            black: player(2, "Black"),
            match_length: 1,
        });

        assert_eq!(
            payload.verify(),
            Err(GameActionError::DisplayNameTooLong(Player::White))
        );
    }

    #[test]
    fn duplicate_player_identity_is_rejected() {
        let payload = GameActionPayload::CreateGame(GameConfiguration {
            white: player(7, "White"),
            black: player(7, "Black"),
            match_length: 1,
        });

        assert_eq!(
            payload.verify(),
            Err(GameActionError::DuplicatePlayerIdentity)
        );
    }

    #[test]
    fn invalid_match_lengths_are_rejected() {
        for match_length in [0, MAX_MATCH_LENGTH + 1] {
            let payload = GameActionPayload::CreateGame(GameConfiguration {
                white: player(1, "White"),
                black: player(2, "Black"),
                match_length,
            });

            assert_eq!(payload.verify(), Err(GameActionError::InvalidMatchLength));
        }
    }

    #[test]
    fn roll_request_action_is_accepted() {
        let payload = GameActionPayload::RequestRoll {
            turn: 3,
            player: Player::Black,
        };

        assert_eq!(record(payload).verify(), Ok(()));
    }

    #[test]
    fn dice_commitment_action_is_accepted() {
        let payload = GameActionPayload::CommitDice {
            turn: 0,
            player: Player::White,
            commitment: [7; 32],
        };

        assert_eq!(record(payload).verify(), Ok(()));
    }

    #[test]
    fn dice_reveal_action_is_accepted() {
        let payload = GameActionPayload::RevealDice {
            turn: 0,
            player: Player::Black,
            secret: [9; 32],
        };

        assert_eq!(record(payload).verify(), Ok(()));
    }

    #[test]
    fn fair_dice_actions_round_trip_canonically() {
        for payload in [
            GameActionPayload::CommitDice {
                turn: 4,
                player: Player::White,
                commitment: [11; 32],
            },
            GameActionPayload::RevealDice {
                turn: 4,
                player: Player::Black,
                secret: [22; 32],
            },
        ] {
            let mut encoded = Vec::new();
            ciborium::ser::into_writer(&payload, &mut encoded).unwrap();

            let decoded: GameActionPayload = ciborium::de::from_reader(encoded.as_slice()).unwrap();

            assert_eq!(decoded, payload);
        }
    }

    #[test]
    fn play_turn_accepts_moves_owned_by_declared_player() {
        let payload = GameActionPayload::PlayTurn {
            turn: 4,
            player: Player::White,
            sequence: TurnSequence {
                moves: vec![CheckerMove {
                    player: Player::White,
                    source: MoveSource::Point(0),
                    die: 3,
                }],
            },
        };

        assert_eq!(payload.verify(), Ok(()));
    }

    #[test]
    fn play_turn_rejects_move_owned_by_other_player() {
        let checker_move = CheckerMove {
            player: Player::Black,
            source: MoveSource::Point(23),
            die: 2,
        };

        let payload = GameActionPayload::PlayTurn {
            turn: 4,
            player: Player::White,
            sequence: TurnSequence {
                moves: vec![checker_move],
            },
        };

        assert_eq!(
            payload.verify(),
            Err(GameActionError::MoveOwnedByWrongPlayer {
                expected: Player::White,
                checker_move,
            })
        );
    }

    #[test]
    fn blocked_turn_may_have_empty_move_sequence() {
        let payload = GameActionPayload::PlayTurn {
            turn: 3,
            player: Player::Black,
            sequence: TurnSequence::default(),
        };

        assert_eq!(payload.verify(), Ok(()));
    }
}
