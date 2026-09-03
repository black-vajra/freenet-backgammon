use backgammon_core::Player;
use backgammon_lobby_core::ChallengeOfferState;
use backgammon_protocol::{
    challenge_offer_body_digest, ChallengeResolution, GameId, GenesisProposal, PlayerId,
};
use freenet_stdlib::prelude::ContractKey;

use crate::game_contract_publication::calculate_expected_game_contract;
use crate::local_identity_store::role_for_player_id;

/// One authenticated accepted challenge involving the persistent local
/// identity, projected into the exact inputs needed by a later game runtime.
///
/// The complete Freenet key and canonical contract ID are recalculated from
/// the game ID authenticated by both challenge participants. No browser
/// transport state, local role selection, or wall clock contributes to this
/// projection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AcceptedGame {
    pub game_id: GameId,
    pub contract_key: ContractKey,
    pub contract_id: String,
    pub accepted_proposal: GenesisProposal,
    pub peer_id: PlayerId,
    pub local_role: Player,
}

/// Projects verified authoritative challenge records into deterministic
/// accepted games involving one persistent local identity.
///
/// Invalid, unresolved, declined, cancelled, conflicting, and unrelated
/// records are ignored independently. Accepted evidence is intentionally not
/// filtered by offer expiry: once authenticated, ordinary wall-clock passage
/// cannot roll the negotiation back.
///
/// Contract calculation errors are returned rather than silently hiding an
/// otherwise valid accepted game.
pub fn project_accepted_games(
    local_player_id: PlayerId,
    offers: &[ChallengeOfferState],
) -> Result<Vec<AcceptedGame>, String> {
    let mut projected = Vec::new();

    for state in offers {
        let resolution = match state.resolution() {
            Ok(resolution) => resolution,
            Err(_) => continue,
        };

        let ChallengeResolution::Accepted { proposal } = resolution else {
            continue;
        };

        let Some(local_role) = role_for_player_id(&proposal.configuration, &local_player_id) else {
            continue;
        };

        let peer_id = match local_role {
            Player::White => proposal.configuration.black.id,
            Player::Black => proposal.configuration.white.id,
        };

        let offer_digest = challenge_offer_body_digest(&state.offer.body)?;
        let expected = calculate_expected_game_contract(proposal.game_id)?;

        projected.push((
            offer_digest,
            AcceptedGame {
                game_id: proposal.game_id,
                contract_key: expected.full_key,
                contract_id: expected.contract_id,
                accepted_proposal: proposal,
                peer_id,
                local_role,
            },
        ));
    }

    projected.sort_by(|left, right| left.0.cmp(&right.0));
    projected.dedup_by(|left, right| left.0 == right.0 && left.1 == right.1);

    Ok(projected
        .into_iter()
        .map(|(_, accepted_game)| accepted_game)
        .collect())
}

/// Resolves volatile browser selection exclusively against the current
/// authoritative accepted-game projection.
///
/// The requested game ID is only UI intent. It never becomes a usable runtime
/// candidate unless exactly one currently verified accepted game carries that
/// ID. Missing and ambiguous selections therefore fail closed.
pub fn resolve_accepted_game_selection(
    selected_game_id: Option<GameId>,
    accepted_games: &[AcceptedGame],
) -> Result<Option<&AcceptedGame>, String> {
    let Some(selected_game_id) = selected_game_id else {
        return Ok(None);
    };

    let mut matches = accepted_games
        .iter()
        .filter(|accepted| accepted.game_id == selected_game_id);

    let Some(selected) = matches.next() else {
        return Err("Selected game is no longer present in the authoritative \
             accepted-game set."
            .to_owned());
    };

    if matches.next().is_some() {
        return Err("Selected game ID is ambiguous in the authoritative \
             accepted-game set."
            .to_owned());
    }

    Ok(Some(selected))
}

#[cfg(test)]
mod tests {
    use super::*;

    use backgammon_protocol::{accept_challenge, ChallengeTerminalEvidence};
    use ed25519_dalek::SigningKey;

    use crate::challenge_offer_planner::{plan_outbound_challenge, OutboundChallengePlannerInput};

    const CREATED: u64 = 700_000;
    const ACCEPTED_AT: u64 = CREATED + 1;
    const EXPIRES: u64 = CREATED + 600;

    fn key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn offer_state(
        challenger: &SigningKey,
        recipient: &SigningKey,
        challenge_id: u8,
        game_id: u8,
        genesis_action_id: u8,
    ) -> ChallengeOfferState {
        let plan = plan_outbound_challenge(OutboundChallengePlannerInput {
            signing_key: challenger,
            challenger_display_name: "Alice",
            recipient_id: recipient.verifying_key().to_bytes(),
            recipient_display_name: "Bob",
            match_length: 3,
            challenge_id: [challenge_id; 32],
            game_id: [game_id; 32],
            genesis_action_id: [genesis_action_id; 32],
            created_at_unix_seconds: CREATED,
            expires_at_unix_seconds: EXPIRES,
        })
        .unwrap();

        ChallengeOfferState::new(plan.signed_offer, Vec::new()).unwrap()
    }

