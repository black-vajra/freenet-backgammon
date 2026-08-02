#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};

pub const POINT_COUNT: usize = 24;
pub const CHECKERS_PER_PLAYER: u8 = 15;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Player {
    White,
    Black,
}

impl Player {
    pub const fn opponent(self) -> Self {
        match self {
            Self::White => Self::Black,
            Self::Black => Self::White,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Point {
    pub owner: Option<Player>,
    pub count: u8,
}

impl Point {
    pub const EMPTY: Self = Self {
        owner: None,
        count: 0,
    };

    pub const fn occupied(owner: Player, count: u8) -> Self {
        Self {
            owner: Some(owner),
            count,
        }
    }

    pub fn verify(self) -> Result<(), StateError> {
        match (self.owner, self.count) {
            (None, 0) => Ok(()),
            (Some(_), 1..) => Ok(()),
            (None, 1..) => Err(StateError::UnownedCheckers),
            (Some(_), 0) => Err(StateError::OwnedEmptyPoint),
        }
    }
}

impl Default for Point {
    fn default() -> Self {
        Self::EMPTY
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlayerArea {
    pub bar: u8,
    pub borne_off: u8,
}

impl PlayerArea {
    pub const EMPTY: Self = Self {
        bar: 0,
        borne_off: 0,
    };
}

impl Default for PlayerArea {
    fn default() -> Self {
        Self::EMPTY
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum TurnPhase {
    AwaitingRoll,
    Moving,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dice {
    pub first: u8,
    pub second: u8,
}

impl Dice {
    pub fn verify(self) -> Result<(), StateError> {
        if !(1..=6).contains(&self.first) || !(1..=6).contains(&self.second) {
            return Err(StateError::InvalidDieValue);
        }

        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum GameStatus {
    InProgress,
    Completed { winner: Player, points: u8 },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct GameState {
    pub points: [Point; POINT_COUNT],
    pub white: PlayerArea,
    pub black: PlayerArea,
    pub active_player: Player,
    pub turn_phase: TurnPhase,
    pub dice: Option<Dice>,
    pub status: GameStatus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StateError {
    UnownedCheckers,
    OwnedEmptyPoint,
    InvalidDieValue,
    DicePhaseMismatch,
    IncorrectCheckerTotal(Player),
}

impl GameState {
    pub fn standard_start() -> Self {
        let mut points = [Point::EMPTY; POINT_COUNT];

        /*
         * Canonical point indexing:
         *
         * - indices 0..=23 represent points 1..=24
         * - White moves toward index 23
         * - Black moves toward index 0
         *
         * Standard setup:
         * White: 2 on 1, 5 on 12, 3 on 17, 5 on 19
         * Black: 2 on 24, 5 on 13, 3 on 8, 5 on 6
         */
        points[0] = Point::occupied(Player::White, 2);
        points[11] = Point::occupied(Player::White, 5);
        points[16] = Point::occupied(Player::White, 3);
        points[18] = Point::occupied(Player::White, 5);

        points[23] = Point::occupied(Player::Black, 2);
        points[12] = Point::occupied(Player::Black, 5);
        points[7] = Point::occupied(Player::Black, 3);
        points[5] = Point::occupied(Player::Black, 5);

        Self {
            points,
            white: PlayerArea::EMPTY,
            black: PlayerArea::EMPTY,
            active_player: Player::White,
            turn_phase: TurnPhase::AwaitingRoll,
            dice: None,
            status: GameStatus::InProgress,
        }
    }

    pub fn verify(&self) -> Result<(), StateError> {
        for point in self.points {
            point.verify()?;
        }

        match (self.turn_phase, self.dice) {
            (TurnPhase::AwaitingRoll, None) | (TurnPhase::Moving, Some(_)) => {}
            _ => return Err(StateError::DicePhaseMismatch),
        }

        if let Some(dice) = self.dice {
            dice.verify()?;
        }

        for player in [Player::White, Player::Black] {
            if self.checker_total(player) != u16::from(CHECKERS_PER_PLAYER) {
                return Err(StateError::IncorrectCheckerTotal(player));
            }
        }

        Ok(())
    }

    pub fn checker_total(&self, player: Player) -> u16 {
        let on_points: u16 = self
            .points
            .iter()
            .filter(|point| point.owner == Some(player))
            .map(|point| u16::from(point.count))
            .sum();

        let area = match player {
            Player::White => self.white,
            Player::Black => self.black,
        };

        on_points + u16::from(area.bar) + u16::from(area.borne_off)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn players_have_opponents() {
        assert_eq!(Player::White.opponent(), Player::Black);
        assert_eq!(Player::Black.opponent(), Player::White);
    }

    #[test]
    fn standard_start_is_valid() {
        let state = GameState::standard_start();

        assert_eq!(state.verify(), Ok(()));
        assert_eq!(state.checker_total(Player::White), CHECKERS_PER_PLAYER);
        assert_eq!(state.checker_total(Player::Black), CHECKERS_PER_PLAYER);
    }

    #[test]
    fn standard_start_uses_expected_points() {
        let state = GameState::standard_start();

        assert_eq!(state.points[0], Point::occupied(Player::White, 2));
        assert_eq!(state.points[11], Point::occupied(Player::White, 5));
        assert_eq!(state.points[16], Point::occupied(Player::White, 3));
        assert_eq!(state.points[18], Point::occupied(Player::White, 5));

        assert_eq!(state.points[23], Point::occupied(Player::Black, 2));
        assert_eq!(state.points[12], Point::occupied(Player::Black, 5));
        assert_eq!(state.points[7], Point::occupied(Player::Black, 3));
        assert_eq!(state.points[5], Point::occupied(Player::Black, 5));
    }

    #[test]
    fn unowned_checker_stack_is_invalid() {
        let point = Point {
            owner: None,
            count: 2,
        };

        assert_eq!(point.verify(), Err(StateError::UnownedCheckers));
    }

    #[test]
    fn owned_empty_point_is_invalid() {
        let point = Point {
            owner: Some(Player::White),
            count: 0,
        };

        assert_eq!(point.verify(), Err(StateError::OwnedEmptyPoint));
    }

    #[test]
    fn invalid_die_value_is_rejected() {
        let dice = Dice {
            first: 0,
            second: 6,
        };

        assert_eq!(dice.verify(), Err(StateError::InvalidDieValue));
    }

    #[test]
    fn dice_and_turn_phase_must_agree() {
        let mut state = GameState::standard_start();
        state.turn_phase = TurnPhase::Moving;

        assert_eq!(state.verify(), Err(StateError::DicePhaseMismatch));

        state.dice = Some(Dice {
            first: 3,
            second: 5,
        });

        assert_eq!(state.verify(), Ok(()));
    }

    #[test]
    fn missing_checker_is_rejected() {
        let mut state = GameState::standard_start();
        state.points[0].count -= 1;

        assert_eq!(
            state.verify(),
            Err(StateError::IncorrectCheckerTotal(Player::White))
        );
    }
}
