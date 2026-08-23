//! Convergent authenticated challenge evidence for the Freenet lobby.

use backgammon_protocol::{
    resolve_challenge, verify_challenge_offer, ChallengeOfferBody, ChallengeResolution,
    ChallengeTerminalEvidence, SignedChallengeOffer,
};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;

/// Acceptance, decline, and cancellation are the only terminal evidence kinds
/// in challenge protocol version 1.
pub const MAX_TERMINAL_EVIDENCE_PER_OFFER: usize = 3;

/// One exact authenticated offer and all canonical terminal evidence known for it.
///
/// The signed offer body—not delivery order—is the identity of this record.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct ChallengeOfferState {
    pub offer: SignedChallengeOffer,
    pub terminal_evidence: Vec<ChallengeTerminalEvidence>,
}

fn terminal_kind(evidence: &ChallengeTerminalEvidence) -> u8 {
    match evidence {
        ChallengeTerminalEvidence::Acceptance(_) => 0,
        ChallengeTerminalEvidence::Decline(_) => 1,
        ChallengeTerminalEvidence::Cancellation(_) => 2,
    }
}

fn terminal_signature(evidence: &ChallengeTerminalEvidence) -> &[u8] {
    match evidence {
        ChallengeTerminalEvidence::Acceptance(value) => value.signature.as_bytes(),
        ChallengeTerminalEvidence::Decline(value) => value.signature.as_bytes(),
        ChallengeTerminalEvidence::Cancellation(value) => value.signature.as_bytes(),
    }
}

fn terminal_evidence_cmp(
    left: &ChallengeTerminalEvidence,
    right: &ChallengeTerminalEvidence,
) -> Ordering {
    terminal_kind(left)
        .cmp(&terminal_kind(right))
        .then_with(|| terminal_signature(left).cmp(terminal_signature(right)))
}

/// Produce one deterministic representative for each terminal evidence kind.
///
/// Verification occurs before this function is used. For one exact offer, a
/// valid evidence kind has one fixed signer and signing body. If multiple valid
/// signature representations ever exist, the smallest signature bytes are
/// retained deterministically.
fn canonical_terminal_evidence(
    mut evidence: Vec<ChallengeTerminalEvidence>,
) -> Vec<ChallengeTerminalEvidence> {
    evidence.sort_by(terminal_evidence_cmp);
    evidence.dedup_by(|left, right| terminal_kind(left) == terminal_kind(right));
    evidence
}

impl ChallengeOfferState {
    pub fn new(
        offer: SignedChallengeOffer,
        terminal_evidence: Vec<ChallengeTerminalEvidence>,
    ) -> Result<Self, String> {
        verify_challenge_offer(&offer)?;
        resolve_challenge(&offer, &terminal_evidence)?;

        let state = Self {
            offer,
            terminal_evidence: canonical_terminal_evidence(terminal_evidence),
        };

        state.verify()?;
        Ok(state)
    }

    pub fn verify(&self) -> Result<(), String> {
        verify_challenge_offer(&self.offer)?;

        if self.terminal_evidence.len() > MAX_TERMINAL_EVIDENCE_PER_OFFER {
            return Err("Challenge offer retains too much terminal evidence.".into());
        }

        resolve_challenge(&self.offer, &self.terminal_evidence)?;

        if canonical_terminal_evidence(self.terminal_evidence.clone()) != self.terminal_evidence {
            return Err("Challenge terminal evidence is not in canonical form.".into());
        }

        Ok(())
    }

    /// Associative/commutative/idempotent merge for one exact signed offer body.
    pub fn merge_from(&mut self, incoming: &Self) -> Result<(), String> {
        self.verify()?;
        incoming.verify()?;

        if self.offer.body != incoming.offer.body {
            return Err("Cannot merge terminal evidence for different challenge offers.".into());
        }

        if incoming.offer.signature.as_bytes() < self.offer.signature.as_bytes() {
            self.offer = incoming.offer.clone();
        }

        let mut combined = self.terminal_evidence.clone();
        combined.extend(incoming.terminal_evidence.iter().cloned());
        self.terminal_evidence = canonical_terminal_evidence(combined);

        self.verify()
    }

