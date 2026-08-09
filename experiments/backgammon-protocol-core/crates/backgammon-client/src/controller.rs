use backgammon_core::{
    CheckerMove, Dice, GameState, GameStatus, MoveError, MoveSource, MoveTarget, Player,
    StateError, TurnError, TurnPhase, TurnSequence,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalTurnRecord {
    pub player: Player,
    pub dice: Dice,
    pub moves: Vec<CheckerMove>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalGameOutcome {
    Completed { winner: Player, points: u8 },
    Resigned { resigned: Player, winner: Player },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ControllerError {
    GameAlreadyCompleted,
    SessionInactive,
    NotAwaitingRoll,
    NoForcedPassPending,
    InvalidDice(StateError),
    InvalidState(StateError),
    TurnGeneration(TurnError),
    IllegalSource,
    NoSourceSelected,
    IllegalDestination,
    Move(MoveError),
    Commit(TurnError),
}

#[derive(Clone, Debug)]
pub struct LocalGameController {
    state: GameState,
    turn_start_state: Option<GameState>,
    preview_state: GameState,
    legal_sequences: Vec<TurnSequence>,
    selected_moves: Vec<CheckerMove>,
    selected_source: Option<MoveSource>,
    history: Vec<LocalTurnRecord>,
    outcome: Option<LocalGameOutcome>,
    left_table: bool,
    status_message: String,
}

impl Default for LocalGameController {
    fn default() -> Self {
        Self::new()
    }
}

impl LocalGameController {
    pub fn new() -> Self {
        let state = GameState::standard_start();

        Self {
            preview_state: state.clone(),
            state,
            turn_start_state: None,
            legal_sequences: Vec::new(),
            selected_moves: Vec::new(),
            selected_source: None,
            history: Vec::new(),
            outcome: None,
            left_table: false,
            status_message: "White to roll.".to_owned(),
        }
    }

    /// Replaces local committed and preview state with an independently
    /// verified authoritative game state.
    ///
    /// Any uncommitted checker selection is discarded. The caller must only
    /// supply state produced by verified protocol replay.
    pub fn sync_authoritative_state(&mut self, state: GameState) -> Result<(), ControllerError> {
        state.verify().map_err(ControllerError::InvalidState)?;

        let legal_sequences = if state.turn_phase == TurnPhase::Moving {
            state
                .legal_turn_sequences()
                .map_err(ControllerError::TurnGeneration)?
        } else {
            Vec::new()
        };

        self.state = state.clone();
        self.preview_state = state.clone();
        self.turn_start_state = if state.turn_phase == TurnPhase::Moving {
            Some(state)
        } else {
            None
        };
        self.legal_sequences = legal_sequences;
        self.selected_moves.clear();
        self.selected_source = None;
        self.left_table = false;

        self.sync_outcome_from_state();

        if self.outcome.is_none() {
            self.status_message = match self.state.turn_phase {
                TurnPhase::AwaitingRoll => {
                    format!("{} to roll.", player_name(self.state.active_player))
                }
                TurnPhase::Moving => {
                    let dice = self
                        .state
                        .dice
                        .ok_or(ControllerError::TurnGeneration(TurnError::MissingDice))?;

                    if self.must_pass() {
                        format!(
                            "{} rolled {} and {} but has no legal move. Select Pass turn.",
                            player_name(self.state.active_player),
                            dice.first,
                            dice.second,
                        )
                    } else {
                        format!(
                            "{} rolled {} and {}.",
                            player_name(self.state.active_player),
                            dice.first,
                            dice.second,
                        )
                    }
                }
            };
        }

        Ok(())
    }

    /// Replaces local committed state and move history with independently
    /// verified authoritative projections.
    ///
    /// The caller must supply both values from the same verified protocol replay.
    pub fn sync_authoritative_state_and_history(
        &mut self,
        state: GameState,
        history: Vec<LocalTurnRecord>,
    ) -> Result<(), ControllerError> {
        self.sync_authoritative_state(state)?;
        self.history = history;
        Ok(())
    }

    pub fn state(&self) -> &GameState {
        &self.state
    }

    pub fn visible_state(&self) -> &GameState {
        &self.preview_state
    }

    pub fn history(&self) -> &[LocalTurnRecord] {
        &self.history
    }

    pub fn selected_moves(&self) -> &[CheckerMove] {
        &self.selected_moves
    }

    pub fn selected_source(&self) -> Option<MoveSource> {
        self.selected_source
    }

    pub fn outcome(&self) -> Option<LocalGameOutcome> {
        self.outcome
    }

    pub fn has_left_table(&self) -> bool {
        self.left_table
    }

    pub fn is_active(&self) -> bool {
        !self.left_table && self.outcome.is_none()
    }

    pub fn status_message(&self) -> &str {
        &self.status_message
    }

    pub fn must_pass(&self) -> bool {
        self.is_active()
            && self.state.turn_phase == TurnPhase::Moving
            && self.legal_sequences.len() == 1
            && self.legal_sequences[0].moves.is_empty()
    }

    pub fn new_game(&mut self) {
        *self = Self::new();
    }

    pub fn resign(&mut self) -> Result<(), ControllerError> {
        if self.left_table {
            return Err(ControllerError::SessionInactive);
        }

        if self.outcome.is_some() || !matches!(self.state.status, GameStatus::InProgress) {
            return Err(ControllerError::GameAlreadyCompleted);
        }

        let resigned = self.state.active_player;
        let winner = resigned.opponent();

        self.outcome = Some(LocalGameOutcome::Resigned { resigned, winner });
        self.cancel_pending_turn();
        self.status_message = format!(
            "{} resigned. {} wins.",
            player_name(resigned),
            player_name(winner)
        );

        Ok(())
    }

    pub fn leave_table(&mut self) {
        self.left_table = true;
        self.cancel_pending_turn();
        self.status_message = "You left the local table.".to_owned();
    }

    pub fn begin_turn(&mut self, dice: Dice) -> Result<(), ControllerError> {
        if !self.is_active() {
            return Err(ControllerError::SessionInactive);
        }

        if !matches!(self.state.status, GameStatus::InProgress) {
            return Err(ControllerError::GameAlreadyCompleted);
        }

        if self.state.turn_phase != TurnPhase::AwaitingRoll {
            return Err(ControllerError::NotAwaitingRoll);
        }

        dice.verify().map_err(ControllerError::InvalidDice)?;

        let player = self.state.active_player;
        let mut turn_state = self.state.clone();
        turn_state.turn_phase = TurnPhase::Moving;
        turn_state.dice = Some(dice);
        turn_state.verify().map_err(ControllerError::InvalidState)?;

        let legal_sequences = turn_state
            .legal_turn_sequences()
            .map_err(ControllerError::TurnGeneration)?;

        self.state = turn_state.clone();
        self.preview_state = turn_state.clone();
        self.turn_start_state = Some(turn_state);
        self.legal_sequences = legal_sequences;
        self.selected_moves.clear();
        self.selected_source = None;
        self.status_message = format!(
            "{} rolled {} and {}.",
            player_name(player),
            dice.first,
            dice.second
        );

        if self.must_pass() {
            self.status_message = format!(
                "{} rolled {} and {} but has no legal move. Select Pass turn.",
                player_name(player),
                dice.first,
                dice.second
            );
        }

        Ok(())
    }

    pub fn pass_turn(&mut self) -> Result<(), ControllerError> {
        if !self.is_active() {
            return Err(ControllerError::SessionInactive);
        }

        if !self.must_pass() {
            return Err(ControllerError::NoForcedPassPending);
        }

        self.commit_sequence(TurnSequence::default())
    }

    /// Prepares a forced pass for network submission without advancing the
    /// locally committed state.
    ///
    /// The authoritative replay must remain the committed source of truth
    /// until Freenet accepts and returns the corresponding PlayTurn action.
    pub fn prepare_pass_for_submission(&mut self) -> Result<TurnSequence, ControllerError> {
        if !self.is_active() {
            return Err(ControllerError::SessionInactive);
        }

        if !self.must_pass() {
            return Err(ControllerError::NoForcedPassPending);
        }

        self.status_message = "Forced pass ready for network submission.".to_owned();

        Ok(TurnSequence::default())
    }

    pub fn legal_sources(&self) -> Vec<MoveSource> {
        if !self.is_active() {
            return Vec::new();
        }

        let mut sources = Vec::new();

        for checker_move in self.next_legal_moves() {
            if !sources.contains(&checker_move.source) {
                sources.push(checker_move.source);
            }
        }

        sources.sort();
        sources
    }

    pub fn select_source(&mut self, source: MoveSource) -> Result<(), ControllerError> {
        if !self.is_active() {
            return Err(ControllerError::SessionInactive);
        }

        if !self.legal_sources().contains(&source) {
            self.selected_source = None;
            self.status_message = "That checker cannot be moved legally.".to_owned();
            return Err(ControllerError::IllegalSource);
        }

        self.selected_source = Some(source);
        self.status_message = format!("Selected {}.", source_name(source));

        Ok(())
    }

    pub fn legal_destinations(&self) -> Result<Vec<MoveTarget>, ControllerError> {
        if !self.is_active() {
            return Err(ControllerError::SessionInactive);
        }

        let source = self
            .selected_source
            .ok_or(ControllerError::NoSourceSelected)?;

        self.legal_destinations_for(source)
    }

    pub fn legal_destinations_for(
        &self,
        source: MoveSource,
    ) -> Result<Vec<MoveTarget>, ControllerError> {
        if !self.is_active() {
            return Err(ControllerError::SessionInactive);
        }

        let mut destinations = Vec::new();

        for checker_move in self
            .next_legal_moves()
            .into_iter()
            .filter(|checker_move| checker_move.source == source)
        {
            let target = move_target_for(&self.preview_state, checker_move)
                .map_err(ControllerError::Move)?;

            if !destinations.contains(&target) {
                destinations.push(target);
            }
        }

        if destinations.is_empty() {
            return Err(ControllerError::IllegalSource);
        }

        Ok(destinations)
    }

    pub fn choose_destination(&mut self, destination: MoveTarget) -> Result<bool, ControllerError> {
        let turn_is_complete = self.select_destination_for_turn(destination)?;

        if turn_is_complete {
            let sequence = TurnSequence {
                moves: self.selected_moves.clone(),
            };

            self.commit_sequence(sequence)?;
        }

        Ok(turn_is_complete)
    }

    /// Applies one legal checker selection to the transient preview.
    ///
    /// When the complete legal turn has been selected, this returns the
    /// canonical TurnSequence but deliberately leaves the committed state,
    /// dice, active player, and history unchanged. The sequence can then be
    /// submitted as a durable PlayTurn action.
    pub fn choose_destination_for_submission(
        &mut self,
        destination: MoveTarget,
    ) -> Result<Option<TurnSequence>, ControllerError> {
        let turn_is_complete = self.select_destination_for_turn(destination)?;

        if !turn_is_complete {
            return Ok(None);
        }

        let sequence = TurnSequence {
            moves: self.selected_moves.clone(),
        };

        self.status_message = "Turn ready for network submission.".to_owned();

        Ok(Some(sequence))
    }

    fn select_destination_for_turn(
        &mut self,
        destination: MoveTarget,
    ) -> Result<bool, ControllerError> {
        if !self.is_active() {
            return Err(ControllerError::SessionInactive);
        }

        let source = self
            .selected_source
            .ok_or(ControllerError::NoSourceSelected)?;

        let checker_move = self
            .next_legal_moves()
            .into_iter()
            .filter(|checker_move| checker_move.source == source)
            .find(|checker_move| {
                move_target_for(&self.preview_state, *checker_move)
                    .is_ok_and(|target| target == destination)
            })
            .ok_or(ControllerError::IllegalDestination)?;

        self.selected_moves.push(checker_move);
        self.selected_source = None;
        self.rebuild_preview()?;

        let matching_sequences = self.matching_sequences();

        if matching_sequences.is_empty() {
            return Err(ControllerError::IllegalDestination);
        }

        let turn_is_complete = matching_sequences
            .iter()
            .all(|sequence| sequence.moves.len() == self.selected_moves.len());

        if !turn_is_complete {
            self.status_message = format!(
                "{} move selected. Choose the next checker.",
                self.selected_moves.len()
            );
        }

        Ok(turn_is_complete)
    }

    fn next_legal_moves(&self) -> Vec<CheckerMove> {
        let prefix_length = self.selected_moves.len();
        let mut moves = Vec::new();

        for sequence in self.matching_sequences() {
            if let Some(checker_move) = sequence.moves.get(prefix_length) {
                if !moves.contains(checker_move) {
                    moves.push(*checker_move);
                }
            }
        }

        moves.sort();
        moves
    }

    fn matching_sequences(&self) -> Vec<&TurnSequence> {
        self.legal_sequences
            .iter()
            .filter(|sequence| sequence.moves.starts_with(&self.selected_moves))
            .collect()
    }

    fn rebuild_preview(&mut self) -> Result<(), ControllerError> {
        let mut preview = self
            .turn_start_state
            .clone()
            .ok_or(ControllerError::NotAwaitingRoll)?;

        for checker_move in &self.selected_moves {
            preview
                .apply_checker_move(*checker_move)
                .map_err(ControllerError::Move)?;
        }

        self.preview_state = preview;
        Ok(())
    }

    fn commit_sequence(&mut self, sequence: TurnSequence) -> Result<(), ControllerError> {
        let turn_start = self
            .turn_start_state
            .clone()
            .ok_or(ControllerError::NotAwaitingRoll)?;

        let player = turn_start.active_player;
        let dice = turn_start
            .dice
            .ok_or(ControllerError::TurnGeneration(TurnError::MissingDice))?;

        let mut committed = turn_start;
        committed
            .apply_turn_sequence(&sequence)
            .map_err(ControllerError::Commit)?;

        self.history.push(LocalTurnRecord {
            player,
            dice,
            moves: sequence.moves,
        });

        self.state = committed.clone();
        self.preview_state = committed;
        self.turn_start_state = None;
        self.legal_sequences.clear();
        self.selected_moves.clear();
        self.selected_source = None;

        self.sync_outcome_from_state();

        if self.outcome.is_none() {
            self.status_message = format!("{} to roll.", player_name(self.state.active_player));
        }

        Ok(())
    }

    fn sync_outcome_from_state(&mut self) {
        if let GameStatus::Completed { winner, points } = self.state.status {
            self.outcome = Some(LocalGameOutcome::Completed { winner, points });
            self.status_message = format!(
                "{} wins a {} for {} point{}.",
                player_name(winner),
                result_name(points),
                points,
                if points == 1 { "" } else { "s" }
            );
        }
    }

    fn cancel_pending_turn(&mut self) {
        self.preview_state = self.state.clone();
        self.turn_start_state = None;
        self.legal_sequences.clear();
        self.selected_moves.clear();
        self.selected_source = None;
    }
}

fn move_target_for(state: &GameState, checker_move: CheckerMove) -> Result<MoveTarget, MoveError> {
    match checker_move.source {
        MoveSource::Bar => {
            let destination = GameState::entry_destination(checker_move.player, checker_move.die)?;

            Ok(MoveTarget::Point(
                u8::try_from(destination).map_err(|_| MoveError::PointOutOfRange)?,
            ))
        }
        MoveSource::Point(source) => {
            state.move_target(checker_move.player, usize::from(source), checker_move.die)
        }
    }
}

fn player_name(player: Player) -> &'static str {
    match player {
        Player::White => "White",
        Player::Black => "Black",
    }
}

fn result_name(points: u8) -> &'static str {
    match points {
        1 => "single game",
        2 => "gammon",
        3 => "backgammon",
        _ => "game",
    }
}

fn source_name(source: MoveSource) -> String {
    match source {
        MoveSource::Bar => "bar checker".to_owned(),
        MoveSource::Point(index) => format!("point {}", index + 1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn play_first_available_turn(controller: &mut LocalGameController) {
        while controller.state().turn_phase == TurnPhase::Moving && controller.outcome().is_none() {
            let source = controller.legal_sources()[0];
            controller.select_source(source).unwrap();

            let destination = controller.legal_destinations().unwrap()[0];
            controller.choose_destination(destination).unwrap();
        }
    }

    fn prepare_first_available_turn(controller: &mut LocalGameController) -> TurnSequence {
        loop {
            let source = controller.legal_sources()[0];
            controller.select_source(source).unwrap();

            let destination = controller.legal_destinations().unwrap()[0];

            if let Some(sequence) = controller
                .choose_destination_for_submission(destination)
                .unwrap()
            {
                return sequence;
            }
        }
    }

    #[test]
    fn authoritative_state_and_history_replaces_local_history() {
        let mut controller = LocalGameController::new();

        controller.history = vec![LocalTurnRecord {
            player: Player::White,
            dice: Dice {
                first: 1,
                second: 2,
            },
            moves: Vec::new(),
        }];

        let authoritative_history = vec![LocalTurnRecord {
            player: Player::Black,
            dice: Dice {
                first: 6,
                second: 6,
            },
            moves: Vec::new(),
        }];

        controller
            .sync_authoritative_state_and_history(
                GameState::standard_start(),
                authoritative_history.clone(),
            )
            .unwrap();

        assert_eq!(controller.history(), authoritative_history.as_slice());
    }

    #[test]
    fn new_controller_starts_with_standard_game() {
        let controller = LocalGameController::new();

        assert_eq!(controller.state(), &GameState::standard_start());
        assert_eq!(controller.visible_state(), controller.state());
        assert!(controller.history().is_empty());
        assert!(controller.legal_sources().is_empty());
        assert_eq!(controller.outcome(), None);
        assert!(!controller.has_left_table());
        assert!(controller.is_active());
        assert_eq!(controller.status_message(), "White to roll.");
    }

    #[test]
    fn authoritative_awaiting_roll_state_replaces_local_progress() {
        let mut controller = LocalGameController::new();

        controller
            .begin_turn(Dice {
                first: 1,
                second: 2,
            })
            .unwrap();

        let mut authoritative = GameState::standard_start();
        authoritative.active_player = Player::Black;
        authoritative.verify().unwrap();

        controller
            .sync_authoritative_state(authoritative.clone())
            .unwrap();

        assert_eq!(controller.state(), &authoritative);
        assert_eq!(controller.visible_state(), &authoritative);
        assert_eq!(controller.selected_source(), None);
        assert!(controller.selected_moves().is_empty());
        assert!(controller.legal_sources().is_empty());
        assert_eq!(controller.status_message(), "Black to roll.");
    }

    #[test]
    fn authoritative_moving_state_restores_derived_dice_and_legal_moves() {
        let mut authoritative = GameState::standard_start();
        authoritative.turn_phase = TurnPhase::Moving;
        authoritative.dice = Some(Dice {
            first: 3,
            second: 1,
        });
        authoritative.verify().unwrap();

        let expected_sequences = authoritative.legal_turn_sequences().unwrap();

        let mut controller = LocalGameController::new();

        controller
            .sync_authoritative_state(authoritative.clone())
            .unwrap();

        assert_eq!(controller.state(), &authoritative);
        assert_eq!(controller.visible_state(), &authoritative);
        assert_eq!(
            controller.state().dice,
            Some(Dice {
                first: 3,
                second: 1,
            })
        );
        assert!(!expected_sequences.is_empty());
        assert!(!controller.legal_sources().is_empty());
        assert_eq!(controller.status_message(), "White rolled 3 and 1.");
    }

    #[test]
    fn invalid_authoritative_state_is_rejected_without_mutation() {
        let mut controller = LocalGameController::new();
        let original = controller.state().clone();

        let mut invalid = GameState::standard_start();
        invalid.turn_phase = TurnPhase::Moving;
        invalid.dice = None;

        assert!(matches!(
            controller.sync_authoritative_state(invalid),
            Err(ControllerError::InvalidState(_))
        ));

        assert_eq!(controller.state(), &original);
        assert_eq!(controller.visible_state(), &original);
    }

    #[test]
    fn valid_roll_generates_legal_sources() {
        let mut controller = LocalGameController::new();

        controller
            .begin_turn(Dice {
                first: 1,
                second: 2,
            })
            .unwrap();

        assert_eq!(controller.state().turn_phase, TurnPhase::Moving);
        assert_eq!(
            controller.state().dice,
            Some(Dice {
                first: 1,
                second: 2
            })
        );
        assert!(!controller.legal_sources().is_empty());
    }

    #[test]
    fn invalid_dice_are_rejected_without_mutating_state() {
        let mut controller = LocalGameController::new();
        let original = controller.state().clone();

        assert_eq!(
            controller.begin_turn(Dice {
                first: 0,
                second: 6,
            }),
            Err(ControllerError::InvalidDice(StateError::InvalidDieValue))
        );

        assert_eq!(controller.state(), &original);
    }

    #[test]
    fn illegal_source_is_rejected() {
        let mut controller = LocalGameController::new();

        controller
            .begin_turn(Dice {
                first: 1,
                second: 2,
            })
            .unwrap();

        assert_eq!(
            controller.select_source(MoveSource::Point(3)),
            Err(ControllerError::IllegalSource)
        );
        assert_eq!(controller.selected_source(), None);
    }

    #[test]
    fn selected_source_exposes_only_legal_destinations() {
        let mut controller = LocalGameController::new();

        controller
            .begin_turn(Dice {
                first: 1,
                second: 2,
            })
            .unwrap();

        controller.select_source(MoveSource::Point(0)).unwrap();

        let destinations = controller.legal_destinations().unwrap();

        assert!(destinations.contains(&MoveTarget::Point(1)));
        assert!(destinations.contains(&MoveTarget::Point(2)));
    }

    #[test]
    fn partial_move_updates_preview_without_committing_turn() {
        let mut controller = LocalGameController::new();

        controller
            .begin_turn(Dice {
                first: 1,
                second: 2,
            })
            .unwrap();

        controller.select_source(MoveSource::Point(0)).unwrap();

        let completed = controller.choose_destination(MoveTarget::Point(1)).unwrap();

        assert!(!completed);
        assert_eq!(controller.state().points[0].count, 2);
        assert_eq!(controller.visible_state().points[0].count, 1);
        assert_eq!(controller.visible_state().points[1].count, 1);
        assert_eq!(controller.selected_moves().len(), 1);
    }

    #[test]
    fn completed_network_turn_preserves_authoritative_state() {
        let mut controller = LocalGameController::new();

        controller
            .begin_turn(Dice {
                first: 1,
                second: 2,
            })
            .unwrap();

        let authoritative = controller.state().clone();
        let sequence = prepare_first_available_turn(&mut controller);

        assert!(!sequence.moves.is_empty());
        assert_eq!(controller.state(), &authoritative);
        assert_eq!(controller.state().active_player, Player::White);
        assert_eq!(controller.state().turn_phase, TurnPhase::Moving);
        assert_eq!(
            controller.state().dice,
            Some(Dice {
                first: 1,
                second: 2,
            })
        );
        assert!(controller.history().is_empty());
        assert_eq!(controller.selected_moves(), sequence.moves.as_slice());
        assert_ne!(controller.visible_state(), controller.state());
        assert!(controller.legal_sources().is_empty());
        assert_eq!(
            controller.status_message(),
            "Turn ready for network submission."
        );
    }

    #[test]
    fn authoritative_sync_discards_prepared_network_turn() {
        let mut controller = LocalGameController::new();

        controller
            .begin_turn(Dice {
                first: 1,
                second: 2,
            })
            .unwrap();

        let authoritative = controller.state().clone();

        let _sequence = prepare_first_available_turn(&mut controller);

        controller
            .sync_authoritative_state(authoritative.clone())
            .unwrap();

        assert_eq!(controller.state(), &authoritative);
        assert_eq!(controller.visible_state(), &authoritative);
        assert!(controller.selected_moves().is_empty());
        assert_eq!(controller.selected_source(), None);
        assert!(!controller.legal_sources().is_empty());
        assert_eq!(controller.status_message(), "White rolled 1 and 2.");
    }

    #[test]
    fn complete_turn_commits_and_switches_player() {
        let mut controller = LocalGameController::new();

        controller
            .begin_turn(Dice {
                first: 1,
                second: 2,
            })
            .unwrap();

        play_first_available_turn(&mut controller);

        assert_eq!(controller.state().turn_phase, TurnPhase::AwaitingRoll);
        assert_eq!(controller.state().active_player, Player::Black);
        assert_eq!(controller.state().dice, None);
        assert_eq!(controller.visible_state(), controller.state());
        assert_eq!(controller.history().len(), 1);
        assert!(controller.selected_moves().is_empty());
    }

    #[test]
    fn forced_pass_waits_for_player_confirmation() {
        use backgammon_core::{PlayerArea, Point};

        let dice = Dice {
            first: 4,
            second: 2,
        };

        let mut points = [Point::EMPTY; 24];

        /*
         * White enters from the bar toward increasing point indices:
         *
         * - die 2 enters on index 1
         * - die 4 enters on index 3
         *
         * Both entry points contain at least two Black checkers, so neither
         * die can be played.
         */
        points[1] = Point::occupied(Player::Black, 2);
        points[3] = Point::occupied(Player::Black, 2);

        let blocked_state = GameState {
            points,
            white: PlayerArea {
                bar: 1,
                borne_off: 14,
            },
            black: PlayerArea {
                bar: 0,
                borne_off: 11,
            },
            active_player: Player::White,
            turn_phase: TurnPhase::AwaitingRoll,
            dice: None,
            status: GameStatus::InProgress,
        };

        blocked_state.verify().unwrap();

        let mut controller = LocalGameController::new();
        controller.state = blocked_state.clone();
        controller.preview_state = blocked_state;

        controller.begin_turn(dice).unwrap();

        assert!(controller.must_pass());
        assert_eq!(controller.state().active_player, Player::White);
        assert_eq!(controller.state().turn_phase, TurnPhase::Moving);
        assert_eq!(controller.state().dice, Some(dice));
        assert_eq!(controller.visible_state(), controller.state());
        assert!(controller.legal_sources().is_empty());
        assert!(controller.selected_moves().is_empty());
        assert!(controller.history().is_empty());
        assert_eq!(
            controller.status_message(),
            "White rolled 4 and 2 but has no legal move. Select Pass turn."
        );

        controller.pass_turn().unwrap();

        assert!(!controller.must_pass());
        assert_eq!(controller.state().active_player, Player::Black);
        assert_eq!(controller.state().turn_phase, TurnPhase::AwaitingRoll);
        assert_eq!(controller.state().dice, None);
        assert_eq!(controller.visible_state(), controller.state());
        assert_eq!(controller.history().len(), 1);
        assert_eq!(controller.history()[0].player, Player::White);
        assert_eq!(controller.history()[0].dice, dice);
        assert!(controller.history()[0].moves.is_empty());
    }

    #[test]
    fn forced_pass_can_be_prepared_without_local_commit() {
        use backgammon_core::{PlayerArea, Point};

        let dice = Dice {
            first: 4,
            second: 2,
        };

        let mut points = [Point::EMPTY; 24];

        points[1] = Point::occupied(Player::Black, 2);
        points[3] = Point::occupied(Player::Black, 2);

        let blocked_state = GameState {
            points,
            white: PlayerArea {
                bar: 1,
                borne_off: 14,
            },
            black: PlayerArea {
                bar: 0,
                borne_off: 11,
            },
            active_player: Player::White,
            turn_phase: TurnPhase::AwaitingRoll,
            dice: None,
            status: GameStatus::InProgress,
        };

        blocked_state.verify().unwrap();

        let mut controller = LocalGameController::new();

        controller.sync_authoritative_state(blocked_state).unwrap();

        controller.begin_turn(dice).unwrap();

        let authoritative = controller.state().clone();

        let sequence = controller.prepare_pass_for_submission().unwrap();

        assert!(sequence.moves.is_empty());
        assert_eq!(controller.state(), &authoritative);
        assert_eq!(controller.state().active_player, Player::White);
        assert_eq!(controller.state().turn_phase, TurnPhase::Moving);
        assert_eq!(controller.state().dice, Some(dice));
        assert!(controller.history().is_empty());
        assert_eq!(
            controller.status_message(),
            "Forced pass ready for network submission."
        );
    }

    #[test]
    fn prepare_pass_is_rejected_without_forced_pass() {
        let mut controller = LocalGameController::new();

        assert_eq!(
            controller.prepare_pass_for_submission(),
            Err(ControllerError::NoForcedPassPending)
        );
    }

    #[test]
    fn pass_turn_is_rejected_when_a_pass_is_not_pending() {
        let mut controller = LocalGameController::new();

        assert_eq!(
            controller.pass_turn(),
            Err(ControllerError::NoForcedPassPending)
        );

        assert_eq!(controller.state(), &GameState::standard_start());
        assert!(controller.history().is_empty());
    }

    #[test]
    fn resignation_records_winner_and_stops_play() {
        let mut controller = LocalGameController::new();

        controller.resign().unwrap();

        assert_eq!(
            controller.outcome(),
            Some(LocalGameOutcome::Resigned {
                resigned: Player::White,
                winner: Player::Black,
            })
        );
        assert!(!controller.is_active());
        assert_eq!(
            controller.begin_turn(Dice {
                first: 1,
                second: 2,
            }),
            Err(ControllerError::SessionInactive)
        );
    }

    #[test]
    fn leave_table_stops_local_session() {
        let mut controller = LocalGameController::new();

        controller.leave_table();

        assert!(controller.has_left_table());
        assert!(!controller.is_active());
        assert!(controller.legal_sources().is_empty());
    }

    #[test]
    fn completed_state_produces_scored_outcome() {
        let mut controller = LocalGameController::new();

        controller.state.status = GameStatus::Completed {
            winner: Player::Black,
            points: 3,
        };

        controller.sync_outcome_from_state();

        assert_eq!(
            controller.outcome(),
            Some(LocalGameOutcome::Completed {
                winner: Player::Black,
                points: 3,
            })
        );
        assert_eq!(
            controller.status_message(),
            "Black wins a backgammon for 3 points."
        );
    }

    #[test]
    fn new_game_clears_progress_history_and_terminal_state() {
        let mut controller = LocalGameController::new();

        controller
            .begin_turn(Dice {
                first: 1,
                second: 2,
            })
            .unwrap();

        play_first_available_turn(&mut controller);
        assert_eq!(controller.history().len(), 1);

        controller.resign().unwrap();
        controller.leave_table();
        controller.new_game();

        assert_eq!(controller.state(), &GameState::standard_start());
        assert!(controller.history().is_empty());
        assert_eq!(controller.outcome(), None);
        assert!(!controller.has_left_table());
        assert!(controller.is_active());
        assert_eq!(controller.status_message(), "White to roll.");
    }
}
