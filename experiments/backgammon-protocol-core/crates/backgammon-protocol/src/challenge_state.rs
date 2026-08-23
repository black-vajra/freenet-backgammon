use crate::challenge::{
    accepted_genesis_proposal, verify_challenge_acceptance, verify_challenge_cancellation,
    verify_challenge_decline, verify_challenge_offer, ChallengeAcceptance, ChallengeCancellation,
    ChallengeDecline, SignedChallengeOffer,
};
use crate::genesis_handshake::GenesisProposal;
use serde::{Deserialize, Serialize};

/// Authenticated terminal evidence received for one exact challenge.
///
/// Transport delivery order is deliberately absent from this representation.
/// Freenet peers may observe these messages in different orders.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChallengeTerminalEvidence {
    Acceptance(ChallengeAcceptance),
    Decline(ChallengeDecline),
    Cancellation(ChallengeCancellation),
}

/// Deterministic local interpretation of all authenticated evidence known for
/// one challenge.
///
/// `Expired` is a lobby-liveness result only. Signed terminal evidence remains
/// cryptographically meaningful after ordinary wall-clock expiry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChallengeResolution {
    Open,
    Expired,
    Accepted { proposal: GenesisProposal },
    Declined,
    Cancelled,
    Conflict,
}

/// Resolves authenticated challenge evidence without consulting a wall clock.
///
/// The result depends only on the exact signed offer and the unordered set of
/// valid terminal evidence. Duplicate delivery is therefore idempotent.
///
/// Conflict policy:
///
/// - acceptance + cancellation => cancellation wins
/// - decline + cancellation    => cancellation wins
/// - acceptance + decline     => fail closed as Conflict
/// - all three                => fail closed as Conflict
///
/// Cancellation cannot erase an already-created authenticated game history;
/// this resolver is for the pre-genesis challenge negotiation layer.
pub fn resolve_challenge(
    offer: &SignedChallengeOffer,
    evidence: &[ChallengeTerminalEvidence],
) -> Result<ChallengeResolution, String> {
    verify_challenge_offer(offer)?;

    let mut acceptance: Option<&ChallengeAcceptance> = None;
    let mut saw_decline = false;
    let mut saw_cancellation = false;

    for item in evidence {
        match item {
            ChallengeTerminalEvidence::Acceptance(value) => {
                verify_challenge_acceptance(offer, value)?;

                if acceptance.is_none() {
                    acceptance = Some(value);
                }
            }

            ChallengeTerminalEvidence::Decline(value) => {
                verify_challenge_decline(offer, value)?;
                saw_decline = true;
            }

            ChallengeTerminalEvidence::Cancellation(value) => {
                verify_challenge_cancellation(offer, value)?;
                saw_cancellation = true;
            }
        }
    }

    let saw_acceptance = acceptance.is_some();

    /*
     * A recipient that authenticates both Acceptance and Decline has produced
     * contradictory terminal evidence. Fail closed regardless of whether a
     * cancellation is also present.
     */
    if saw_acceptance && saw_decline {
        return Ok(ChallengeResolution::Conflict);
    }

    /*
     * Before authenticated genesis exists, the challenger may withdraw the
     * offer. Cancellation therefore wins over a crossed Acceptance or Decline
     * message. Arrival order is irrelevant.
     */
    if saw_cancellation {
        return Ok(ChallengeResolution::Cancelled);
    }

    if saw_decline {
        return Ok(ChallengeResolution::Declined);
    }

    if let Some(acceptance) = acceptance {
        return Ok(ChallengeResolution::Accepted {
            proposal: accepted_genesis_proposal(offer, acceptance)?,
        });
    }

    Ok(ChallengeResolution::Open)
}

