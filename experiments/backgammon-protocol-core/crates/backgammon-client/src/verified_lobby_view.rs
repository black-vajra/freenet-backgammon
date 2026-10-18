//! Current verified lobby evidence for long-lived transport callbacks.
//!
//! Browser render handles are snapshots. Share this store through a stable
//! Rc<RefCell<_>>, update it synchronously before scheduling render, and read
//! it again after asynchronous acceptance prerequisites have completed.
use backgammon_lobby_core::{ChallengeOfferState, LobbyContractState};
use backgammon_protocol::SignedChallengeOffer;

use crate::lobby_codec::verify_lobby_state;

#[derive(Default)]
pub struct VerifiedLobbyView {
    state: Option<LobbyContractState>,
}

impl VerifiedLobbyView {
    /// Use the contract's authenticated, bounded canonical merge rules. A
    /// delayed complete response cannot erase already observed acceptance.
    /// Work on a copy so rejected input never changes the last verified view.
    pub fn observe(&mut self, incoming: LobbyContractState) -> Result<LobbyContractState, String> {
        verify_lobby_state(&incoming)?;
        let next = match &self.state {
            Some(current) => {
                let mut merged = current.clone();
                merged.lobby.0.merge_from(&incoming.lobby.0)?;
                merged.challenges.merge_from(&incoming.challenges)?;
                merged
            }
            None => incoming,
        };
        verify_lobby_state(&next)?;
        self.state = Some(next.clone());
        // Keep only the canonically retained state, not a separate unbounded
        // archive or a selected-offer override.
        Ok(next)
    }