    pub fn resolution(&self) -> Result<ChallengeResolution, String> {
        self.verify()?;
        resolve_challenge(&self.offer, &self.terminal_evidence)
    }

    pub fn evidence_mask(&self) -> u8 {
        self.terminal_evidence.iter().fold(0_u8, |mask, evidence| {
            mask | match evidence {
                ChallengeTerminalEvidence::Acceptance(_) => 0b001,
                ChallengeTerminalEvidence::Decline(_) => 0b010,
                ChallengeTerminalEvidence::Cancellation(_) => 0b100,
            }
        })
    }

    pub fn body(&self) -> &ChallengeOfferBody {
        &self.offer.body
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use backgammon_protocol::{
        accept_challenge, cancel_challenge, decline_challenge, sign_challenge_offer,
        ChallengeOfferBody, GameConfiguration, GenesisProposal, PlayerDescriptor,
    };
    use ed25519_dalek::SigningKey;

    const CREATED: u64 = 20_000;
    const EXPIRES: u64 = 20_600;

    fn fixture(challenge_seed: u8) -> (SignedChallengeOffer, SigningKey, SigningKey) {
        let white_key = SigningKey::from_bytes(&[81; 32]);
        let black_key = SigningKey::from_bytes(&[82; 32]);

        let proposal = GenesisProposal::new(
            [challenge_seed.wrapping_add(1); 32],
            [challenge_seed.wrapping_add(2); 32],
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
            [challenge_seed; 32],
            white_key.verifying_key().to_bytes(),
            CREATED,
            EXPIRES,
            proposal,
        );

        let offer = sign_challenge_offer(body, &white_key).unwrap();
        (offer, white_key, black_key)
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
    fn empty_offer_state_is_open_and_round_trips_through_cbor() {
        let (offer, _, _) = fixture(41);
        let state = ChallengeOfferState::new(offer, Vec::new()).unwrap();

        assert_eq!(state.resolution().unwrap(), ChallengeResolution::Open);
        assert_eq!(state.evidence_mask(), 0);

        let mut encoded = Vec::new();
        ciborium::ser::into_writer(&state, &mut encoded).unwrap();

        let decoded: ChallengeOfferState = ciborium::de::from_reader(encoded.as_slice()).unwrap();

        assert_eq!(decoded, state);
        assert_eq!(decoded.verify(), Ok(()));
    }

    #[test]
    fn constructor_canonicalizes_duplicates_and_terminal_order() {
        let (offer, white_key, black_key) = fixture(42);
        let (acceptance, decline, cancellation) = terminal_evidence(&offer, &white_key, &black_key);

        let state = ChallengeOfferState::new(
            offer,
            vec![cancellation, acceptance.clone(), decline, acceptance],
        )
        .unwrap();

        assert_eq!(state.terminal_evidence.len(), 3);
        assert_eq!(state.evidence_mask(), 0b111);
        assert!(matches!(
            state.terminal_evidence.first(),
            Some(ChallengeTerminalEvidence::Acceptance(_))
        ));
        assert!(matches!(
            state.terminal_evidence.get(1),
            Some(ChallengeTerminalEvidence::Decline(_))
        ));
        assert!(matches!(
            state.terminal_evidence.get(2),
            Some(ChallengeTerminalEvidence::Cancellation(_))
        ));
        assert_eq!(state.resolution().unwrap(), ChallengeResolution::Conflict);
    }

    #[test]
    fn duplicate_terminal_delivery_is_idempotent() {
        let (offer, white_key, black_key) = fixture(43);
        let (acceptance, _, _) = terminal_evidence(&offer, &white_key, &black_key);

        let original = ChallengeOfferState::new(offer, vec![acceptance]).unwrap();

        let mut merged = original.clone();
        merged.merge_from(&original).unwrap();

        assert_eq!(merged, original);
        assert_eq!(merged.terminal_evidence.len(), 1);
    }

    #[test]
    fn opposite_terminal_merge_orders_converge_to_cancellation() {
        let (offer, white_key, black_key) = fixture(44);
        let (acceptance, _, cancellation) = terminal_evidence(&offer, &white_key, &black_key);

        let accepted = ChallengeOfferState::new(offer.clone(), vec![acceptance]).unwrap();
        let cancelled = ChallengeOfferState::new(offer, vec![cancellation]).unwrap();

        let mut left = accepted.clone();
        left.merge_from(&cancelled).unwrap();

        let mut right = cancelled;
        right.merge_from(&accepted).unwrap();

        assert_eq!(left, right);
        assert_eq!(left.resolution().unwrap(), ChallengeResolution::Cancelled);
    }

    #[test]
    fn three_terminal_merges_are_associative() {
        let (offer, white_key, black_key) = fixture(45);
        let (acceptance, decline, cancellation) = terminal_evidence(&offer, &white_key, &black_key);

        let accepted = ChallengeOfferState::new(offer.clone(), vec![acceptance]).unwrap();
        let declined = ChallengeOfferState::new(offer.clone(), vec![decline]).unwrap();
        let cancelled = ChallengeOfferState::new(offer, vec![cancellation]).unwrap();

        let mut left = accepted.clone();
        left.merge_from(&declined).unwrap();
        left.merge_from(&cancelled).unwrap();

        let mut right_group = declined;
        right_group.merge_from(&cancelled).unwrap();

        let mut right = accepted;
        right.merge_from(&right_group).unwrap();

        assert_eq!(left, right);
        assert_eq!(left.evidence_mask(), 0b111);
        assert_eq!(left.resolution().unwrap(), ChallengeResolution::Conflict);
    }

    #[test]
    fn acceptance_and_decline_conflict_is_preserved() {
        let (offer, white_key, black_key) = fixture(46);
        let (acceptance, decline, _) = terminal_evidence(&offer, &white_key, &black_key);

        let mut state = ChallengeOfferState::new(offer.clone(), vec![acceptance]).unwrap();
        let incoming = ChallengeOfferState::new(offer, vec![decline]).unwrap();

        state.merge_from(&incoming).unwrap();

        assert_eq!(state.evidence_mask(), 0b011);
        assert_eq!(state.resolution().unwrap(), ChallengeResolution::Conflict);
    }

    #[test]
    fn evidence_for_a_different_offer_is_rejected() {
        let (first_offer, _, _) = fixture(47);
        let (second_offer, second_white, second_black) = fixture(48);

        let (foreign_acceptance, _, _) =
            terminal_evidence(&second_offer, &second_white, &second_black);

        assert!(ChallengeOfferState::new(first_offer, vec![foreign_acceptance]).is_err());
    }

    #[test]
    fn different_offer_bodies_cannot_merge() {
        let (first_offer, _, _) = fixture(49);
        let (second_offer, _, _) = fixture(50);

        let mut first = ChallengeOfferState::new(first_offer, Vec::new()).unwrap();
        let second = ChallengeOfferState::new(second_offer, Vec::new()).unwrap();

        assert!(first.merge_from(&second).is_err());
    }

    #[test]
    fn noncanonical_terminal_order_is_rejected() {
        let (offer, white_key, black_key) = fixture(51);
        let (acceptance, decline, _) = terminal_evidence(&offer, &white_key, &black_key);

        let noncanonical = ChallengeOfferState {
            offer,
            terminal_evidence: vec![decline, acceptance],
        };

        assert!(noncanonical.verify().is_err());
    }

    #[test]
    fn oversized_deserialized_terminal_state_is_rejected() {
        let (offer, white_key, black_key) = fixture(52);
        let (acceptance, _, _) = terminal_evidence(&offer, &white_key, &black_key);

        let oversized = ChallengeOfferState {
            offer,
            terminal_evidence: vec![
                acceptance.clone(),
                acceptance.clone(),
                acceptance.clone(),
                acceptance,
            ],
        };

        assert!(oversized.verify().is_err());
    }
}