/// Applies local lobby-expiry presentation to the deterministic cryptographic
/// resolution.
///
/// Expiry only changes an otherwise-open challenge. It never overrides
/// authenticated terminal evidence that has already been observed.
pub fn resolve_challenge_at(
    offer: &SignedChallengeOffer,
    evidence: &[ChallengeTerminalEvidence],
    now_unix_seconds: u64,
) -> Result<ChallengeResolution, String> {
    let resolution = resolve_challenge(offer, evidence)?;

    if resolution != ChallengeResolution::Open {
        return Ok(resolution);
    }

    if now_unix_seconds >= offer.body.expires_at_unix_seconds {
        Ok(ChallengeResolution::Expired)
    } else {
        Ok(ChallengeResolution::Open)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::challenge::{
        accept_challenge, cancel_challenge, decline_challenge, sign_challenge_offer,
        ChallengeOfferBody,
    };
    use crate::{GameConfiguration, PlayerDescriptor};
    use ed25519_dalek::SigningKey;

    const CREATED: u64 = 10_000;
    const EXPIRES: u64 = 10_600;

    fn fixture() -> (
        SignedChallengeOffer,
        SigningKey,
        SigningKey,
        GenesisProposal,
    ) {
        let white_key = SigningKey::from_bytes(&[71; 32]);
        let black_key = SigningKey::from_bytes(&[72; 32]);

        let proposal = GenesisProposal::new(
            [31; 32],
            [32; 32],
            GameConfiguration {
                white: PlayerDescriptor {
                    id: white_key.verifying_key().to_bytes(),
                    display_name: "Alice".to_owned(),
                },
                black: PlayerDescriptor {
                    id: black_key.verifying_key().to_bytes(),
                    display_name: "Bob".to_owned(),
                },
                match_length: 5,
            },
        );

        let body = ChallengeOfferBody::new(
            [33; 32],
            white_key.verifying_key().to_bytes(),
            CREATED,
            EXPIRES,
            proposal.clone(),
        );

        let offer = sign_challenge_offer(body, &white_key).unwrap();

        (offer, white_key, black_key, proposal)
    }

    fn terminal_evidence(
        offer: &SignedChallengeOffer,
        white_key: &SigningKey,
        black_key: &SigningKey,
    ) -> (
        ChallengeTerminalEvidence,
        ChallengeTerminalEvidence,
        ChallengeTerminalEvidence,
    ) {
        let acceptance = accept_challenge(offer, black_key, CREATED + 1).unwrap();

        let decline = decline_challenge(offer, black_key, CREATED + 1).unwrap();

        let cancellation = cancel_challenge(offer, white_key, CREATED + 1).unwrap();

        (
            ChallengeTerminalEvidence::Acceptance(acceptance),
            ChallengeTerminalEvidence::Decline(decline),
            ChallengeTerminalEvidence::Cancellation(cancellation),
        )
    }

    #[test]
    fn terminal_evidence_round_trips_through_cbor() {
        let (offer, white_key, black_key, _) = fixture();
        let (acceptance, decline, cancellation) = terminal_evidence(&offer, &white_key, &black_key);

        for expected in [acceptance, decline, cancellation] {
            let mut encoded = Vec::new();
            ciborium::ser::into_writer(&expected, &mut encoded).unwrap();

            let decoded: ChallengeTerminalEvidence =
                ciborium::de::from_reader(encoded.as_slice()).unwrap();

            assert_eq!(decoded, expected);
        }
    }

    #[test]
    fn challenge_without_terminal_evidence_is_open() {
        let (offer, _, _, _) = fixture();

        assert_eq!(
            resolve_challenge(&offer, &[]).unwrap(),
            ChallengeResolution::Open
        );

        assert_eq!(
            resolve_challenge_at(&offer, &[], CREATED + 1).unwrap(),
            ChallengeResolution::Open
        );
    }

    #[test]
    fn only_open_challenge_becomes_expired() {
        let (offer, _, _, _) = fixture();

        assert_eq!(
            resolve_challenge_at(&offer, &[], EXPIRES).unwrap(),
            ChallengeResolution::Expired
        );
    }

    #[test]
    fn acceptance_resolves_to_exact_genesis_proposal() {
        let (offer, _, black_key, expected) = fixture();

        let acceptance = accept_challenge(&offer, &black_key, CREATED + 1).unwrap();

        assert_eq!(
            resolve_challenge(&offer, &[ChallengeTerminalEvidence::Acceptance(acceptance)])
                .unwrap(),
            ChallengeResolution::Accepted { proposal: expected }
        );
    }

    #[test]
    fn decline_resolves_to_declined() {
        let (offer, _, black_key, _) = fixture();

        let decline = decline_challenge(&offer, &black_key, CREATED + 1).unwrap();

        assert_eq!(
            resolve_challenge(&offer, &[ChallengeTerminalEvidence::Decline(decline)]).unwrap(),
            ChallengeResolution::Declined
        );
    }

    #[test]
    fn cancellation_resolves_to_cancelled() {
        let (offer, white_key, _, _) = fixture();

        let cancellation = cancel_challenge(&offer, &white_key, CREATED + 1).unwrap();

        assert_eq!(
            resolve_challenge(
                &offer,
                &[ChallengeTerminalEvidence::Cancellation(cancellation)]
            )
            .unwrap(),
            ChallengeResolution::Cancelled
        );
    }

    #[test]
    fn duplicate_terminal_delivery_is_idempotent() {
        let (offer, _, black_key, expected) = fixture();

        let acceptance = accept_challenge(&offer, &black_key, CREATED + 1).unwrap();

        let evidence = ChallengeTerminalEvidence::Acceptance(acceptance);

        assert_eq!(
            resolve_challenge(&offer, &[evidence.clone(), evidence]).unwrap(),
            ChallengeResolution::Accepted { proposal: expected }
        );
    }

    #[test]
    fn cancellation_beats_crossed_acceptance_in_either_order() {
        let (offer, white_key, black_key, _) = fixture();
        let (acceptance, _, cancellation) = terminal_evidence(&offer, &white_key, &black_key);

        for evidence in [
            vec![acceptance.clone(), cancellation.clone()],
            vec![cancellation.clone(), acceptance.clone()],
        ] {
            assert_eq!(
                resolve_challenge(&offer, &evidence).unwrap(),
                ChallengeResolution::Cancelled
            );
        }
    }

    #[test]
    fn cancellation_beats_crossed_decline_in_either_order() {
        let (offer, white_key, black_key, _) = fixture();
        let (_, decline, cancellation) = terminal_evidence(&offer, &white_key, &black_key);

        for evidence in [
            vec![decline.clone(), cancellation.clone()],
            vec![cancellation.clone(), decline.clone()],
        ] {
            assert_eq!(
                resolve_challenge(&offer, &evidence).unwrap(),
                ChallengeResolution::Cancelled
            );
        }
    }

    #[test]
    fn acceptance_and_decline_conflict_in_either_order() {
        let (offer, white_key, black_key, _) = fixture();
        let (acceptance, decline, _) = terminal_evidence(&offer, &white_key, &black_key);

        for evidence in [
            vec![acceptance.clone(), decline.clone()],
            vec![decline.clone(), acceptance.clone()],
        ] {
            assert_eq!(
                resolve_challenge(&offer, &evidence).unwrap(),
                ChallengeResolution::Conflict
            );
        }
    }

    #[test]
    fn all_terminal_messages_conflict_in_every_order() {
        let (offer, white_key, black_key, _) = fixture();
        let (acceptance, decline, cancellation) = terminal_evidence(&offer, &white_key, &black_key);

        let permutations = [
            vec![acceptance.clone(), decline.clone(), cancellation.clone()],
            vec![acceptance.clone(), cancellation.clone(), decline.clone()],
            vec![decline.clone(), acceptance.clone(), cancellation.clone()],
            vec![decline.clone(), cancellation.clone(), acceptance.clone()],
            vec![cancellation.clone(), acceptance.clone(), decline.clone()],
            vec![cancellation.clone(), decline.clone(), acceptance.clone()],
        ];

        for evidence in permutations {
            assert_eq!(
                resolve_challenge(&offer, &evidence).unwrap(),
                ChallengeResolution::Conflict
            );
        }
    }

    #[test]
    fn terminal_resolution_survives_wall_clock_expiry() {
        let (offer, white_key, black_key, expected) = fixture();
        let (acceptance, decline, cancellation) = terminal_evidence(&offer, &white_key, &black_key);

        assert_eq!(
            resolve_challenge_at(&offer, &[acceptance], EXPIRES).unwrap(),
            ChallengeResolution::Accepted { proposal: expected }
        );

        assert_eq!(
            resolve_challenge_at(&offer, &[decline], EXPIRES).unwrap(),
            ChallengeResolution::Declined
        );

        assert_eq!(
            resolve_challenge_at(&offer, &[cancellation], EXPIRES).unwrap(),
            ChallengeResolution::Cancelled
        );
    }

    #[test]
    fn malformed_terminal_evidence_fails_closed() {
        let (offer, _, black_key, _) = fixture();

        let mut acceptance = accept_challenge(&offer, &black_key, CREATED + 1).unwrap();

        acceptance.signature.0.pop();

        assert!(
            resolve_challenge(&offer, &[ChallengeTerminalEvidence::Acceptance(acceptance)])
                .is_err()
        );
    }

    #[test]
    fn evidence_for_different_offer_is_rejected() {
        let (offer, white_key, black_key, _) = fixture();

        let acceptance = accept_challenge(&offer, &black_key, CREATED + 1).unwrap();

        let mut different_body = offer.body.clone();
        different_body.challenge_id = [34; 32];

        let different_offer = sign_challenge_offer(different_body, &white_key).unwrap();

        assert!(resolve_challenge(
            &different_offer,
            &[ChallengeTerminalEvidence::Acceptance(acceptance)]
        )
        .is_err());
    }
}
