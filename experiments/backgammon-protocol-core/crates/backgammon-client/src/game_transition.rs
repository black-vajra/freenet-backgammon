use backgammon_protocol::GenesisProposal;

use crate::accepted_game_projection::{resolve_accepted_game_selection, AcceptedGame};
use crate::active_game_scope::ActiveGameScopeSnapshot;

/// Volatile navigation intent, created only by a player's challenge/accept
/// action. It is never inferred from the newest historical accepted record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameTransitionIntent {
    pub proposal: GenesisProposal,
    pub origin_scope: ActiveGameScopeSnapshot,
    pub automatic: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GameTransitionDecision {
    Idle,
    WaitingForAcceptance,
    WaitingForConnection(AcceptedGame),
    ManualActivationRequired(AcceptedGame),
    Activate(AcceptedGame),
    AlreadyActive,
    Superseded,
}

/// Candidates must come from project_accepted_games on verified lobby state.
/// Even then, resolve the exact game uniquely and compare the entire proposal,
/// including signed colors, before permitting navigation to that game.
pub fn plan_game_transition(
    intent: Option<&GameTransitionIntent>,
    current_scope: &ActiveGameScopeSnapshot,
    active_game: Option<&AcceptedGame>,
    accepted_games: &[AcceptedGame],
    connection_ready: bool,
) -> Result<GameTransitionDecision, String> {
    let Some(intent) = intent else {
        return Ok(GameTransitionDecision::Idle);
    };
    if active_game.is_some_and(|game| game.accepted_proposal == intent.proposal) {
        return Ok(GameTransitionDecision::AlreadyActive);
    }
    if current_scope != &intent.origin_scope {
        return Ok(GameTransitionDecision::Superseded);
    }
    if !accepted_games.iter().any(|game| game.game_id == intent.proposal.game_id) {
        return Ok(GameTransitionDecision::WaitingForAcceptance);
    }
    let accepted = resolve_accepted_game_selection(Some(intent.proposal.game_id), accepted_games)?
        .ok_or_else(|| "Requested game has no accepted candidate".to_owned())?;
    if accepted.accepted_proposal != intent.proposal {
        return Err("Accepted game differs from the exact challenge you chose".to_owned());
    }
    if !intent.automatic {
        return Ok(GameTransitionDecision::ManualActivationRequired(accepted.clone()));
    }
    if !connection_ready {
        return Ok(GameTransitionDecision::WaitingForConnection(accepted.clone()));
    }
    Ok(GameTransitionDecision::Activate(accepted.clone()))
}

/// Recheck after durable awaits: an intervening lobby update must not leave
/// a removed, conflicting, or substituted accepted candidate eligible.
pub fn verify_activation_candidate(
    selected: &AcceptedGame,
    accepted_games: &[AcceptedGame],
) -> Result<(), String> {
    let current = resolve_accepted_game_selection(Some(selected.game_id), accepted_games)?
        .ok_or_else(|| "Accepted game is no longer available".to_owned())?;
    if current != selected {
        return Err("Accepted game changed while preparing activation".to_owned());
    }
    Ok(())
}

/// An activation can await several durable replies. Duplicate clicks cannot
/// overlap it, and cancelled work cannot finish a later activation attempt.
#[derive(Default)]
pub struct ActivationGate {
    next_id: u64,
    current: Option<u64>,
}

impl ActivationGate {
    pub fn is_busy(&self) -> bool {
        self.current.is_some()
    }

    pub fn begin(&mut self) -> Option<u64> {
        if self.current.is_some() {
            return None;
        }
        self.next_id = self.next_id.checked_add(1)?;
        self.current = Some(self.next_id);
        self.current
    }

    pub fn is_current(&self, request: u64) -> bool {
        self.current == Some(request)
    }

    pub fn finish(&mut self, request: u64) -> bool {
        if !self.is_current(request) {
            return false;
        }
        self.current = None;
        true
    }

