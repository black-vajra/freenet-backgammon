use backgammon_protocol::PlayerId;

use crate::accepted_game_projection::AcceptedGame;
use crate::game_contract_publication::calculate_expected_game_contract;
use crate::local_identity_store::role_for_player_id;

const MAX_CONTRACT_ID_BYTES: usize = 128;

/// Immutable identity of one active browser game scope.
///
/// `epoch` changes whenever an accepted game is activated. Asynchronous work
/// may mutate game-scoped browser state only while its captured snapshot still
/// matches the current scope.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActiveGameScope {
    epoch: u64,
    contract_id: String,
    accepted_game: Option<AcceptedGame>,
}

/// Minimal value captured by delayed game work.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActiveGameScopeSnapshot {
    pub epoch: u64,
    pub contract_id: String,
}

impl ActiveGameScope {
    /// Creates the initial compatibility scope for the existing published test
    /// contract. It contains no accepted-game authority.
    pub fn initial_test(contract_id: &str) -> Result<Self, String> {
        validate_contract_id(contract_id)?;

        Ok(Self {
            epoch: 0,
            contract_id: contract_id.to_owned(),
            accepted_game: None,
        })
    }

    /// Produces the next scope from an authoritative accepted-game candidate.
    ///
    /// All public fields of `AcceptedGame` are independently cross-checked
    /// before activation, including the proposal game ID, calculated Freenet
    /// contract identity, persistent local identity role, and peer identity.
    pub fn activate_accepted(
        &self,
        local_player_id: PlayerId,
        accepted: &AcceptedGame,
    ) -> Result<Self, String> {
        if accepted.accepted_proposal.game_id != accepted.game_id {
            return Err("Accepted-game proposal carries a different game ID.".to_owned());
        }

        let expected_contract = calculate_expected_game_contract(accepted.game_id)?;

        if accepted.contract_key != expected_contract.full_key {
            return Err("Accepted game carries an unexpected full contract key.".to_owned());
        }

        if accepted.contract_id != expected_contract.contract_id
            || accepted.contract_key.id().encode() != accepted.contract_id
        {
            return Err("Accepted game carries a noncanonical contract ID.".to_owned());
        }

        let configuration = &accepted.accepted_proposal.configuration;

        let expected_role =
            role_for_player_id(configuration, &local_player_id).ok_or_else(|| {
                "Persistent local identity is not a participant in the \
                     accepted game."
                    .to_owned()
            })?;

        if accepted.local_role != expected_role {
            return Err("Accepted game carries the wrong local player role.".to_owned());
        }

        let expected_peer = match expected_role {
            backgammon_core::Player::White => configuration.black.id,
            backgammon_core::Player::Black => configuration.white.id,
        };

        if accepted.peer_id != expected_peer {
            return Err("Accepted game carries the wrong peer identity.".to_owned());
        }

        let epoch = self
            .epoch
            .checked_add(1)
            .ok_or_else(|| "Active-game scope epoch overflowed.".to_owned())?;

        Ok(Self {
            epoch,
            contract_id: accepted.contract_id.clone(),
            accepted_game: Some(accepted.clone()),
        })
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    pub fn contract_id(&self) -> &str {
        &self.contract_id
    }

    pub fn accepted_game(&self) -> Option<&AcceptedGame> {
        self.accepted_game.as_ref()
    }

    pub fn snapshot(&self) -> ActiveGameScopeSnapshot {
        ActiveGameScopeSnapshot {
            epoch: self.epoch,
            contract_id: self.contract_id.clone(),
        }
    }

    pub fn recognizes(&self, snapshot: &ActiveGameScopeSnapshot) -> bool {
        self.epoch == snapshot.epoch && self.contract_id == snapshot.contract_id
    }
}

fn validate_contract_id(contract_id: &str) -> Result<(), String> {
    if contract_id.is_empty() {
        return Err("Active game contract ID is empty.".to_owned());
    }

    if contract_id.len() > MAX_CONTRACT_ID_BYTES {
        return Err(format!(
            "Active game contract ID exceeds {MAX_CONTRACT_ID_BYTES} bytes."
        ));
    }

    if !contract_id
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err("Active game contract ID contains unsupported characters.".to_owned());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use backgammon_lobby_core::ChallengeOfferState;
    use backgammon_protocol::{accept_challenge, ChallengeTerminalEvidence};
    use ed25519_dalek::SigningKey;

    use crate::accepted_game_projection::project_accepted_games;
    use crate::challenge_offer_planner::{plan_outbound_challenge, OutboundChallengePlannerInput};

    fn key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn accepted_fixture() -> (PlayerId, AcceptedGame) {
        let challenger = key(41);
        let recipient = key(42);

        let plan = plan_outbound_challenge(OutboundChallengePlannerInput {
            signing_key: &challenger,
            challenger_display_name: "Alice",
            recipient_id: recipient.verifying_key().to_bytes(),
            recipient_display_name: "Bob",
            match_length: 3,
            challenge_id: [51; 32],
            game_id: [52; 32],
            genesis_action_id: [53; 32],
            created_at_unix_seconds: 700_000,
            expires_at_unix_seconds: 700_600,
        })
        .unwrap();

        let acceptance = accept_challenge(&plan.signed_offer, &recipient, 700_001).unwrap();

        let state = ChallengeOfferState::new(
            plan.signed_offer,
            vec![ChallengeTerminalEvidence::Acceptance(acceptance)],
        )
        .unwrap();

        let local_player_id = recipient.verifying_key().to_bytes();

        let accepted = project_accepted_games(local_player_id, &[state])
            .unwrap()
            .into_iter()
            .next()
            .unwrap();

        (local_player_id, accepted)
    }

    #[test]
    fn initial_test_scope_has_epoch_zero_and_no_accepted_authority() {
        let scope = ActiveGameScope::initial_test("test-contract_123").unwrap();

        assert_eq!(scope.epoch(), 0);
        assert_eq!(scope.contract_id(), "test-contract_123");
        assert_eq!(scope.accepted_game(), None);
    }

    #[test]
    fn malformed_initial_contract_ids_are_rejected() {
        assert!(ActiveGameScope::initial_test("").is_err());
        assert!(ActiveGameScope::initial_test("contains space").is_err());
        assert!(ActiveGameScope::initial_test("contains/slash").is_err());
        assert!(ActiveGameScope::initial_test(&"a".repeat(129)).is_err());
    }

    #[test]
    fn accepted_activation_increments_epoch_and_preserves_exact_target() {
        let initial = ActiveGameScope::initial_test("test-contract").unwrap();

        let (local_player_id, accepted) = accepted_fixture();

        let active = initial
            .activate_accepted(local_player_id, &accepted)
            .unwrap();

        assert_eq!(active.epoch(), 1);
        assert_eq!(active.contract_id(), accepted.contract_id);
        assert_eq!(active.accepted_game(), Some(&accepted));
    }

    #[test]
    fn tampered_accepted_contract_identity_is_rejected() {
        let initial = ActiveGameScope::initial_test("test-contract").unwrap();

        let (local_player_id, mut accepted) = accepted_fixture();
        accepted.contract_id = "different-contract".to_owned();

        assert!(initial
            .activate_accepted(local_player_id, &accepted)
            .is_err());
    }

    #[test]
    fn tampered_role_or_peer_is_rejected() {
        let initial = ActiveGameScope::initial_test("test-contract").unwrap();

        let (local_player_id, accepted) = accepted_fixture();

        let mut wrong_role = accepted.clone();
        wrong_role.local_role = match wrong_role.local_role {
            backgammon_core::Player::White => backgammon_core::Player::Black,
            backgammon_core::Player::Black => backgammon_core::Player::White,
        };

        assert!(initial
            .activate_accepted(local_player_id, &wrong_role)
            .is_err());

        let mut wrong_peer = accepted;
        wrong_peer.peer_id = [99; 32];

        assert!(initial
            .activate_accepted(local_player_id, &wrong_peer)
            .is_err());
    }

    #[test]
    fn snapshots_match_only_the_scope_that_created_them() {
        let initial = ActiveGameScope::initial_test("test-contract").unwrap();

        let old_snapshot = initial.snapshot();
        assert!(initial.recognizes(&old_snapshot));

        let (local_player_id, accepted) = accepted_fixture();

        let active = initial
            .activate_accepted(local_player_id, &accepted)
            .unwrap();

        assert!(!active.recognizes(&old_snapshot));
        assert!(active.recognizes(&active.snapshot()));
    }
}
