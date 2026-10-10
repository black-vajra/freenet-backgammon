use backgammon_lobby_core::ChallengeOfferState;
use backgammon_protocol::{decline_challenge, resolve_challenge_at, ChallengeResolution, ChallengeTerminalEvidence, SignedChallengeOffer};
use ed25519_dalek::SigningKey;
use crate::lobby_codec::build_encoded_challenge_state_update;

/// Sign only the exact current, verified, live offer addressed to this identity.
pub fn plan_incoming_challenge_decline(
    current: &ChallengeOfferState,
    key: &SigningKey,
    now: u64,
) -> Result<Vec<u8>, String> {
    current.verify()?;
    if resolve_challenge_at(&current.offer, &current.terminal_evidence, now)? != ChallengeResolution::Open {
        return Err("This challenge is no longer open; refresh the lobby.".to_owned());
    }
    let decline = decline_challenge(&current.offer, key, now)?;
    let mut evidence = current.terminal_evidence.clone();
    evidence.push(ChallengeTerminalEvidence::Decline(decline));
    build_encoded_challenge_state_update(ChallengeOfferState::new(current.offer.clone(), evidence)?)
}

/// A sent request alone is not confirmation: require the exact verified offer
/// and an authenticated decline, with no conflicting acceptance.
pub fn incoming_decline_confirmed(offer: &SignedChallengeOffer, entries: &[ChallengeOfferState], now: u64) -> bool {
    entries.iter().any(|entry| entry.offer == *offer && entry.verify().is_ok()
        && matches!(resolve_challenge_at(&entry.offer, &entry.terminal_evidence, now), Ok(ChallengeResolution::Declined)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::challenge_offer_planner::{plan_outbound_challenge, OutboundChallengePlannerInput};
    use crate::lobby_codec::decode_verified_lobby_state;
    fn fixture() -> (ChallengeOfferState, SigningKey, SigningKey) {
        let sender = SigningKey::from_bytes(&[101;32]);
        let receiver = SigningKey::from_bytes(&[102;32]);
        let plan = plan_outbound_challenge(OutboundChallengePlannerInput {
            signing_key: &sender, challenger_display_name: "Dada",
            recipient_id: receiver.verifying_key().to_bytes(), recipient_display_name: "Mama",
            challenger_role: backgammon_core::Player::Black, match_length: 1,
            challenge_id: [11;32], game_id: [12;32], genesis_action_id: [13;32],
            created_at_unix_seconds: 200_000, expires_at_unix_seconds: 200_600,
        }).unwrap();
        (ChallengeOfferState::new(plan.signed_offer, vec![]).unwrap(), sender, receiver)
    }
    #[test]
    fn recipient_decline_preserves_offer_and_fixed_colours() {
        let (entry, _, key) = fixture();
        let update = plan_incoming_challenge_decline(&entry, &key, 200_010).unwrap();
        let state = decode_verified_lobby_state(&update).unwrap();
        assert_eq!(state.challenges.offers.len(),1);
        assert_eq!(state.challenges.offers[0].offer, entry.offer);
        assert_eq!(entry.offer.body.proposal.configuration.white.id,key.verifying_key().to_bytes());
        assert!(incoming_decline_confirmed(&entry.offer,&state.challenges.offers,200_010));
        assert!(!incoming_decline_confirmed(&entry.offer,&[entry.clone()],200_010));
    }
    #[test]
    fn wrong_identity_and_expired_offer_are_rejected() {
        let (entry, sender, receiver) = fixture();
        assert!(plan_incoming_challenge_decline(&entry,&sender,200_010).is_err());
        assert!(plan_incoming_challenge_decline(&entry,&receiver,200_600).is_err());
    }
    #[test]
    fn accepted_offer_cannot_be_declined() {
        let (mut entry, _, receiver) = fixture();
        let accept = backgammon_protocol::accept_challenge(&entry.offer,&receiver,200_010).unwrap();
        entry.terminal_evidence.push(ChallengeTerminalEvidence::Acceptance(accept));
        assert!(plan_incoming_challenge_decline(&entry,&receiver,200_020).is_err());
        assert!(!incoming_decline_confirmed(&entry.offer,&[entry.clone()],200_020));
        // A simultaneous accept/decline is a conflict, never a confirmed decline.
        let decline = decline_challenge(&entry.offer,&receiver,200_020).unwrap();
        entry.terminal_evidence.push(ChallengeTerminalEvidence::Decline(decline));
        assert!(!incoming_decline_confirmed(&entry.offer,&[entry.clone()],200_020));
    }
}
