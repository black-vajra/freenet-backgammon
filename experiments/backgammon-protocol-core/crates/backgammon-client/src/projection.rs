use backgammon_core::{Dice, GameState, GameStatus, Player, TurnPhase, POINT_COUNT};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoardView {
    pub points: [PointView; POINT_COUNT],
    pub white_bar: u8,
    pub black_bar: u8,
    pub white_borne_off: u8,
    pub black_borne_off: u8,
    pub active_player: Player,
    pub turn_phase: TurnPhase,
    pub dice: Option<Dice>,
    pub status: GameStatus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PointView {
    pub index: usize,
    pub owner: Option<Player>,
    pub count: u8,
}

impl From<&GameState> for BoardView {
    fn from(state: &GameState) -> Self {
        let points: [PointView; POINT_COUNT] = state
            .points
            .iter()
            .enumerate()
            .map(|(index, point)| PointView {
                index,
                owner: point.owner,
                count: point.count,
            })
            .collect::<Vec<_>>()
            .try_into()
            .expect("POINT_COUNT mismatch");

        Self {
            points,
            white_bar: state.white.bar,
            black_bar: state.black.bar,
            white_borne_off: state.white.borne_off,
            black_borne_off: state.black.borne_off,
            active_player: state.active_player,
            turn_phase: state.turn_phase,
            dice: state.dice,
            status: state.status,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_start_projects_expected_positions() {
        let state = GameState::standard_start();
        let view = BoardView::from(&state);

        assert_eq!(view.points.len(), POINT_COUNT);

        assert_eq!(view.points[0].owner, Some(Player::White));
        assert_eq!(view.points[0].count, 2);

        assert_eq!(view.points[11].owner, Some(Player::White));
        assert_eq!(view.points[11].count, 5);

        assert_eq!(view.points[23].owner, Some(Player::Black));
        assert_eq!(view.points[23].count, 2);

        assert_eq!(view.points[12].owner, Some(Player::Black));
        assert_eq!(view.points[12].count, 5);
    }

    #[test]
    fn projection_preserves_turn_state() {
        let state = GameState::standard_start();
        let view = BoardView::from(&state);

        assert_eq!(view.active_player, Player::White);
        assert_eq!(view.turn_phase, TurnPhase::AwaitingRoll);
        assert_eq!(view.dice, None);
    }
}
