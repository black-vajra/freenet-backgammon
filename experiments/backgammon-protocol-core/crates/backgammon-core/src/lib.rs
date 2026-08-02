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

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveSource {
    Bar,
    Point(u8),
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct CheckerMove {
    pub player: Player,
    pub source: MoveSource,
    pub die: u8,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveError {
    GameAlreadyCompleted,
    WrongPlayer,
    NotMovingPhase,
    InvalidDieValue,
    PointOutOfRange,
    MustEnterFromBar,
    NoCheckerOnBar,
    SourcePointEmpty,
    SourceOwnedByOpponent,
    DestinationBlocked,
    BearingOffNotImplemented,
    CheckerCountOverflow,
    CheckerCountUnderflow,
    InvalidResultingState(StateError),
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

        let area = self.player_area(player);

        on_points + u16::from(area.bar) + u16::from(area.borne_off)
    }

    pub const fn player_area(&self, player: Player) -> PlayerArea {
        match player {
            Player::White => self.white,
            Player::Black => self.black,
        }
    }

    fn player_area_mut(&mut self, player: Player) -> &mut PlayerArea {
        match player {
            Player::White => &mut self.white,
            Player::Black => &mut self.black,
        }
    }

    pub fn entry_destination(player: Player, die: u8) -> Result<usize, MoveError> {
        if !(1..=6).contains(&die) {
            return Err(MoveError::InvalidDieValue);
        }

        Ok(match player {
            Player::White => usize::from(die - 1),
            Player::Black => POINT_COUNT - usize::from(die),
        })
    }

    pub fn point_destination(player: Player, source: usize, die: u8) -> Result<usize, MoveError> {
        if source >= POINT_COUNT {
            return Err(MoveError::PointOutOfRange);
        }

        if !(1..=6).contains(&die) {
            return Err(MoveError::InvalidDieValue);
        }

        match player {
            Player::White => source
                .checked_add(usize::from(die))
                .filter(|destination| *destination < POINT_COUNT)
                .ok_or(MoveError::BearingOffNotImplemented),
            Player::Black => source
                .checked_sub(usize::from(die))
                .ok_or(MoveError::BearingOffNotImplemented),
        }
    }

    pub fn destination_is_blocked(&self, player: Player, destination: usize) -> bool {
        let point = self.points[destination];

        point.owner == Some(player.opponent()) && point.count >= 2
    }

    pub fn apply_checker_move(&mut self, checker_move: CheckerMove) -> Result<(), MoveError> {
        let mut candidate = self.clone();
        candidate.apply_checker_move_unchecked(checker_move)?;
        candidate
            .verify()
            .map_err(MoveError::InvalidResultingState)?;
        *self = candidate;

        Ok(())
    }

    fn apply_checker_move_unchecked(&mut self, checker_move: CheckerMove) -> Result<(), MoveError> {
        if !matches!(self.status, GameStatus::InProgress) {
            return Err(MoveError::GameAlreadyCompleted);
        }

        if checker_move.player != self.active_player {
            return Err(MoveError::WrongPlayer);
        }

        if self.turn_phase != TurnPhase::Moving {
            return Err(MoveError::NotMovingPhase);
        }

        if !(1..=6).contains(&checker_move.die) {
            return Err(MoveError::InvalidDieValue);
        }

        let bar_count = self.player_area(checker_move.player).bar;

        let destination = match checker_move.source {
            MoveSource::Bar => {
                if bar_count == 0 {
                    return Err(MoveError::NoCheckerOnBar);
                }

                Self::entry_destination(checker_move.player, checker_move.die)?
            }
            MoveSource::Point(source) => {
                if bar_count > 0 {
                    return Err(MoveError::MustEnterFromBar);
                }

                let source = usize::from(source);

                if source >= POINT_COUNT {
                    return Err(MoveError::PointOutOfRange);
                }

                let point = self.points[source];

                match point.owner {
                    None => return Err(MoveError::SourcePointEmpty),
                    Some(owner) if owner != checker_move.player => {
                        return Err(MoveError::SourceOwnedByOpponent)
                    }
                    Some(_) => {}
                }

                Self::point_destination(checker_move.player, source, checker_move.die)?
            }
        };

        if self.destination_is_blocked(checker_move.player, destination) {
            return Err(MoveError::DestinationBlocked);
        }

        self.remove_checker(checker_move.player, checker_move.source)?;
        self.place_checker(checker_move.player, destination)?;

        Ok(())
    }

    fn remove_checker(&mut self, player: Player, source: MoveSource) -> Result<(), MoveError> {
        match source {
            MoveSource::Bar => {
                let area = self.player_area_mut(player);

                if area.bar == 0 {
                    return Err(MoveError::NoCheckerOnBar);
                }

                area.bar -= 1;
            }
            MoveSource::Point(source) => {
                let source = usize::from(source);

                if source >= POINT_COUNT {
                    return Err(MoveError::PointOutOfRange);
                }

                let point = &mut self.points[source];

                match point.owner {
                    None => return Err(MoveError::SourcePointEmpty),
                    Some(owner) if owner != player => return Err(MoveError::SourceOwnedByOpponent),
                    Some(_) => {}
                }

                point.count = point
                    .count
                    .checked_sub(1)
                    .ok_or(MoveError::CheckerCountUnderflow)?;

                if point.count == 0 {
                    *point = Point::EMPTY;
                }
            }
        }

        Ok(())
    }

    fn place_checker(&mut self, player: Player, destination: usize) -> Result<(), MoveError> {
        let destination_point = self.points[destination];

        if destination_point.owner == Some(player.opponent()) {
            debug_assert_eq!(destination_point.count, 1);

            let opponent_area = self.player_area_mut(player.opponent());
            opponent_area.bar = opponent_area
                .bar
                .checked_add(1)
                .ok_or(MoveError::CheckerCountOverflow)?;

            self.points[destination] = Point::occupied(player, 1);
            return Ok(());
        }

        let point = &mut self.points[destination];

        match point.owner {
            None => {
                *point = Point::occupied(player, 1);
            }
            Some(owner) if owner == player => {
                point.count = point
                    .count
                    .checked_add(1)
                    .ok_or(MoveError::CheckerCountOverflow)?;
            }
            Some(_) => return Err(MoveError::DestinationBlocked),
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn moving_state() -> GameState {
        let mut state = GameState::standard_start();
        state.turn_phase = TurnPhase::Moving;
        state.dice = Some(Dice {
            first: 3,
            second: 5,
        });
        state
    }

    fn sparse_state() -> GameState {
        let mut state = GameState {
            points: [Point::EMPTY; POINT_COUNT],
            white: PlayerArea {
                bar: 0,
                borne_off: 14,
            },
            black: PlayerArea {
                bar: 0,
                borne_off: 14,
            },
            active_player: Player::White,
            turn_phase: TurnPhase::Moving,
            dice: Some(Dice {
                first: 1,
                second: 2,
            }),
            status: GameStatus::InProgress,
        };

        state.points[0] = Point::occupied(Player::White, 1);
        state.points[5] = Point::occupied(Player::Black, 1);
        state
    }

    #[test]
    fn players_have_opponents() {
        assert_eq!(Player::White.opponent(), Player::Black);
        assert_eq!(Player::Black.opponent(), Player::White);
    }

    #[test]
    fn standard_start_is_valid() {
        let state = GameState::standard_start();

        assert_eq!(state.verify(), Ok(()));
        assert_eq!(
            state.checker_total(Player::White),
            u16::from(CHECKERS_PER_PLAYER)
        );
        assert_eq!(
            state.checker_total(Player::Black),
            u16::from(CHECKERS_PER_PLAYER)
        );
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

    #[test]
    fn entry_destinations_follow_player_orientation() {
        assert_eq!(GameState::entry_destination(Player::White, 1), Ok(0));
        assert_eq!(GameState::entry_destination(Player::White, 6), Ok(5));
        assert_eq!(GameState::entry_destination(Player::Black, 1), Ok(23));
        assert_eq!(GameState::entry_destination(Player::Black, 6), Ok(18));
    }

    #[test]
    fn ordinary_destinations_follow_player_orientation() {
        assert_eq!(GameState::point_destination(Player::White, 5, 3), Ok(8));
        assert_eq!(GameState::point_destination(Player::Black, 18, 3), Ok(15));
    }

    #[test]
    fn moving_beyond_board_is_not_yet_supported() {
        assert_eq!(
            GameState::point_destination(Player::White, 22, 3),
            Err(MoveError::BearingOffNotImplemented)
        );
        assert_eq!(
            GameState::point_destination(Player::Black, 1, 2),
            Err(MoveError::BearingOffNotImplemented)
        );
    }

    #[test]
    fn ordinary_move_relocates_checker() {
        let mut state = moving_state();

        assert_eq!(
            state.apply_checker_move(CheckerMove {
                player: Player::White,
                source: MoveSource::Point(0),
                die: 3,
            }),
            Ok(())
        );

        assert_eq!(state.points[0], Point::occupied(Player::White, 1));
        assert_eq!(state.points[3], Point::occupied(Player::White, 1));
        assert_eq!(state.verify(), Ok(()));
    }

    #[test]
    fn player_cannot_move_opponents_checker() {
        let mut state = moving_state();

        assert_eq!(
            state.apply_checker_move(CheckerMove {
                player: Player::White,
                source: MoveSource::Point(5),
                die: 1,
            }),
            Err(MoveError::SourceOwnedByOpponent)
        );
    }

    #[test]
    fn blocked_destination_is_rejected_without_mutation() {
        let mut state = moving_state();
        state.points[3] = Point::occupied(Player::Black, 2);
        state.points[23] = Point::EMPTY;
        let before = state.clone();

        assert_eq!(
            state.apply_checker_move(CheckerMove {
                player: Player::White,
                source: MoveSource::Point(0),
                die: 3,
            }),
            Err(MoveError::DestinationBlocked)
        );

        assert_eq!(state, before);
    }

    #[test]
    fn landing_on_blot_hits_opponent() {
        let mut state = sparse_state();

        assert_eq!(
            state.apply_checker_move(CheckerMove {
                player: Player::White,
                source: MoveSource::Point(0),
                die: 5,
            }),
            Ok(())
        );

        assert_eq!(state.points[0], Point::EMPTY);
        assert_eq!(state.points[5], Point::occupied(Player::White, 1));
        assert_eq!(state.black.bar, 1);
        assert_eq!(state.black.borne_off, 14);
        assert_eq!(state.verify(), Ok(()));
    }

    #[test]
    fn checker_on_bar_must_enter_before_point_move() {
        let mut state = moving_state();
        state.white.bar = 1;
        state.points[0].count -= 1;

        assert_eq!(state.verify(), Ok(()));

        assert_eq!(
            state.apply_checker_move(CheckerMove {
                player: Player::White,
                source: MoveSource::Point(0),
                die: 3,
            }),
            Err(MoveError::MustEnterFromBar)
        );
    }

    #[test]
    fn checker_can_enter_from_bar() {
        let mut state = moving_state();
        state.white.bar = 1;
        state.points[0].count -= 1;

        assert_eq!(
            state.apply_checker_move(CheckerMove {
                player: Player::White,
                source: MoveSource::Bar,
                die: 4,
            }),
            Ok(())
        );

        assert_eq!(state.white.bar, 0);
        assert_eq!(state.points[3], Point::occupied(Player::White, 1));
        assert_eq!(state.verify(), Ok(()));
    }

    #[test]
    fn bar_entry_can_hit_blot() {
        let mut state = moving_state();
        state.white.bar = 1;
        state.points[0].count -= 1;

        state.points[3] = Point::occupied(Player::Black, 1);
        state.points[23].count -= 1;

        assert_eq!(
            state.apply_checker_move(CheckerMove {
                player: Player::White,
                source: MoveSource::Bar,
                die: 4,
            }),
            Ok(())
        );

        assert_eq!(state.white.bar, 0);
        assert_eq!(state.black.bar, 1);
        assert_eq!(state.points[3], Point::occupied(Player::White, 1));
        assert_eq!(state.verify(), Ok(()));
    }

    #[test]
    fn bar_entry_to_blocked_point_is_rejected() {
        let mut state = moving_state();
        state.white.bar = 1;
        state.points[0].count -= 1;

        state.points[3] = Point::occupied(Player::Black, 2);
        state.points[23] = Point::EMPTY;
        let before = state.clone();

        assert_eq!(
            state.apply_checker_move(CheckerMove {
                player: Player::White,
                source: MoveSource::Bar,
                die: 4,
            }),
            Err(MoveError::DestinationBlocked)
        );

        assert_eq!(state, before);
    }

    #[test]
    fn malformed_owned_empty_source_is_rejected_without_underflow() {
        let mut state = moving_state();
        state.points[0] = Point {
            owner: Some(Player::White),
            count: 0,
        };
        let before = state.clone();

        assert_eq!(
            state.apply_checker_move(CheckerMove {
                player: Player::White,
                source: MoveSource::Point(0),
                die: 3,
            }),
            Err(MoveError::CheckerCountUnderflow)
        );

        assert_eq!(state, before);
    }

    #[test]
    fn inactive_player_cannot_move() {
        let mut state = moving_state();

        assert_eq!(
            state.apply_checker_move(CheckerMove {
                player: Player::Black,
                source: MoveSource::Point(23),
                die: 3,
            }),
            Err(MoveError::WrongPlayer)
        );
    }

    #[test]
    fn move_requires_moving_phase() {
        let mut state = GameState::standard_start();

        assert_eq!(
            state.apply_checker_move(CheckerMove {
                player: Player::White,
                source: MoveSource::Point(0),
                die: 3,
            }),
            Err(MoveError::NotMovingPhase)
        );
    }
}
