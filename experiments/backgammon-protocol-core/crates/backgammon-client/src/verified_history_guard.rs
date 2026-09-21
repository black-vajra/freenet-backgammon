use crate::active_game_scope::ActiveGameScopeSnapshot;
use backgammon_protocol::GameActionRecord;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryDecision {
    Advance,
    Unchanged,
    Stale,
    Divergent,
}

/// Retains the last accepted verified action prefix for one activated game.
/// Transport reconnects must preserve this cursor: a late response from a
/// fresh connection cannot move the visible board backward.
#[derive(Default)]
pub struct VerifiedHistoryGuard {
    scope: Option<ActiveGameScopeSnapshot>,
    actions: Vec<GameActionRecord>,
}

impl VerifiedHistoryGuard {
    pub fn action_count(&self, scope: &ActiveGameScopeSnapshot) -> Option<usize> {
        (self.scope.as_ref() == Some(scope)).then_some(self.actions.len())
    }

    pub fn observe(
        &mut self,
        scope: &ActiveGameScopeSnapshot,
        actions: &[GameActionRecord],
    ) -> HistoryDecision {
        if self.scope.as_ref() != Some(scope) {
            self.scope = Some(scope.clone());
            self.actions = actions.to_vec();
            return HistoryDecision::Advance;
        }

        let shared = self.actions.len().min(actions.len());
        if self.actions[..shared] != actions[..shared] {
            return HistoryDecision::Divergent;
        }

        match actions.len().cmp(&self.actions.len()) {
            std::cmp::Ordering::Less => HistoryDecision::Stale,
            std::cmp::Ordering::Equal => HistoryDecision::Unchanged,
            std::cmp::Ordering::Greater => {
                self.actions = actions.to_vec();
                HistoryDecision::Advance
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope(epoch: u64) -> ActiveGameScopeSnapshot {
        ActiveGameScopeSnapshot {
            epoch,
            contract_id: "game".to_owned(),
        }
    }

    fn history(count: usize) -> Vec<GameActionRecord> {
        (0..count)
            .map(|index| GameActionRecord {
                protocol_version: backgammon_protocol::PROTOCOL_VERSION,
                game_id: [1; 32],
                action_id: [index as u8; 32],
                sequence: index as u64,
                previous_state_hash: [0; 32],
                resulting_state_hash: [0; 32],
                payload: backgammon_protocol::GameActionPayload::Resign {
                    player: backgammon_core::Player::White,
                },
            })
            .collect()
    }

    #[test]
    fn thirty_three_then_thirty_one_then_thirty_four_never_rewinds() {
        let mut guard = VerifiedHistoryGuard::default();
        let game = scope(1);
        assert_eq!(guard.observe(&game, &history(33)), HistoryDecision::Advance);
        assert_eq!(guard.observe(&game, &history(31)), HistoryDecision::Stale);
        assert_eq!(guard.actions.len(), 33);
        assert_eq!(guard.observe(&game, &history(34)), HistoryDecision::Advance);
        assert_eq!(guard.actions.len(), 34);
    }

    #[test]
    fn diverging_prefix_is_quarantined_and_new_scope_is_independent() {
        let mut guard = VerifiedHistoryGuard::default();
        let mut alternate = history(4);
        assert_eq!(
            guard.observe(&scope(1), &alternate),
            HistoryDecision::Advance
        );
        alternate[1].action_id = [255; 32];
        assert_eq!(
            guard.observe(&scope(1), &alternate),
            HistoryDecision::Divergent
        );
        assert_eq!(guard.actions[1].action_id, [1; 32]);
        assert_eq!(
            guard.observe(&scope(2), &alternate),
            HistoryDecision::Advance
        );
    }
}
