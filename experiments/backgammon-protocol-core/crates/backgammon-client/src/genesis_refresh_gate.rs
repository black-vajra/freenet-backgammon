use crate::active_game_scope::ActiveGameScopeSnapshot;

/// Requests one game-ledger read when verified lobby evidence first makes
/// authenticated genesis ready. A new connection may retry the same read.
#[derive(Default)]
pub struct GenesisRefreshGate {
    requested_for: Option<ActiveGameScopeSnapshot>,
    connection_epoch: u64,
}

impl GenesisRefreshGate {
    pub fn claim(
        &mut self,
        scope: &ActiveGameScopeSnapshot,
        verified_ready: bool,
        has_connection: bool,
    ) -> bool {
        if !verified_ready || !has_connection || self.requested_for.as_ref() == Some(scope) {
            return false;
        }

        self.requested_for = Some(scope.clone());
        true
    }

    pub fn connection_epoch(&self) -> u64 {
        self.connection_epoch
    }

    pub fn owns_request(&self, scope: &ActiveGameScopeSnapshot, epoch: u64) -> bool {
        self.connection_epoch == epoch && self.requested_for.as_ref() == Some(scope)
    }

    pub fn revoke(&mut self, scope: &ActiveGameScopeSnapshot) {
        if self.requested_for.as_ref() == Some(scope) {
            self.requested_for = None;
        }
    }

    pub fn reset_connection(&mut self) {
        self.requested_for = None;
        self.connection_epoch = self.connection_epoch.wrapping_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_response_before_readiness_is_woken_once_and_retried_after_disconnect() {
        let scope = ActiveGameScopeSnapshot {
            epoch: 1,
            contract_id: "accepted-game".to_owned(),
        };
        let mut gate = GenesisRefreshGate::default();

        // The initial empty game response precedes the second lobby share.
        assert!(!gate.claim(&scope, false, true));
        assert!(!gate.claim(&scope, true, false));
        assert!(gate.claim(&scope, true, true));
        assert!(!gate.claim(&scope, true, true));
        let first_epoch = gate.connection_epoch();

        // A closed socket before submission must not consume the retry.
        gate.reset_connection();
        assert!(!gate.owns_request(&scope, first_epoch));
        assert!(gate.claim(&scope, true, true));
        assert!(!gate.claim(&scope, true, true));
        assert!(gate.owns_request(&scope, gate.connection_epoch()));
    }

    #[test]
    fn old_scope_cannot_claim_a_new_games_wakeup() {
        let old = ActiveGameScopeSnapshot {
            epoch: 1,
            contract_id: "old-game".to_owned(),
        };
        let current = ActiveGameScopeSnapshot {
            epoch: 2,
            contract_id: "current-game".to_owned(),
        };
        let mut gate = GenesisRefreshGate::default();

        assert!(gate.claim(&old, true, true));
        assert!(gate.claim(&current, true, true));
        assert!(!gate.owns_request(&old, gate.connection_epoch()));
        assert!(!gate.claim(&current, true, true));
        gate.revoke(&old);
        assert!(!gate.claim(&current, true, true));
        gate.revoke(&current);
        assert!(gate.claim(&current, true, true));
    }
}
