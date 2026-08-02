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
    pub fn values(self) -> Vec<u8> {
        if self.first == self.second {
            vec![self.first; 4]
        } else {
            vec![self.first, self.second]
        }
    }

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

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum MoveSource {
    Bar,
    Point(u8),
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct CheckerMove {
    pub player: Player,
    pub source: MoveSource,
    pub die: u8,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveTarget {
    Point(u8),
    BearOff,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct TurnSequence {
    pub moves: Vec<CheckerMove>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TurnError {
    GameAlreadyCompleted,
    NotMovingPhase,
    MissingDice,
    InvalidState(StateError),
    IllegalTurnSequence,
    Move(MoveError),
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
    NotAllCheckersInHome,
    OversizeBearOffBlocked,
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

    pub const fn point_is_in_home_board(player: Player, point: usize) -> bool {
        match player {
            Player::White => point >= 18 && point < POINT_COUNT,
            Player::Black => point < 6,
        }
    }

    pub fn all_checkers_in_home(&self, player: Player) -> bool {
        self.player_area(player).bar == 0
            && self.points.iter().enumerate().all(|(index, point)| {
                point.owner != Some(player) || Self::point_is_in_home_board(player, index)
            })
    }

    pub const fn bear_off_distance(player: Player, source: usize) -> usize {
        match player {
            Player::White => POINT_COUNT - source,
            Player::Black => source + 1,
        }
    }

    fn has_farther_checker(&self, player: Player, source: usize) -> bool {
        match player {
            Player::White => self.points[..source]
                .iter()
                .any(|point| point.owner == Some(player)),
            Player::Black => self.points[source + 1..]
                .iter()
                .any(|point| point.owner == Some(player)),
        }
    }

    pub fn move_target(
        &self,
        player: Player,
        source: usize,
        die: u8,
    ) -> Result<MoveTarget, MoveError> {
        match Self::point_destination(player, source, die) {
            Ok(destination) => Ok(MoveTarget::Point(
                u8::try_from(destination).map_err(|_| MoveError::PointOutOfRange)?,
            )),
            Err(MoveError::BearingOffNotImplemented) => {
                if !self.all_checkers_in_home(player) {
                    return Err(MoveError::NotAllCheckersInHome);
                }

                let distance = Self::bear_off_distance(player, source);
                let die = usize::from(die);

                if die > distance && self.has_farther_checker(player, source) {
                    return Err(MoveError::OversizeBearOffBlocked);
                }

                Ok(MoveTarget::BearOff)
            }
            Err(error) => Err(error),
        }
    }

    pub fn destination_is_blocked(&self, player: Player, destination: usize) -> bool {
        let point = self.points[destination];

        point.owner == Some(player.opponent()) && point.count >= 2
    }

    pub fn legal_checker_moves_for_die(&self, player: Player, die: u8) -> Vec<CheckerMove> {
        let sources: Vec<MoveSource> = if self.player_area(player).bar > 0 {
            vec![MoveSource::Bar]
        } else {
            self.points
                .iter()
                .enumerate()
                .filter(|(_, point)| point.owner == Some(player))
                .filter_map(|(index, _)| u8::try_from(index).ok().map(MoveSource::Point))
                .collect()
        };

        let mut legal = Vec::new();

        for source in sources {
            let checker_move = CheckerMove {
                player,
                source,
                die,
            };

            let mut candidate = self.clone();

            if candidate.apply_checker_move(checker_move).is_ok() {
                legal.push(checker_move);
            }
        }

        legal.sort_unstable();
        legal.dedup();
        legal
    }

    fn collect_turn_sequences(
        state: &GameState,
        remaining_dice: &[u8],
        current: &mut Vec<CheckerMove>,
        output: &mut Vec<TurnSequence>,
    ) {
        if !matches!(state.status, GameStatus::InProgress) {
            output.push(TurnSequence {
                moves: current.clone(),
            });
            return;
        }

        let mut advanced = false;
        let mut seen_die = [false; 7];

        for (index, die) in remaining_dice.iter().copied().enumerate() {
            let die_index = usize::from(die);

            if seen_die[die_index] {
                continue;
            }

            seen_die[die_index] = true;

            for checker_move in state.legal_checker_moves_for_die(state.active_player, die) {
                let mut next_state = state.clone();

                if next_state.apply_checker_move(checker_move).is_err() {
                    continue;
                }

                let mut next_dice = remaining_dice.to_vec();
                next_dice.remove(index);

                current.push(checker_move);
                Self::collect_turn_sequences(&next_state, &next_dice, current, output);
                current.pop();

                advanced = true;
            }
        }

        if !advanced {
            output.push(TurnSequence {
                moves: current.clone(),
            });
        }
    }

    pub fn legal_turn_sequences(&self) -> Result<Vec<TurnSequence>, TurnError> {
        self.verify().map_err(TurnError::InvalidState)?;

        if !matches!(self.status, GameStatus::InProgress) {
            return Err(TurnError::GameAlreadyCompleted);
        }

        if self.turn_phase != TurnPhase::Moving {
            return Err(TurnError::NotMovingPhase);
        }

        let dice = self.dice.ok_or(TurnError::MissingDice)?;
        let dice_values = dice.values();

        let mut generated = Vec::new();
        let mut current = Vec::new();

        Self::collect_turn_sequences(self, &dice_values, &mut current, &mut generated);

        let maximum_moves = generated
            .iter()
            .map(|sequence| sequence.moves.len())
            .max()
            .unwrap_or(0);

        generated.retain(|sequence| sequence.moves.len() == maximum_moves);

        /*
         * When only one of two distinct dice can be played, backgammon
         * requires use of the higher die if that die is playable.
         */
        if dice.first != dice.second && maximum_moves == 1 {
            let higher_die = dice.first.max(dice.second);

            if generated.iter().any(|sequence| {
                sequence.moves.first().map(|checker_move| checker_move.die) == Some(higher_die)
            }) {
                generated.retain(|sequence| {
                    sequence.moves.first().map(|checker_move| checker_move.die) == Some(higher_die)
                });
            }
        }

        generated.sort_unstable();
        generated.dedup();

        Ok(generated)
    }

    pub fn apply_turn_sequence(&mut self, sequence: &TurnSequence) -> Result<(), TurnError> {
        let legal = self.legal_turn_sequences()?;

        if legal.binary_search(sequence).is_err() {
            return Err(TurnError::IllegalTurnSequence);
        }

        let mut candidate = self.clone();

        for checker_move in &sequence.moves {
            candidate
                .apply_checker_move(*checker_move)
                .map_err(TurnError::Move)?;
        }

        candidate.dice = None;
        candidate.turn_phase = TurnPhase::AwaitingRoll;

        if matches!(candidate.status, GameStatus::InProgress) {
            candidate.active_player = candidate.active_player.opponent();
        }

        candidate.verify().map_err(TurnError::InvalidState)?;
        *self = candidate;

        Ok(())
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

        let target = match checker_move.source {
            MoveSource::Bar => {
                if bar_count == 0 {
                    return Err(MoveError::NoCheckerOnBar);
                }

                MoveTarget::Point(
                    u8::try_from(Self::entry_destination(
                        checker_move.player,
                        checker_move.die,
                    )?)
                    .map_err(|_| MoveError::PointOutOfRange)?,
                )
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

                self.move_target(checker_move.player, source, checker_move.die)?
            }
        };

        if let MoveTarget::Point(destination) = target {
            if self.destination_is_blocked(checker_move.player, usize::from(destination)) {
                return Err(MoveError::DestinationBlocked);
            }
        }

        self.remove_checker(checker_move.player, checker_move.source)?;

        match target {
            MoveTarget::Point(destination) => {
                self.place_checker(checker_move.player, usize::from(destination))?;
            }
            MoveTarget::BearOff => self.bear_off_checker(checker_move.player)?,
        }

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

    fn bear_off_checker(&mut self, player: Player) -> Result<(), MoveError> {
        let area = self.player_area_mut(player);
        area.borne_off = area
            .borne_off
            .checked_add(1)
            .ok_or(MoveError::CheckerCountOverflow)?;

        if area.borne_off == CHECKERS_PER_PLAYER {
            self.status = GameStatus::Completed {
                winner: player,
                points: self.completion_points(player),
            };
        }

        Ok(())
    }

    fn completion_points(&self, winner: Player) -> u8 {
        let loser = winner.opponent();
        let loser_area = self.player_area(loser);

        if loser_area.borne_off > 0 {
            return 1;
        }

        let loser_in_winners_home = self.points.iter().enumerate().any(|(index, point)| {
            point.owner == Some(loser) && Self::point_is_in_home_board(winner, index)
        });

        if loser_area.bar > 0 || loser_in_winners_home {
            3
        } else {
            2
        }
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

    fn white_bear_off_state(source: usize) -> GameState {
        let mut state = GameState {
            points: [Point::EMPTY; POINT_COUNT],
            white: PlayerArea {
                bar: 0,
                borne_off: 14,
            },
            black: PlayerArea {
                bar: 0,
                borne_off: 1,
            },
            active_player: Player::White,
            turn_phase: TurnPhase::Moving,
            dice: Some(Dice {
                first: 1,
                second: 6,
            }),
            status: GameStatus::InProgress,
        };

        state.points[source] = Point::occupied(Player::White, 1);
        state.points[6] = Point::occupied(Player::Black, 14);
        state
    }

    #[test]
    fn exact_die_bears_checker_off() {
        let mut state = white_bear_off_state(23);

        assert_eq!(
            state.apply_checker_move(CheckerMove {
                player: Player::White,
                source: MoveSource::Point(23),
                die: 1,
            }),
            Ok(())
        );

        assert_eq!(state.white.borne_off, 15);
        assert_eq!(
            state.status,
            GameStatus::Completed {
                winner: Player::White,
                points: 1,
            }
        );
        assert_eq!(state.verify(), Ok(()));
    }

    #[test]
    fn cannot_bear_off_until_all_checkers_are_home() {
        let mut state = moving_state();

        assert_eq!(
            state.apply_checker_move(CheckerMove {
                player: Player::White,
                source: MoveSource::Point(18),
                die: 6,
            }),
            Err(MoveError::NotAllCheckersInHome)
        );
    }

    #[test]
    fn oversized_die_can_bear_off_farthest_checker() {
        let mut state = white_bear_off_state(23);

        assert_eq!(
            state.apply_checker_move(CheckerMove {
                player: Player::White,
                source: MoveSource::Point(23),
                die: 6,
            }),
            Ok(())
        );
    }

    #[test]
    fn oversized_die_cannot_skip_farther_checker() {
        let mut state = white_bear_off_state(23);
        state.white.borne_off = 13;
        state.points[22] = Point::occupied(Player::White, 1);
        let before = state.clone();

        assert_eq!(
            state.apply_checker_move(CheckerMove {
                player: Player::White,
                source: MoveSource::Point(23),
                die: 6,
            }),
            Err(MoveError::OversizeBearOffBlocked)
        );

        assert_eq!(state, before);
    }

    #[test]
    fn black_bears_off_toward_point_zero() {
        let mut state = GameState {
            points: [Point::EMPTY; POINT_COUNT],
            white: PlayerArea {
                bar: 0,
                borne_off: 1,
            },
            black: PlayerArea {
                bar: 0,
                borne_off: 14,
            },
            active_player: Player::Black,
            turn_phase: TurnPhase::Moving,
            dice: Some(Dice {
                first: 1,
                second: 2,
            }),
            status: GameStatus::InProgress,
        };

        state.points[0] = Point::occupied(Player::Black, 1);
        state.points[17] = Point::occupied(Player::White, 14);

        assert_eq!(
            state.apply_checker_move(CheckerMove {
                player: Player::Black,
                source: MoveSource::Point(0),
                die: 1,
            }),
            Ok(())
        );

        assert_eq!(state.black.borne_off, 15);
    }

    #[test]
    fn completion_scores_gammon() {
        let mut state = white_bear_off_state(23);
        state.black.borne_off = 0;
        state.points[6] = Point::occupied(Player::Black, 15);

        state
            .apply_checker_move(CheckerMove {
                player: Player::White,
                source: MoveSource::Point(23),
                die: 1,
            })
            .unwrap();

        assert_eq!(
            state.status,
            GameStatus::Completed {
                winner: Player::White,
                points: 2,
            }
        );
    }

    #[test]
    fn completion_scores_backgammon_for_checker_on_bar() {
        let mut state = white_bear_off_state(23);
        state.black.borne_off = 0;
        state.black.bar = 1;
        state.points[6] = Point::occupied(Player::Black, 14);

        state
            .apply_checker_move(CheckerMove {
                player: Player::White,
                source: MoveSource::Point(23),
                die: 1,
            })
            .unwrap();

        assert_eq!(
            state.status,
            GameStatus::Completed {
                winner: Player::White,
                points: 3,
            }
        );
    }

    #[test]
    fn completion_scores_backgammon_in_winners_home() {
        let mut state = white_bear_off_state(23);
        state.black.borne_off = 0;
        state.points[6] = Point::occupied(Player::Black, 14);
        state.points[18] = Point::occupied(Player::Black, 1);

        state
            .apply_checker_move(CheckerMove {
                player: Player::White,
                source: MoveSource::Point(23),
                die: 1,
            })
            .unwrap();

        assert_eq!(
            state.status,
            GameStatus::Completed {
                winner: Player::White,
                points: 3,
            }
        );
    }

    fn turn_state(
        white_points: &[(usize, u8)],
        white_bar: u8,
        white_borne_off: u8,
        black_points: &[(usize, u8)],
        black_bar: u8,
        black_borne_off: u8,
        dice: Dice,
    ) -> GameState {
        let mut state = GameState {
            points: [Point::EMPTY; POINT_COUNT],
            white: PlayerArea {
                bar: white_bar,
                borne_off: white_borne_off,
            },
            black: PlayerArea {
                bar: black_bar,
                borne_off: black_borne_off,
            },
            active_player: Player::White,
            turn_phase: TurnPhase::Moving,
            dice: Some(dice),
            status: GameStatus::InProgress,
        };

        for (point, count) in white_points {
            state.points[*point] = Point::occupied(Player::White, *count);
        }

        for (point, count) in black_points {
            state.points[*point] = Point::occupied(Player::Black, *count);
        }

        assert_eq!(state.verify(), Ok(()));
        state
    }

    #[test]
    fn turn_generation_uses_both_dice_when_possible() {
        let state = turn_state(
            &[(0, 2)],
            0,
            13,
            &[(23, 1)],
            0,
            14,
            Dice {
                first: 1,
                second: 2,
            },
        );

        let sequences = state.legal_turn_sequences().unwrap();

        assert!(!sequences.is_empty());
        assert!(sequences.iter().all(|sequence| sequence.moves.len() == 2));
    }

    #[test]
    fn doubles_generate_four_moves_when_possible() {
        let state = turn_state(
            &[(0, 4)],
            0,
            11,
            &[(23, 1)],
            0,
            14,
            Dice {
                first: 1,
                second: 1,
            },
        );

        let sequences = state.legal_turn_sequences().unwrap();

        assert!(!sequences.is_empty());
        assert!(sequences.iter().all(|sequence| sequence.moves.len() == 4));
        assert!(sequences.iter().all(|sequence| {
            sequence
                .moves
                .iter()
                .all(|checker_move| checker_move.die == 1)
        }));
    }

    #[test]
    fn higher_die_is_required_when_only_one_die_can_be_used() {
        let state = turn_state(
            &[(23, 1)],
            0,
            14,
            &[(6, 1)],
            0,
            14,
            Dice {
                first: 1,
                second: 2,
            },
        );

        let sequences = state.legal_turn_sequences().unwrap();

        assert!(!sequences.is_empty());
        assert!(sequences.iter().all(|sequence| sequence.moves.len() == 1));
        assert!(sequences.iter().all(|sequence| sequence.moves[0].die == 2));
    }

    #[test]
    fn lower_die_is_allowed_when_higher_die_is_unplayable() {
        let state = turn_state(
            &[],
            1,
            14,
            &[(1, 2), (2, 2)],
            0,
            11,
            Dice {
                first: 1,
                second: 2,
            },
        );

        let sequences = state.legal_turn_sequences().unwrap();

        assert_eq!(
            sequences,
            vec![TurnSequence {
                moves: vec![CheckerMove {
                    player: Player::White,
                    source: MoveSource::Bar,
                    die: 1,
                }],
            }]
        );
    }

    #[test]
    fn bar_priority_applies_to_each_move_in_sequence() {
        let state = turn_state(
            &[],
            2,
            13,
            &[(23, 1)],
            0,
            14,
            Dice {
                first: 1,
                second: 2,
            },
        );

        let sequences = state.legal_turn_sequences().unwrap();

        assert!(!sequences.is_empty());
        assert!(sequences.iter().all(|sequence| sequence.moves.len() == 2));
        assert!(sequences.iter().all(|sequence| {
            sequence
                .moves
                .iter()
                .all(|checker_move| checker_move.source == MoveSource::Bar)
        }));
    }

    #[test]
    fn blocked_turn_has_one_empty_legal_sequence() {
        let state = turn_state(
            &[],
            1,
            14,
            &[(0, 2), (1, 2)],
            0,
            11,
            Dice {
                first: 1,
                second: 2,
            },
        );

        assert_eq!(
            state.legal_turn_sequences(),
            Ok(vec![TurnSequence::default()])
        );
    }

    #[test]
    fn applying_complete_turn_switches_player_and_clears_dice() {
        let mut state = turn_state(
            &[(0, 2)],
            0,
            13,
            &[(23, 1)],
            0,
            14,
            Dice {
                first: 1,
                second: 2,
            },
        );

        let sequence = state.legal_turn_sequences().unwrap()[0].clone();

        assert_eq!(state.apply_turn_sequence(&sequence), Ok(()));
        assert_eq!(state.active_player, Player::Black);
        assert_eq!(state.turn_phase, TurnPhase::AwaitingRoll);
        assert_eq!(state.dice, None);
        assert_eq!(state.verify(), Ok(()));
    }

    #[test]
    fn incomplete_turn_sequence_is_rejected_without_mutation() {
        let mut state = turn_state(
            &[(0, 2)],
            0,
            13,
            &[(23, 1)],
            0,
            14,
            Dice {
                first: 1,
                second: 2,
            },
        );
        let before = state.clone();

        let incomplete = TurnSequence {
            moves: vec![CheckerMove {
                player: Player::White,
                source: MoveSource::Point(0),
                die: 1,
            }],
        };

        assert_eq!(
            state.apply_turn_sequence(&incomplete),
            Err(TurnError::IllegalTurnSequence)
        );
        assert_eq!(state, before);
    }

    #[test]
    fn only_one_move_order_uses_both_dice() {
        let state = turn_state(
            &[(0, 1)],
            0,
            14,
            &[(2, 2), (23, 13)],
            0,
            0,
            Dice {
                first: 1,
                second: 2,
            },
        );

        let sequences = state.legal_turn_sequences().unwrap();

        assert_eq!(
            sequences,
            vec![TurnSequence {
                moves: vec![
                    CheckerMove {
                        player: Player::White,
                        source: MoveSource::Point(0),
                        die: 1,
                    },
                    CheckerMove {
                        player: Player::White,
                        source: MoveSource::Point(1),
                        die: 2,
                    },
                ],
            }]
        );
    }

    #[test]
    fn hitting_with_first_die_opens_second_move() {
        let state = turn_state(
            &[(0, 1)],
            0,
            14,
            &[(1, 1), (2, 2), (23, 12)],
            0,
            0,
            Dice {
                first: 1,
                second: 2,
            },
        );

        let sequences = state.legal_turn_sequences().unwrap();

        assert_eq!(
            sequences,
            vec![TurnSequence {
                moves: vec![
                    CheckerMove {
                        player: Player::White,
                        source: MoveSource::Point(0),
                        die: 1,
                    },
                    CheckerMove {
                        player: Player::White,
                        source: MoveSource::Point(1),
                        die: 2,
                    },
                ],
            }]
        );
    }

    #[test]
    fn bar_entry_with_one_die_enables_other_die() {
        let state = turn_state(
            &[],
            1,
            14,
            &[(1, 2), (23, 13)],
            0,
            0,
            Dice {
                first: 1,
                second: 2,
            },
        );

        let sequences = state.legal_turn_sequences().unwrap();

        assert_eq!(
            sequences,
            vec![TurnSequence {
                moves: vec![
                    CheckerMove {
                        player: Player::White,
                        source: MoveSource::Bar,
                        die: 1,
                    },
                    CheckerMove {
                        player: Player::White,
                        source: MoveSource::Point(0),
                        die: 2,
                    },
                ],
            }]
        );
    }

    #[test]
    fn doubles_use_three_moves_when_fourth_is_blocked() {
        let state = turn_state(
            &[(0, 1)],
            0,
            14,
            &[(4, 2), (23, 13)],
            0,
            0,
            Dice {
                first: 1,
                second: 1,
            },
        );

        let sequences = state.legal_turn_sequences().unwrap();

        assert_eq!(
            sequences,
            vec![TurnSequence {
                moves: vec![
                    CheckerMove {
                        player: Player::White,
                        source: MoveSource::Point(0),
                        die: 1,
                    },
                    CheckerMove {
                        player: Player::White,
                        source: MoveSource::Point(1),
                        die: 1,
                    },
                    CheckerMove {
                        player: Player::White,
                        source: MoveSource::Point(2),
                        die: 1,
                    },
                ],
            }]
        );
    }

    #[test]
    fn game_completion_stops_remaining_dice_and_uses_higher_die() {
        let state = turn_state(
            &[(23, 1)],
            0,
            14,
            &[(6, 15)],
            0,
            0,
            Dice {
                first: 1,
                second: 2,
            },
        );

        assert_eq!(
            state.legal_turn_sequences(),
            Ok(vec![TurnSequence {
                moves: vec![CheckerMove {
                    player: Player::White,
                    source: MoveSource::Point(23),
                    die: 2,
                }],
            }])
        );
    }

    #[test]
    fn generated_turn_sequences_are_sorted_and_unique() {
        let mut state = GameState::standard_start();
        state.turn_phase = TurnPhase::Moving;
        state.dice = Some(Dice {
            first: 1,
            second: 2,
        });

        let sequences = state.legal_turn_sequences().unwrap();

        assert!(!sequences.is_empty());

        for pair in sequences.windows(2) {
            assert!(pair[0] < pair[1]);
        }
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
