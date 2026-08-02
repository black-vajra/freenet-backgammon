use backgammon_core::{Dice, Player};
use serde::{Deserialize, Serialize};

use crate::GameId;

const COMMITMENT_DOMAIN: &[u8] = b"freenet-backgammon-dice-commitment-v1\0";
const ROLL_DOMAIN: &[u8] = b"freenet-backgammon-dice-roll-v1\0";

pub type DiceSecret = [u8; 32];
pub type DiceCommitment = [u8; 32];

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct DiceRoundState {
    pub white_commitment: Option<DiceCommitment>,
    pub black_commitment: Option<DiceCommitment>,
    pub white_reveal: Option<DiceSecret>,
    pub black_reveal: Option<DiceSecret>,
}

impl DiceRoundState {
    pub fn is_empty(&self) -> bool {
        self.white_commitment.is_none()
            && self.black_commitment.is_none()
            && self.white_reveal.is_none()
            && self.black_reveal.is_none()
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct DiceCommit {
    pub turn: u32,
    pub player: Player,
    pub commitment: DiceCommitment,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct DiceReveal {
    pub turn: u32,
    pub player: Player,
    pub secret: DiceSecret,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FairDiceError {
    WrongTurn { expected: u32, found: u32 },
    WrongPlayer { expected: Player, found: Player },
    CommitmentMismatch(Player),
    InsufficientRandomMaterial,
}

impl DiceCommit {
    pub fn new(game_id: &GameId, turn: u32, player: Player, secret: &DiceSecret) -> Self {
        Self {
            turn,
            player,
            commitment: dice_commitment(game_id, turn, player, secret),
        }
    }

    pub fn verify_reveal(
        &self,
        game_id: &GameId,
        reveal: &DiceReveal,
    ) -> Result<(), FairDiceError> {
        if reveal.turn != self.turn {
            return Err(FairDiceError::WrongTurn {
                expected: self.turn,
                found: reveal.turn,
            });
        }

        if reveal.player != self.player {
            return Err(FairDiceError::WrongPlayer {
                expected: self.player,
                found: reveal.player,
            });
        }

        let actual = dice_commitment(game_id, reveal.turn, reveal.player, &reveal.secret);

        if actual != self.commitment {
            return Err(FairDiceError::CommitmentMismatch(reveal.player));
        }

        Ok(())
    }
}

pub fn dice_commitment(
    game_id: &GameId,
    turn: u32,
    player: Player,
    secret: &DiceSecret,
) -> DiceCommitment {
    let mut hasher = blake3::Hasher::new();

    hasher.update(COMMITMENT_DOMAIN);
    hasher.update(game_id);
    hasher.update(&turn.to_be_bytes());
    hasher.update(&[player_tag(player)]);
    hasher.update(secret);

    *hasher.finalize().as_bytes()
}

pub fn verify_and_derive_dice(
    game_id: &GameId,
    turn: u32,
    white_commit: &DiceCommit,
    black_commit: &DiceCommit,
    white_reveal: &DiceReveal,
    black_reveal: &DiceReveal,
) -> Result<Dice, FairDiceError> {
    verify_commit_owner(white_commit, turn, Player::White)?;
    verify_commit_owner(black_commit, turn, Player::Black)?;

    white_commit.verify_reveal(game_id, white_reveal)?;
    black_commit.verify_reveal(game_id, black_reveal)?;

    derive_dice(game_id, turn, &white_reveal.secret, &black_reveal.secret)
}

pub fn derive_dice(
    game_id: &GameId,
    turn: u32,
    white_secret: &DiceSecret,
    black_secret: &DiceSecret,
) -> Result<Dice, FairDiceError> {
    let mut hasher = blake3::Hasher::new();

    hasher.update(ROLL_DOMAIN);
    hasher.update(game_id);
    hasher.update(&turn.to_be_bytes());

    /*
     * Contributions are always ordered by player, never by network
     * arrival order.
     */
    hasher.update(white_secret);
    hasher.update(black_secret);

    let mut reader = hasher.finalize_xof();
    let mut material = [0_u8; 64];

    reader.fill(&mut material);

    dice_from_random_bytes(&material)
}

fn verify_commit_owner(
    commitment: &DiceCommit,
    expected_turn: u32,
    expected_player: Player,
) -> Result<(), FairDiceError> {
    if commitment.turn != expected_turn {
        return Err(FairDiceError::WrongTurn {
            expected: expected_turn,
            found: commitment.turn,
        });
    }

    if commitment.player != expected_player {
        return Err(FairDiceError::WrongPlayer {
            expected: expected_player,
            found: commitment.player,
        });
    }

    Ok(())
}

/*
 * Values 0 through 251 are accepted because 252 is divisible by six.
 * Values 252 through 255 are discarded, avoiding modulo bias.
 */
fn dice_from_random_bytes(bytes: &[u8]) -> Result<Dice, FairDiceError> {
    let mut values = bytes
        .iter()
        .copied()
        .filter(|byte| *byte < 252)
        .map(|byte| (byte % 6) + 1);

    let first = values
        .next()
        .ok_or(FairDiceError::InsufficientRandomMaterial)?;

    let second = values
        .next()
        .ok_or(FairDiceError::InsufficientRandomMaterial)?;

    Ok(Dice { first, second })
}

const fn player_tag(player: Player) -> u8 {
    match player {
        Player::White => 0,
        Player::Black => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game_id(byte: u8) -> GameId {
        [byte; 32]
    }

    fn secret(byte: u8) -> DiceSecret {
        [byte; 32]
    }

    fn reveal(turn: u32, player: Player, byte: u8) -> DiceReveal {
        DiceReveal {
            turn,
            player,
            secret: secret(byte),
        }
    }

    #[test]
    fn commitment_is_deterministic_and_context_bound() {
        let game = game_id(7);
        let material = secret(11);

        let first = dice_commitment(&game, 3, Player::White, &material);

        assert_eq!(first, dice_commitment(&game, 3, Player::White, &material,));

        assert_ne!(
            first,
            dice_commitment(&game_id(8), 3, Player::White, &material,)
        );

        assert_ne!(first, dice_commitment(&game, 4, Player::White, &material,));

        assert_ne!(first, dice_commitment(&game, 3, Player::Black, &material,));
    }

    #[test]
    fn matching_reveal_is_accepted() {
        let game = game_id(7);
        let reveal = reveal(3, Player::White, 11);
        let commitment = DiceCommit::new(&game, reveal.turn, reveal.player, &reveal.secret);

        assert_eq!(commitment.verify_reveal(&game, &reveal), Ok(()));
    }

    #[test]
    fn wrong_secret_is_rejected() {
        let game = game_id(7);
        let commitment = DiceCommit::new(&game, 3, Player::White, &secret(11));

        assert_eq!(
            commitment.verify_reveal(&game, &reveal(3, Player::White, 12),),
            Err(FairDiceError::CommitmentMismatch(Player::White))
        );
    }

    #[test]
    fn wrong_turn_and_player_are_rejected() {
        let game = game_id(7);
        let commitment = DiceCommit::new(&game, 3, Player::White, &secret(11));

        assert_eq!(
            commitment.verify_reveal(&game, &reveal(4, Player::White, 11),),
            Err(FairDiceError::WrongTurn {
                expected: 3,
                found: 4,
            })
        );

        assert_eq!(
            commitment.verify_reveal(&game, &reveal(3, Player::Black, 11),),
            Err(FairDiceError::WrongPlayer {
                expected: Player::White,
                found: Player::Black,
            })
        );
    }

    #[test]
    fn verified_reveals_derive_reproducible_dice() {
        let game = game_id(7);
        let white = reveal(3, Player::White, 11);
        let black = reveal(3, Player::Black, 22);

        let white_commit = DiceCommit::new(&game, 3, Player::White, &white.secret);

        let black_commit = DiceCommit::new(&game, 3, Player::Black, &black.secret);

        let first =
            verify_and_derive_dice(&game, 3, &white_commit, &black_commit, &white, &black).unwrap();

        let second =
            verify_and_derive_dice(&game, 3, &white_commit, &black_commit, &white, &black).unwrap();

        assert_eq!(first, second);
        assert!((1..=6).contains(&first.first));
        assert!((1..=6).contains(&first.second));
    }

    #[test]
    fn contribution_order_is_fixed_by_player() {
        let game = game_id(7);

        let ordered = derive_dice(&game, 3, &secret(11), &secret(22)).unwrap();

        let swapped = derive_dice(&game, 3, &secret(22), &secret(11)).unwrap();

        assert_ne!(ordered, swapped);
    }

    #[test]
    fn derived_dice_remain_in_range_across_many_turns() {
        let game = game_id(7);

        for turn in 0..10_000 {
            let dice = derive_dice(&game, turn, &secret(11), &secret(22)).unwrap();

            assert!((1..=6).contains(&dice.first));
            assert!((1..=6).contains(&dice.second));
        }
    }

    #[test]
    fn rejection_sampling_skips_out_of_range_bytes() {
        let dice = dice_from_random_bytes(&[255, 254, 253, 252, 251, 0]).unwrap();

        assert_eq!(
            dice,
            Dice {
                first: 6,
                second: 1,
            }
        );
    }

    #[test]
    fn rejection_sampling_requires_two_accepted_bytes() {
        assert_eq!(
            dice_from_random_bytes(&[255, 252, 7]),
            Err(FairDiceError::InsufficientRandomMaterial)
        );
    }
}
