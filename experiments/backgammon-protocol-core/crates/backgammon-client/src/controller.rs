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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ControllerError {
    GameAlreadyCompleted,
    NotAwaitingRoll,
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
            status_message: "White to roll.".to_owned(),
        }
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

    pub fn status_message(&self) -> &str {
        &self.status_message
    }

    pub fn new_game(&mut self) {
        *self = Self::new();
    }

    pub fn begin_turn(&mut self, dice: Dice) -> Result<(), ControllerError> {
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

        if self
            .legal_sequences
            .iter()
            .any(|sequence| sequence.moves.is_empty())
        {
            self.commit_sequence(TurnSequence::default())?;
            self.status_message =
                format!("{} had no legal move. Turn passed.", player_name(player));
        }

        Ok(())
    }

    pub fn legal_sources(&self) -> Vec<MoveSource> {
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
        let source = self
            .selected_source
            .ok_or(ControllerError::NoSourceSelected)?;

        self.legal_destinations_for(source)
    }

    pub fn legal_destinations_for(
        &self,
        source: MoveSource,
    ) -> Result<Vec<MoveTarget>, ControllerError> {
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

        if turn_is_complete {
            let sequence = TurnSequence {
                moves: self.selected_moves.clone(),
            };

            self.commit_sequence(sequence)?;
            return Ok(true);
        }

        self.status_message = format!(
            "{} move selected. Choose the next checker.",
            self.selected_moves.len()
        );

        Ok(false)
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
        self.status_message = format!("{} to roll.", player_name(self.state.active_player));

        Ok(())
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
        while controller.state().turn_phase == TurnPhase::Moving {
            let source = controller.legal_sources()[0];
            controller.select_source(source).unwrap();

            let destination = controller.legal_destinations().unwrap()[0];
            controller.choose_destination(destination).unwrap();
        }
    }

    #[test]
    fn new_controller_starts_with_standard_game() {
        let controller = LocalGameController::new();

        assert_eq!(controller.state(), &GameState::standard_start());
        assert_eq!(controller.visible_state(), controller.state());
        assert!(controller.history().is_empty());
        assert!(controller.legal_sources().is_empty());
        assert_eq!(controller.status_message(), "White to roll.");
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
    fn new_game_clears_progress_and_history() {
        let mut controller = LocalGameController::new();

        controller
            .begin_turn(Dice {
                first: 1,
                second: 2,
            })
            .unwrap();

        play_first_available_turn(&mut controller);
        assert_eq!(controller.history().len(), 1);

        controller.new_game();

        assert_eq!(controller.state(), &GameState::standard_start());
        assert!(controller.history().is_empty());
        assert_eq!(controller.status_message(), "White to roll.");
    }
}
