use backgammon_core::{GameStatus, Player};

/// Only an identified winner of a verified completed game gets celebration.
pub fn local_player_won(local_role: Option<Player>, verified_status: Option<GameStatus>) -> bool {
    match (local_role, verified_status) {
        (Some(local), Some(GameStatus::Completed { winner, .. })) => local == winner,
        _ => false,
    }
}

/// Resolve a freshly recovered identity against the same verified snapshot
/// that supplied the completed status. No separate raw ledger cache is used.
pub fn local_identity_won(
    local_id: Option<[u8; 32]>, white_id: [u8; 32], black_id: [u8; 32], status: GameStatus,
) -> bool {
    if white_id == black_id { return false; }
    let role = match local_id {
        Some(id) if id == white_id => Some(Player::White),
        Some(id) if id == black_id => Some(Player::Black),
        _ => None,
    };
    local_player_won(role, Some(status))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn celebration_follows_winner_for_both_player_colors() {
        for winner in [Player::White, Player::Black] {
            let status = Some(GameStatus::Completed { winner, points: 1 });
            assert!(local_player_won(Some(winner), status));
            assert!(!local_player_won(Some(winner.opponent()), status));
            assert!(!local_player_won(None, status));
        }
    }

    #[test]
    fn recovered_recipient_and_challenger_identities_follow_verified_colors() {
        let challenger = [11; 32];
        let recipient = [22; 32];
        for (white, black) in [(challenger, recipient), (recipient, challenger)] {
            for winner in [Player::White, Player::Black] {
                let status = GameStatus::Completed { winner, points: 3 };
                let winner_id = if winner == Player::White { white } else { black };
                let loser_id = if winner == Player::White { black } else { white };
                assert!(!local_identity_won(None, white, black, status));
                assert!(local_identity_won(Some(winner_id), white, black, status));
                assert!(!local_identity_won(Some(loser_id), white, black, status));
                assert!(!local_identity_won(Some([33; 32]), white, black, status));
                assert!(!local_identity_won(Some(winner_id), white, black, GameStatus::InProgress));
            }
        }
        assert!(!local_identity_won(Some(challenger), challenger, challenger,
            GameStatus::Completed { winner: Player::White, points: 1 }));
    }

    #[test]
    fn unverified_or_unfinished_games_never_celebrate() {
        assert!(!local_player_won(Some(Player::White), None));
        assert!(!local_player_won(Some(Player::White), Some(GameStatus::InProgress)));
    }
}
