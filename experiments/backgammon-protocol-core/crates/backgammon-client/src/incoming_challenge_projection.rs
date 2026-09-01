use std::collections::BTreeMap;

use backgammon_lobby_core::ChallengeOfferState;
use backgammon_protocol::{
    challenge_offer_body_digest, resolve_challenge_at, ChallengeId, ChallengeResolution, GameId,
    PlayerId, SignedChallengeOffer,
};

/// One exact, live, unresolved challenge addressed to this browser identity.
///
/// The complete signed offer is retained so a later acceptance planner can
/// authenticate and sign these exact bytes without reconstructing evidence
/// from presentation fields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IncomingChallenge {
    pub signed_offer: SignedChallengeOffer,
    pub challenge_id: ChallengeId,
    pub challenger_id: PlayerId,
    pub challenger_display_name: String,
    pub game_id: GameId,
    pub match_length: u16,
    pub created_at_unix_seconds: u64,
    pub expires_at_unix_seconds: u64,
}

fn project_candidate(
    local_player_id: PlayerId,
    state: &ChallengeOfferState,
    now_unix_seconds: u64,
) -> Result<Option<([u8; 32], IncomingChallenge)>, String> {
    if resolve_challenge_at(&state.offer, &state.terminal_evidence, now_unix_seconds)?
        != ChallengeResolution::Open
    {
        return Ok(None);
    }

    let recipient_id = state.offer.body.recipient_id()?;

    if recipient_id != local_player_id {
        return Ok(None);
    }

    let challenger_id = state.offer.body.challenger_id;
    let configuration = &state.offer.body.proposal.configuration;

    let challenger_display_name = if configuration.white.id == challenger_id {
        configuration.white.display_name.clone()
    } else if configuration.black.id == challenger_id {
        configuration.black.display_name.clone()
    } else {
        return Err("Challenge signer is absent from its verified game configuration.".to_owned());
    };

    let body_digest = challenge_offer_body_digest(&state.offer.body)?;

    Ok(Some((
        body_digest,
        IncomingChallenge {
            signed_offer: state.offer.clone(),
            challenge_id: state.offer.body.challenge_id,
            challenger_id,
            challenger_display_name,
            game_id: state.offer.body.proposal.game_id,
            match_length: configuration.match_length,
            created_at_unix_seconds: state.offer.body.created_at_unix_seconds,
            expires_at_unix_seconds: state.offer.body.expires_at_unix_seconds,
        },
    )))
}

/// Projects verified lobby challenge records into deterministic live incoming
/// offers for one persistent local identity.
///
/// Invalid records are ignored independently. Multiple valid offer bodies from
/// the same challenger using the same challenge ID are authenticated
/// equivocation and suppress that ambiguous identity completely.
pub fn project_incoming_challenges(
    local_player_id: PlayerId,
    offers: &[ChallengeOfferState],
    now_unix_seconds: u64,
) -> Vec<IncomingChallenge> {
    let mut grouped: BTreeMap<(PlayerId, ChallengeId), BTreeMap<[u8; 32], IncomingChallenge>> =
        BTreeMap::new();

    for state in offers {
        let Ok(Some((body_digest, candidate))) =
            project_candidate(local_player_id, state, now_unix_seconds)
        else {
            continue;
        };

        let bodies = grouped
            .entry((candidate.challenger_id, candidate.challenge_id))
            .or_default();

        match bodies.get_mut(&body_digest) {
            Some(existing)
                if candidate.signed_offer.signature.as_bytes()
                    < existing.signed_offer.signature.as_bytes() =>
            {
                *existing = candidate;
            }

            Some(_) => {}

            None => {
                bodies.insert(body_digest, candidate);
            }
        }
    }

    let mut projected = grouped
        .into_values()
        .filter_map(|mut bodies| {
            /*
             * One challenger reusing one challenge ID for distinct signed
             * bodies is ambiguous even if every signature is valid.
             */
            if bodies.len() != 1 {
                return None;
            }

            bodies.pop_first().map(|(_, challenge)| challenge)
        })
        .collect::<Vec<_>>();

    projected.sort_by(|left, right| {
        left.created_at_unix_seconds
            .cmp(&right.created_at_unix_seconds)
            .then_with(|| {
                left.challenger_display_name
                    .cmp(&right.challenger_display_name)
            })
            .then_with(|| left.challenger_id.cmp(&right.challenger_id))
            .then_with(|| left.challenge_id.cmp(&right.challenge_id))
            .then_with(|| left.game_id.cmp(&right.game_id))
    });

    projected
}