    fn accepted_state(
        challenger: &SigningKey,
        recipient: &SigningKey,
        challenge_id: u8,
        game_id: u8,
        genesis_action_id: u8,
    ) -> ChallengeOfferState {
        let open = offer_state(
            challenger,
            recipient,
            challenge_id,
            game_id,
            genesis_action_id,
        );

        let acceptance = accept_challenge(&open.offer, recipient, ACCEPTED_AT).unwrap();

        ChallengeOfferState::new(
            open.offer,
            vec![ChallengeTerminalEvidence::Acceptance(acceptance)],
        )
        .unwrap()
    }

    #[test]
    fn accepted_game_projects_exact_contract_proposal_peer_and_black_role() {
        let challenger = key(1);
        let recipient = key(2);
        let state = accepted_state(&challenger, &recipient, 11, 12, 13);

        let projected =
            project_accepted_games(recipient.verifying_key().to_bytes(), &[state.clone()]).unwrap();

        let expected_contract = calculate_expected_game_contract([12; 32]).unwrap();

        assert_eq!(projected.len(), 1);
        assert_eq!(projected[0].game_id, [12; 32]);
        assert_eq!(projected[0].contract_key, expected_contract.full_key);
        assert_eq!(projected[0].contract_id, expected_contract.contract_id);
        assert_eq!(projected[0].accepted_proposal, state.offer.body.proposal);
        assert_eq!(projected[0].peer_id, challenger.verifying_key().to_bytes());
        assert_eq!(projected[0].local_role, Player::Black);
    }

    #[test]
    fn challenger_projects_the_same_acceptance_with_white_role() {
        let challenger = key(3);
        let recipient = key(4);
        let state = accepted_state(&challenger, &recipient, 21, 22, 23);

        let projected =
            project_accepted_games(challenger.verifying_key().to_bytes(), &[state.clone()])
                .unwrap();

        assert_eq!(projected.len(), 1);
        assert_eq!(projected[0].accepted_proposal, state.offer.body.proposal);
        assert_eq!(projected[0].peer_id, recipient.verifying_key().to_bytes());
        assert_eq!(projected[0].local_role, Player::White);
    }

    #[test]
    fn open_unrelated_and_malformed_records_are_ignored() {
        let challenger = key(5);
        let recipient = key(6);
        let outsider = key(7);

        let open = offer_state(&challenger, &recipient, 31, 32, 33);
        let accepted = accepted_state(&challenger, &recipient, 34, 35, 36);

        let mut malformed = accepted.clone();

        let ChallengeTerminalEvidence::Acceptance(acceptance) = &mut malformed.terminal_evidence[0]
        else {
            panic!("fixture must contain acceptance evidence");
        };

        acceptance.signature.0[0] ^= 0xff;

        assert!(
            project_accepted_games(outsider.verifying_key().to_bytes(), &[accepted],)
                .unwrap()
                .is_empty()
        );

        assert!(
            project_accepted_games(recipient.verifying_key().to_bytes(), &[open, malformed],)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn projection_is_independent_of_delivery_order_and_duplicates() {
        let challenger = key(8);
        let recipient = key(9);

        let first = accepted_state(&challenger, &recipient, 41, 42, 43);
        let second = accepted_state(&challenger, &recipient, 44, 45, 46);

        let forward = project_accepted_games(
            recipient.verifying_key().to_bytes(),
            &[first.clone(), second.clone(), first.clone()],
        )
        .unwrap();

        let reverse =
            project_accepted_games(recipient.verifying_key().to_bytes(), &[second, first]).unwrap();

        assert_eq!(forward, reverse);
        assert_eq!(forward.len(), 2);

        let mut game_ids = forward
            .iter()
            .map(|accepted| accepted.game_id)
            .collect::<Vec<_>>();

        game_ids.sort();

        assert_eq!(game_ids, vec![[42; 32], [45; 32]]);
    }

    #[test]
    fn absent_selection_resolves_to_no_runtime_candidate() {
        assert_eq!(resolve_accepted_game_selection(None, &[]).unwrap(), None);
    }

    #[test]
    fn exact_current_candidate_is_selected() {
        let challenger = key(10);
        let recipient = key(11);
        let state = accepted_state(&challenger, &recipient, 51, 52, 53);

        let accepted =
            project_accepted_games(recipient.verifying_key().to_bytes(), &[state]).unwrap();

        let selected = resolve_accepted_game_selection(Some([52; 32]), &accepted)
            .unwrap()
            .unwrap();

        assert_eq!(selected, &accepted[0]);
    }

    #[test]
    fn selection_missing_from_current_candidates_fails_closed() {
        let challenger = key(12);
        let recipient = key(13);
        let state = accepted_state(&challenger, &recipient, 61, 62, 63);

        let accepted =
            project_accepted_games(recipient.verifying_key().to_bytes(), &[state]).unwrap();

        assert!(resolve_accepted_game_selection(Some([64; 32]), &accepted).is_err());
    }

    #[test]
    fn duplicate_game_id_selection_fails_as_ambiguous() {
        let challenger = key(14);
        let recipient = key(15);
        let state = accepted_state(&challenger, &recipient, 71, 72, 73);

        let accepted =
            project_accepted_games(recipient.verifying_key().to_bytes(), &[state]).unwrap();

        let duplicated = vec![accepted[0].clone(), accepted[0].clone()];

        assert!(resolve_accepted_game_selection(Some([72; 32]), &duplicated).is_err());
    }
}