    pub fn exact_offer(&self, offer: &SignedChallengeOffer) -> Result<ChallengeOfferState, String> {
        let state = self.state.as_ref().ok_or_else(||
            "Current verified lobby state is unavailable before acceptance finalization".to_owned())?;
        let mut matches = state.challenges.offers.iter().filter(|entry| entry.offer == *offer);
        let found = matches.next().ok_or_else(||
            "The probed signed challenge is absent from the current verified lobby view".to_owned())?;
        if matches.next().is_some() {
            return Err("Current verified lobby contains ambiguous duplicate signed offers".to_owned());
        }
        Ok(found.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, rc::Rc};
    use backgammon_core::Player;
    use backgammon_lobby_core::ChallengeEntries;
    use backgammon_protocol::{accept_challenge, ChallengeTerminalEvidence};
    use ed25519_dalek::SigningKey;
    use crate::challenge_offer_planner::{plan_outbound_challenge, OutboundChallengePlannerInput};
    use crate::accepted_game_projection::project_accepted_games;
    use crate::active_game_scope::ActiveGameScopeSnapshot;
    use crate::game_transition::{plan_game_transition, GameTransitionDecision, GameTransitionIntent};

    fn offer(seed: u8, role: Player) -> SignedChallengeOffer {
        let challenger = SigningKey::from_bytes(&[41; 32]);
        let recipient = SigningKey::from_bytes(&[42; 32]);
        plan_outbound_challenge(OutboundChallengePlannerInput {
            signing_key: &challenger,
            challenger_display_name: "Dada", recipient_id: recipient.verifying_key().to_bytes(),
            recipient_display_name: "Mama", challenger_role: role, match_length: 1,
            challenge_id: [seed; 32], game_id: [seed + 40; 32], genesis_action_id: [seed + 80; 32],
            created_at_unix_seconds: 700_000 + u64::from(seed),
            expires_at_unix_seconds: 700_600 + u64::from(seed),
        }).unwrap().signed_offer
    }

    fn entry(offer: SignedChallengeOffer, accepted: bool) -> ChallengeOfferState {
        let evidence = if accepted {
            vec![ChallengeTerminalEvidence::Acceptance(accept_challenge(&offer,
                &SigningKey::from_bytes(&[42; 32]), 700_100).unwrap())]
        } else { vec![] };
        ChallengeOfferState::new(offer, evidence).unwrap()
    }

    fn state(entries: Vec<ChallengeOfferState>) -> LobbyContractState {
        LobbyContractState { lobby: Default::default(), challenges: ChallengeEntries::new(entries).unwrap() }
    }

    #[test]
    fn callback_registered_before_rematch_reads_the_new_exact_offer_after_an_await() {
        let runtime = Rc::new(RefCell::new(VerifiedLobbyView::default()));
        let callback_runtime = runtime.clone();
        let callback = |offer: &SignedChallengeOffer| callback_runtime.borrow().exact_offer(offer);
        let old = offer(1, Player::Black);
        runtime.borrow_mut().observe(state(vec![entry(old.clone(), true)])).unwrap();
        let new = offer(2, Player::White);
        assert!(callback(&new).is_err());
        runtime.borrow_mut().observe(state(vec![entry(old, true), entry(new.clone(), false)])).unwrap();
        // The same callback, without reconnecting or being recreated.
        assert_eq!(callback(&new).unwrap().offer, new);
        runtime.borrow_mut().observe(state(vec![entry(new.clone(), true)])).unwrap();
        assert_eq!(callback(&new).unwrap().terminal_evidence.len(), 1);
    }

    #[test]
    fn delayed_pre_acceptance_response_cannot_return_a_rematch_to_waiting() {
        let old = offer(1, Player::Black);
        let new = offer(2, Player::White);
        let local = SigningKey::from_bytes(&[41; 32]).verifying_key().to_bytes();
        let before = state(vec![entry(old.clone(), true), entry(new.clone(), false)]);
        let after = state(vec![entry(old, true), entry(new.clone(), true)]);
        let mut view = VerifiedLobbyView::default();
        view.observe(before.clone()).unwrap();
        view.observe(after).unwrap();
        let merged = view.observe(before).unwrap();
        let games = project_accepted_games(local, &merged.challenges.offers).unwrap();
        let old_game = games.iter().find(|game| game.game_id != new.body.proposal.game_id).unwrap();
        let selected = games.iter().find(|game| game.game_id == new.body.proposal.game_id).unwrap();
        let scope = ActiveGameScopeSnapshot { epoch: 1, contract_id: old_game.contract_id.clone() };
        let intent = GameTransitionIntent { proposal: new.body.proposal, origin_scope: scope.clone(), automatic: true };
        assert_eq!(plan_game_transition(Some(&intent), &scope, Some(old_game), &games, true),
            Ok(GameTransitionDecision::Activate(selected.clone())));
        assert_eq!(selected.local_role, Player::White);
        assert_eq!(old_game.local_role, Player::Black);
    }

    #[test]
    fn invalid_response_cannot_replace_current_verified_evidence() {
        let signed = offer(1, Player::White);
        let mut view = VerifiedLobbyView::default();
        view.observe(state(vec![entry(signed.clone(), true)])).unwrap();
        let mut forged = state(vec![entry(signed.clone(), false)]);
        forged.challenges.offers[0].offer.signature.0[0] ^= 1;
        assert!(view.observe(forged).is_err());
        assert_eq!(view.exact_offer(&signed).unwrap().terminal_evidence.len(), 1);
    }

    #[test]
    fn merging_late_contradictory_evidence_still_blocks_activation() {
        let signed = offer(1, Player::White);
        let mut view = VerifiedLobbyView::default();
        view.observe(state(vec![entry(signed.clone(), true)])).unwrap();
        let refusal = backgammon_protocol::decline_challenge(&signed,
            &SigningKey::from_bytes(&[42; 32]), 700_100).unwrap();
        let declined = ChallengeOfferState::new(signed, vec![
            ChallengeTerminalEvidence::Decline(refusal)]).unwrap();
        let merged = view.observe(state(vec![declined])).unwrap();
        let local = SigningKey::from_bytes(&[41; 32]).verifying_key().to_bytes();
        assert!(project_accepted_games(local, &merged.challenges.offers).unwrap().is_empty());
    }

    #[test]
    fn canonical_retention_does_not_resurrect_pruned_games_on_old_responses() {
        let first = offer(1, Player::White);
        let mut view = VerifiedLobbyView::default();
        let old = state(vec![entry(first.clone(), true)]);
        view.observe(old.clone()).unwrap();
        let incoming = state((2..=18).map(|seed| entry(offer(seed, Player::White), true)).collect());
        let retained = view.observe(incoming).unwrap();
        assert_eq!(retained.challenges.offers.len(), 16);
        assert!(view.exact_offer(&first).is_err());
        let retained = view.observe(old).unwrap();
        assert_eq!(retained.challenges.offers.len(), 16);
        assert!(view.exact_offer(&first).is_err());
    }
}