#[cfg(test)]
mod tests {
    use super::*;

    use backgammon_protocol::{accept_challenge, ChallengeTerminalEvidence};
    use ed25519_dalek::SigningKey;

    use crate::challenge_offer_planner::{plan_outbound_challenge, OutboundChallengePlannerInput};

    const CREATED: u64 = 500_000;
    const NOW: u64 = CREATED + 1;
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
        expires_at_unix_seconds: u64,
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
            expires_at_unix_seconds,
        })
        .unwrap();

        ChallengeOfferState::new(plan.signed_offer, Vec::new()).unwrap()
    }

    #[test]
    fn live_open_offer_for_local_recipient_is_projected() {
        let challenger = key(1);
        let recipient = key(2);

        let state = offer_state(&challenger, &recipient, 11, 12, 13, EXPIRES);

        let projected = project_incoming_challenges(
            recipient.verifying_key().to_bytes(),
            &[state.clone()],
            NOW,
        );

        assert_eq!(projected.len(), 1);
        assert_eq!(projected[0].signed_offer, state.offer,);
        assert_eq!(
            projected[0].challenger_id,
            challenger.verifying_key().to_bytes(),
        );
        assert_eq!(projected[0].challenger_display_name, "Alice",);
        assert_eq!(projected[0].challenge_id, [11; 32]);
        assert_eq!(projected[0].game_id, [12; 32]);
        assert_eq!(projected[0].match_length, 3);
        assert_eq!(projected[0].expires_at_unix_seconds, EXPIRES,);
    }

    #[test]
    fn outgoing_and_other_recipient_offers_are_suppressed() {
        let challenger = key(3);
        let recipient = key(4);
        let observer = key(5);

        let state = offer_state(&challenger, &recipient, 21, 22, 23, EXPIRES);

        assert!(project_incoming_challenges(
            challenger.verifying_key().to_bytes(),
            &[state.clone()],
            NOW,
        )
        .is_empty());

        assert!(
            project_incoming_challenges(observer.verifying_key().to_bytes(), &[state], NOW,)
                .is_empty()
        );
    }

    #[test]
    fn expired_and_terminal_offers_are_suppressed() {
        let challenger = key(6);
        let recipient = key(7);

        let expired = offer_state(&challenger, &recipient, 31, 32, 33, NOW);

        let open = offer_state(&challenger, &recipient, 34, 35, 36, EXPIRES);

        let acceptance = accept_challenge(&open.offer, &recipient, NOW).unwrap();

        let accepted = ChallengeOfferState::new(
            open.offer,
            vec![ChallengeTerminalEvidence::Acceptance(acceptance)],
        )
        .unwrap();

        assert!(project_incoming_challenges(
            recipient.verifying_key().to_bytes(),
            &[expired, accepted],
            NOW,
        )
        .is_empty());
    }

    #[test]
    fn delivery_order_duplicates_and_forgery_do_not_change_projection() {
        let challenger = key(8);
        let recipient = key(9);

        let first = offer_state(&challenger, &recipient, 41, 42, 43, EXPIRES);

        let second = offer_state(&challenger, &recipient, 44, 45, 46, EXPIRES);

        let mut forged = first.clone();
        forged.offer.signature.0[0] ^= 0xff;

        let forward = project_incoming_challenges(
            recipient.verifying_key().to_bytes(),
            &[forged, second.clone(), first.clone(), first.clone()],
            NOW,
        );

        let reverse = project_incoming_challenges(
            recipient.verifying_key().to_bytes(),
            &[first, second],
            NOW,
        );

        assert_eq!(forward, reverse);
        assert_eq!(forward.len(), 2);
    }

    #[test]
    fn authenticated_challenge_id_equivocation_is_suppressed() {
        let challenger = key(10);
        let recipient = key(11);

        let first = offer_state(&challenger, &recipient, 51, 52, 53, EXPIRES);

        let different_body = offer_state(&challenger, &recipient, 51, 54, 55, EXPIRES);

        assert!(project_incoming_challenges(
            recipient.verifying_key().to_bytes(),
            &[first, different_body],
            NOW,
        )
        .is_empty());
    }
}