    pub fn cancel(&mut self) {
        self.current = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use backgammon_core::Player;
    use backgammon_lobby_core::ChallengeOfferState;
    use backgammon_protocol::{accept_challenge, ChallengeTerminalEvidence};
    use ed25519_dalek::SigningKey;
    use crate::accepted_game_projection::project_accepted_games;
    use crate::challenge_offer_planner::{plan_outbound_challenge, OutboundChallengePlannerInput};

    fn fixture(role: Player, game_seed: u8) -> (GameTransitionIntent, AcceptedGame, AcceptedGame) {
        let challenger = SigningKey::from_bytes(&[41; 32]);
        let recipient = SigningKey::from_bytes(&[42; 32]);
        let plan = plan_outbound_challenge(OutboundChallengePlannerInput {
            signing_key: &challenger,
            challenger_display_name: "Dada",
            recipient_id: recipient.verifying_key().to_bytes(),
            recipient_display_name: "Mama",
            challenger_role: role,
            match_length: 1,
            challenge_id: [game_seed + 10; 32],
            game_id: [game_seed; 32],
            genesis_action_id: [game_seed + 20; 32],
            created_at_unix_seconds: 700_000,
            expires_at_unix_seconds: 700_600,
        }).unwrap();
        let acceptance = accept_challenge(&plan.signed_offer, &recipient, 700_001).unwrap();
        let state = ChallengeOfferState::new(plan.signed_offer.clone(),
            vec![ChallengeTerminalEvidence::Acceptance(acceptance)]).unwrap();
        let challenger_game = project_accepted_games(challenger.verifying_key().to_bytes(), &[state.clone()])
            .unwrap().remove(0);
        let recipient_game = project_accepted_games(recipient.verifying_key().to_bytes(), &[state])
            .unwrap().remove(0);
        let intent = GameTransitionIntent {
            proposal: plan.signed_offer.body.proposal,
            origin_scope: ActiveGameScopeSnapshot { epoch: 4, contract_id: "completed-game".to_owned() },
            automatic: true,
        };
        (intent, challenger_game, recipient_game)
    }

    fn decision(intent: &GameTransitionIntent, games: &[AcceptedGame], ready: bool) -> Result<GameTransitionDecision, String> {
        plan_game_transition(Some(intent), &intent.origin_scope, None, games, ready)
    }

    #[test]
    fn switched_colors_activate_exact_same_game_for_both_participants() {
        let (intent, dada, mama) = fixture(Player::White, 51);
        assert_eq!(dada.local_role, Player::White);
        assert_eq!(mama.local_role, Player::Black);
        assert_eq!(decision(&intent, &[dada.clone()], true), Ok(GameTransitionDecision::Activate(dada)));
        assert_eq!(decision(&intent, &[mama.clone()], true), Ok(GameTransitionDecision::Activate(mama)));
    }

    #[test]
    fn crossed_offer_and_historical_games_cannot_replace_chosen_rematch() {
        let (intent, chosen, _) = fixture(Player::Black, 51);
        let (_, crossed, _) = fixture(Player::White, 53);
        assert_eq!(decision(&intent, &[crossed.clone()], true), Ok(GameTransitionDecision::WaitingForAcceptance));
        assert_eq!(decision(&intent, &[crossed, chosen.clone()], true), Ok(GameTransitionDecision::Activate(chosen)));
    }

    #[test]
    fn an_accepted_game_does_not_activate_without_player_intent() {
        let (intent, game, _) = fixture(Player::White, 51);
        assert_eq!(plan_game_transition(None, &intent.origin_scope, None, &[game], true), Ok(GameTransitionDecision::Idle));
    }

    #[test]
    fn ambiguous_or_substituted_proposals_fail_closed() {
        let (intent, game, _) = fixture(Player::White, 51);
        assert!(decision(&intent, &[game.clone(), game.clone()], true).is_err());
        let mut substituted = game;
        substituted.accepted_proposal.configuration.match_length = 3;
        assert!(decision(&intent, &[substituted], true).is_err());
    }

    #[test]
    fn accepted_rematch_waits_for_current_connection_and_delegate() {
        let (intent, game, _) = fixture(Player::White, 51);
        assert_eq!(decision(&intent, &[game.clone()], false), Ok(GameTransitionDecision::WaitingForConnection(game)));
    }

    #[test]
    fn challenge_during_live_game_requires_manual_activation() {
        let (mut intent, game, _) = fixture(Player::White, 51);
        intent.automatic = false;
        assert_eq!(decision(&intent, &[game.clone()], true), Ok(GameTransitionDecision::ManualActivationRequired(game)));
    }

    #[test]
    fn later_scope_change_supersedes_waiting_intent() {
        let (intent, game, _) = fixture(Player::White, 51);
        let later = ActiveGameScopeSnapshot { epoch: 5, contract_id: "another-game".to_owned() };
        assert_eq!(plan_game_transition(Some(&intent), &later, None, &[game], true), Ok(GameTransitionDecision::Superseded));
    }

    #[test]
    fn activation_success_does_not_start_the_same_game_again() {
        let (intent, game, _) = fixture(Player::White, 51);
        let current = ActiveGameScopeSnapshot { epoch: 5, contract_id: game.contract_id.clone() };
        assert_eq!(plan_game_transition(Some(&intent), &current, Some(&game), &[game.clone()], true), Ok(GameTransitionDecision::AlreadyActive));
    }

    #[test]
    fn removed_or_changed_acceptance_cannot_finish_activation() {
        let (_, game, _) = fixture(Player::White, 51);
        assert!(verify_activation_candidate(&game, &[]).is_err());
        let mut changed = game.clone();
        changed.accepted_proposal.configuration.match_length = 3;
        assert!(verify_activation_candidate(&game, &[changed]).is_err());
        assert_eq!(verify_activation_candidate(&game, &[game.clone()]), Ok(()));
    }

    #[test]
    fn duplicate_clicks_and_cancelled_replies_cannot_overlap_activation() {
        let mut gate = ActivationGate::default();
        let first = gate.begin().unwrap();
        assert!(gate.begin().is_none());
        gate.cancel();
        let second = gate.begin().unwrap();
        assert_ne!(first, second);
        assert!(!gate.finish(first));
        assert!(gate.is_current(second));
        assert!(gate.finish(second));
        assert!(!gate.is_current(second));
        assert!(gate.begin().is_some());
    }
}
