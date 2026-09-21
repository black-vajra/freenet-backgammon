use backgammon_protocol::GameId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AcceptedGameSection {
    Current,
    AwaitingActivation,
    Recoverable,
    FailedActivation,
    CompletedArchive,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InspectedGameStatus {
    Empty,
    InProgress,
    Completed,
    MissingContract,
    Failed(String),
}

impl AcceptedGameSection {
    pub const ALL: [Self; 5] = [
        Self::Current,
        Self::AwaitingActivation,
        Self::Recoverable,
        Self::FailedActivation,
        Self::CompletedArchive,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Current => "Current game",
            Self::AwaitingActivation => "Awaiting activation",
            Self::Recoverable => "Recoverable / uninspected",
            Self::FailedActivation => "Failed activation",
            Self::CompletedArchive => "Completed archive",
        }
    }
}

/// An older accepted record remains recoverable until its own contract has
/// been independently fetched and replayed.
pub fn classify_accepted_game(
    game_id: GameId,
    active_game_id: Option<GameId>,
    selected_game_id: Option<GameId>,
    active_game_completed: bool,
    selected_activation_failed: bool,
    inspected: Option<&InspectedGameStatus>,
) -> AcceptedGameSection {
    if active_game_id == Some(game_id) || inspected == Some(&InspectedGameStatus::Completed) {
        return if active_game_completed || inspected == Some(&InspectedGameStatus::Completed) {
            AcceptedGameSection::CompletedArchive
        } else {
            AcceptedGameSection::Current
        };
    }
    if selected_game_id == Some(game_id) {
        return if selected_activation_failed
            || inspected == Some(&InspectedGameStatus::MissingContract)
        {
            AcceptedGameSection::FailedActivation
        } else {
            AcceptedGameSection::AwaitingActivation
        };
    }
    if inspected == Some(&InspectedGameStatus::MissingContract) {
        return AcceptedGameSection::FailedActivation;
    }
    if inspected == Some(&InspectedGameStatus::Empty) {
        return AcceptedGameSection::AwaitingActivation;
    }
    AcceptedGameSection::Recoverable
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_requires_verified_terminal_active_game() {
        assert_eq!(
            classify_accepted_game([1; 32], Some([1; 32]), Some([1; 32]), true, false, None),
            AcceptedGameSection::CompletedArchive
        );
        assert_eq!(
            classify_accepted_game([2; 32], Some([1; 32]), None, true, false, None),
            AcceptedGameSection::Recoverable
        );
    }

    #[test]
    fn failed_selection_is_distinct_from_uninspected_history() {
        assert_eq!(
            classify_accepted_game([3; 32], None, Some([3; 32]), false, true, None),
            AcceptedGameSection::FailedActivation
        );
    }

    #[test]
    fn inspected_old_game_moves_to_completed_archive() {
        assert_eq!(
            classify_accepted_game(
                [4; 32],
                Some([1; 32]),
                None,
                false,
                false,
                Some(&InspectedGameStatus::Completed),
            ),
            AcceptedGameSection::CompletedArchive
        );
    }
}
