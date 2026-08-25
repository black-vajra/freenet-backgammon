//! Convergent authenticated challenge evidence for the Freenet lobby.

use crate::LobbyContractState;
use backgammon_protocol::{
    challenge_offer_body_digest, ChallengeId, ChallengeOfferBodyDigest, PlayerId,
    ED25519_SIGNATURE_BYTES,
};
use backgammon_protocol::{
    resolve_challenge, verify_challenge_offer, ChallengeOfferBody, ChallengeResolution,
    ChallengeTerminalEvidence, SignedChallengeOffer,
};
use freenet_scaffold::ComposableState as FreenetComposableState;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::BTreeMap;

/// Acceptance, decline, and cancellation are the only terminal evidence kinds
/// in challenge protocol version 1.
pub const MAX_TERMINAL_EVIDENCE_PER_OFFER: usize = 3;

/// Maximum retained challenge offers from one challenger.
pub const MAX_CHALLENGE_OFFERS_PER_CHALLENGER: usize = 16;

/// Maximum retained challenge offers across the lobby.
pub const MAX_CHALLENGE_OFFERS: usize = 256;

/// Maximum number of per-challenger retention horizons in one summary.
pub const MAX_CHALLENGER_HORIZONS: usize =
    MAX_CHALLENGE_OFFERS / MAX_CHALLENGE_OFFERS_PER_CHALLENGER;

/// Immutable canonical ordering identity for one exact challenge-offer body.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct ChallengeOfferOrderKey {
    pub created_at_unix_seconds: u64,
    pub challenger_id: PlayerId,
    pub challenge_id: ChallengeId,
    pub body_digest: ChallengeOfferBodyDigest,
}

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

    /// Returns the immutable key used for canonical retention ordering.
    ///
    /// Signature bytes and terminal evidence are deliberately excluded so the
    /// key cannot change as equivalent authenticated representations merge.
    pub fn order_key(&self) -> Result<ChallengeOfferOrderKey, String> {
        self.verify()?;

        Ok(ChallengeOfferOrderKey {
            created_at_unix_seconds: self.offer.body.created_at_unix_seconds,
            challenger_id: self.offer.body.challenger_id,
            challenge_id: self.offer.body.challenge_id,
            body_digest: challenge_offer_body_digest(&self.offer.body)?,
        })
    }

    pub fn body(&self) -> &ChallengeOfferBody {
        &self.offer.body
    }
}

/// A retention boundary is open until its corresponding collection reaches
/// capacity. At capacity it publishes the oldest key still retained.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub enum ChallengeRetentionHorizon {
    Open,
    OldestRetained(ChallengeOfferOrderKey),
}

impl Default for ChallengeRetentionHorizon {
    fn default() -> Self {
        Self::Open
    }
}

/// Per-challenger retention boundary. Absence from a summary means open.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct ChallengeChallengerHorizon {
    pub challenger_id: PlayerId,
    pub oldest_retained: ChallengeOfferOrderKey,
}

/// Compact representation of one retained terminal evidence kind.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct ChallengeTerminalEvidenceSummary {
    pub kind: u8,
    pub signature: Vec<u8>,
}

/// Summary of one exact retained offer and its canonical evidence versions.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct ChallengeOfferSummary {
    pub key: ChallengeOfferOrderKey,
    pub offer_signature: Vec<u8>,
    pub terminal_evidence: Vec<ChallengeTerminalEvidenceSummary>,
}

/// Canonical receiver summary used to calculate a bounded challenge delta.
#[derive(Serialize, Deserialize, Clone, Default, PartialEq, Eq, Debug)]
pub struct ChallengeEntriesSummary {
    pub offers: Vec<ChallengeOfferSummary>,
    pub challenger_horizons: Vec<ChallengeChallengerHorizon>,
    pub global_horizon: ChallengeRetentionHorizon,
}

impl ChallengeEntriesSummary {
    /// Verify that an untrusted synchronization summary is bounded, canonical,
    /// and internally consistent with the retention rules.
    pub fn verify(&self) -> Result<(), String> {
        if self.offers.len() > MAX_CHALLENGE_OFFERS {
            return Err("Challenge summary contains too many offers.".into());
        }

        if self.challenger_horizons.len() > MAX_CHALLENGER_HORIZONS {
            return Err("Challenge summary contains too many challenger horizons.".into());
        }

        let mut previous_key: Option<&ChallengeOfferOrderKey> = None;
        let mut challenger_windows: BTreeMap<PlayerId, (usize, ChallengeOfferOrderKey)> =
            BTreeMap::new();

        for offer in &self.offers {
            if previous_key
                .as_ref()
                .is_some_and(|previous| *previous >= &offer.key)
            {
                return Err("Challenge summary offers are not in strict canonical order.".into());
            }

            if offer.offer_signature.len() != ED25519_SIGNATURE_BYTES {
                return Err(format!(
                    "Challenge offer summary signature length is invalid: expected \
                     {ED25519_SIGNATURE_BYTES} bytes, got {}.",
                    offer.offer_signature.len()
                ));
            }

            if offer.terminal_evidence.len() > MAX_TERMINAL_EVIDENCE_PER_OFFER {
                return Err("Challenge offer summary contains too much terminal evidence.".into());
            }

            let mut previous_kind: Option<u8> = None;

            for evidence in &offer.terminal_evidence {
                if usize::from(evidence.kind) >= MAX_TERMINAL_EVIDENCE_PER_OFFER {
                    return Err("Challenge summary contains an unknown evidence kind.".into());
                }

                if previous_kind.is_some_and(|previous| previous >= evidence.kind) {
                    return Err(
                        "Challenge summary evidence is not in strict canonical order.".into(),
                    );
                }

                if evidence.signature.len() != ED25519_SIGNATURE_BYTES {
                    return Err(format!(
                        "Challenge evidence summary signature length is invalid: expected \
                         {ED25519_SIGNATURE_BYTES} bytes, got {}.",
                        evidence.signature.len()
                    ));
                }

                previous_kind = Some(evidence.kind);
            }

            let challenger = challenger_windows
                .entry(offer.key.challenger_id)
                .or_insert_with(|| (0, offer.key.clone()));
            challenger.0 += 1;

            if challenger.0 > MAX_CHALLENGE_OFFERS_PER_CHALLENGER {
                return Err(
                    "Challenge summary contains too many offers from one challenger.".into(),
                );
            }

            previous_key = Some(&offer.key);
        }

        let expected_challenger_horizons = challenger_windows
            .into_iter()
            .filter_map(|(challenger_id, (count, oldest_retained))| {
                (count == MAX_CHALLENGE_OFFERS_PER_CHALLENGER).then_some(
                    ChallengeChallengerHorizon {
                        challenger_id,
                        oldest_retained,
                    },
                )
            })
            .collect::<Vec<_>>();

        if self.challenger_horizons != expected_challenger_horizons {
            return Err(
                "Challenge summary challenger horizons do not match retained offers.".into(),
            );
        }

        let expected_global_horizon = if self.offers.len() < MAX_CHALLENGE_OFFERS {
            ChallengeRetentionHorizon::Open
        } else {
            ChallengeRetentionHorizon::OldestRetained(self.offers[0].key.clone())
        };

        if self.global_horizon != expected_global_horizon {
            return Err("Challenge summary global horizon does not match retained offers.".into());
        }

        Ok(())
    }
}

/// Full authenticated offer states selected for one synchronization delta.
#[derive(Serialize, Deserialize, Clone, Default, PartialEq, Eq, Debug)]
pub struct ChallengeEntriesDelta {
    pub offers: Vec<ChallengeOfferState>,
}

/// Canonically ordered, deterministically bounded challenge state.
#[derive(Serialize, Deserialize, Clone, Default, PartialEq, Eq, Debug)]
pub struct ChallengeEntries {
    pub offers: Vec<ChallengeOfferState>,
}

fn canonical_challenge_offers(
    offers: Vec<ChallengeOfferState>,
) -> Result<Vec<ChallengeOfferState>, String> {
    let mut keyed = Vec::with_capacity(offers.len());

    for offer in offers {
        let key = offer.order_key()?;
        keyed.push((key, offer));
    }

    keyed.sort_by(|left, right| left.0.cmp(&right.0));

    let mut merged: Vec<(ChallengeOfferOrderKey, ChallengeOfferState)> =
        Vec::with_capacity(keyed.len());

    for (key, offer) in keyed {
        if let Some((last_key, last_offer)) = merged.last_mut() {
            if *last_key == key {
                last_offer.merge_from(&offer)?;
                continue;
            }
        }

        merged.push((key, offer));
    }

    let mut by_challenger: BTreeMap<PlayerId, Vec<(ChallengeOfferOrderKey, ChallengeOfferState)>> =
        BTreeMap::new();

    for entry in merged {
        by_challenger
            .entry(entry.0.challenger_id)
            .or_default()
            .push(entry);
    }

    let mut retained = Vec::new();

    for (_, mut challenger_offers) in by_challenger {
        if challenger_offers.len() > MAX_CHALLENGE_OFFERS_PER_CHALLENGER {
            let excess = challenger_offers.len() - MAX_CHALLENGE_OFFERS_PER_CHALLENGER;
            challenger_offers.drain(..excess);
        }

        retained.extend(challenger_offers);
    }

    retained.sort_by(|left, right| left.0.cmp(&right.0));

    if retained.len() > MAX_CHALLENGE_OFFERS {
        let excess = retained.len() - MAX_CHALLENGE_OFFERS;
        retained.drain(..excess);
    }

    Ok(retained.into_iter().map(|(_, offer)| offer).collect())
}

fn sender_has_new_information(
    offer: &ChallengeOfferState,
    receiver: &ChallengeOfferSummary,
) -> bool {
    if offer.offer.signature.as_bytes() < receiver.offer_signature.as_slice() {
        return true;
    }

    offer.terminal_evidence.iter().any(|evidence| {
        let kind = terminal_kind(evidence);
        let signature = terminal_signature(evidence);
        let receiver_signature = receiver
            .terminal_evidence
            .iter()
            .filter(|summary| summary.kind == kind)
            .map(|summary| summary.signature.as_slice())
            .min();

        match receiver_signature {
            Some(known) => signature < known,
            None => true,
        }
    })
}

fn key_is_above_receiver_horizons(
    key: &ChallengeOfferOrderKey,
    receiver: &ChallengeEntriesSummary,
) -> bool {
    let challenger_horizon = receiver
        .challenger_horizons
        .iter()
        .find(|horizon| horizon.challenger_id == key.challenger_id);
    let above_challenger_horizon = match challenger_horizon {
        Some(horizon) => key > &horizon.oldest_retained,
        None => true,
    };

    let above_global_horizon = match &receiver.global_horizon {
        ChallengeRetentionHorizon::Open => true,
        ChallengeRetentionHorizon::OldestRetained(oldest) => key > oldest,
    };

    above_challenger_horizon && above_global_horizon
}

impl ChallengeEntries {
    pub fn new(offers: Vec<ChallengeOfferState>) -> Result<Self, String> {
        let state = Self {
            offers: canonical_challenge_offers(offers)?,
        };

        state.verify_state()?;
        Ok(state)
    }

    pub fn verify_state(&self) -> Result<(), String> {
        if self.offers.len() > MAX_CHALLENGE_OFFERS {
            return Err("Lobby retains too many challenge offers.".into());
        }

        let mut previous_key: Option<ChallengeOfferOrderKey> = None;
        let mut challenger_counts: BTreeMap<PlayerId, usize> = BTreeMap::new();

        for offer in &self.offers {
            let key = offer.order_key()?;

            if previous_key
                .as_ref()
                .is_some_and(|previous| previous >= &key)
            {
                return Err("Challenge offers are not in strict canonical order.".into());
            }

            let count = challenger_counts.entry(key.challenger_id).or_default();
            *count += 1;

            if *count > MAX_CHALLENGE_OFFERS_PER_CHALLENGER {
                return Err("Lobby retains too many offers from one challenger.".into());
            }

            previous_key = Some(key);
        }

        Ok(())
    }

    /// Associative/commutative/idempotent merge followed by deterministic
    /// per-challenger and global top-key retention.
    pub fn merge_from(&mut self, incoming: &Self) -> Result<(), String> {
        self.verify_state()?;
        incoming.verify_state()?;

        let mut combined = self.offers.clone();
        combined.extend(incoming.offers.iter().cloned());
        self.offers = canonical_challenge_offers(combined)?;

        self.verify_state()
    }

    pub fn challenger_horizons(&self) -> Result<Vec<ChallengeChallengerHorizon>, String> {
        self.verify_state()?;

        let mut grouped: BTreeMap<PlayerId, Vec<ChallengeOfferOrderKey>> = BTreeMap::new();

        for offer in &self.offers {
            let key = offer.order_key()?;
            grouped.entry(key.challenger_id).or_default().push(key);
        }

        Ok(grouped
            .into_iter()
            .filter_map(|(challenger_id, keys)| {
                (keys.len() == MAX_CHALLENGE_OFFERS_PER_CHALLENGER).then(|| {
                    ChallengeChallengerHorizon {
                        challenger_id,
                        oldest_retained: keys[0].clone(),
                    }
                })
            })
            .collect())
    }

    pub fn global_horizon(&self) -> Result<ChallengeRetentionHorizon, String> {
        self.verify_state()?;

        if self.offers.len() < MAX_CHALLENGE_OFFERS {
            return Ok(ChallengeRetentionHorizon::Open);
        }

        Ok(ChallengeRetentionHorizon::OldestRetained(
            self.offers[0].order_key()?,
        ))
    }

    pub fn retention_summary(&self) -> Result<ChallengeEntriesSummary, String> {
        self.verify_state()?;

        let offers = self
            .offers
            .iter()
            .map(|offer| {
                Ok(ChallengeOfferSummary {
                    key: offer.order_key()?,
                    offer_signature: offer.offer.signature.as_bytes().to_vec(),
                    terminal_evidence: offer
                        .terminal_evidence
                        .iter()
                        .map(|evidence| ChallengeTerminalEvidenceSummary {
                            kind: terminal_kind(evidence),
                            signature: terminal_signature(evidence).to_vec(),
                        })
                        .collect(),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;

        Ok(ChallengeEntriesSummary {
            offers,
            challenger_horizons: self.challenger_horizons()?,
            global_horizon: self.global_horizon()?,
        })
    }

    pub fn delta_from_summary(
        &self,
        receiver: &ChallengeEntriesSummary,
    ) -> Result<Option<ChallengeEntriesDelta>, String> {
        self.verify_state()?;
        receiver.verify()?;

        let offers = self
            .offers
            .iter()
            .filter_map(|offer| {
                let key = offer.order_key().ok()?;
                let retained = receiver.offers.iter().find(|known| known.key == key);

                match retained {
                    Some(known) if sender_has_new_information(offer, known) => Some(offer.clone()),
                    Some(_) => None,
                    None if key_is_above_receiver_horizons(&key, receiver) => Some(offer.clone()),
                    None => None,
                }
            })
            .collect::<Vec<_>>();

        Ok((!offers.is_empty()).then_some(ChallengeEntriesDelta { offers }))
    }

    pub fn apply_challenge_delta(&mut self, delta: &ChallengeEntriesDelta) -> Result<(), String> {
        let mut combined = self.offers.clone();
        combined.extend(delta.offers.iter().cloned());
        self.offers = canonical_challenge_offers(combined)?;

        self.verify_state()
    }
}

impl FreenetComposableState for ChallengeEntries {
    type ParentState = LobbyContractState;
    // Optional so a legacy presence-only parent summary means that the
    // receiver has no challenge knowledge.
    type Summary = Option<ChallengeEntriesSummary>;
    type Delta = ChallengeEntriesDelta;
    type Parameters = ();

    fn verify(
        &self,
        _parent: &Self::ParentState,
        _parameters: &Self::Parameters,
    ) -> Result<(), String> {
        ChallengeEntries::verify_state(self)
    }

    fn summarize(
        &self,
        _parent: &Self::ParentState,
        _parameters: &Self::Parameters,
    ) -> Self::Summary {
        self.retention_summary().ok()
    }

    fn delta(
        &self,
        _parent: &Self::ParentState,
        _parameters: &Self::Parameters,
        old: &Self::Summary,
    ) -> Option<Self::Delta> {
        let empty_summary = ChallengeEntriesSummary::default();
        let receiver = match old {
            Some(summary) if summary.verify().is_ok() => summary,
            _ => &empty_summary,
        };

        self.delta_from_summary(receiver).ok().flatten()
    }

    fn apply_delta(
        &mut self,
        _parent: &Self::ParentState,
        _parameters: &Self::Parameters,
        delta: &Option<Self::Delta>,
    ) -> Result<(), String> {
        if let Some(incoming) = delta {
            self.apply_challenge_delta(incoming)?;
        }

        ChallengeEntries::verify_state(self)
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

    fn fixture_at(
        challenger_seed: u8,
        challenge_seed: u8,
        created_at_unix_seconds: u64,
    ) -> (SignedChallengeOffer, SigningKey, SigningKey) {
        let white_key = SigningKey::from_bytes(&[challenger_seed; 32]);
        let black_key = SigningKey::from_bytes(&[challenger_seed.wrapping_add(100); 32]);

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
            created_at_unix_seconds,
            created_at_unix_seconds + 600,
            proposal,
        );

        let offer = sign_challenge_offer(body, &white_key).unwrap();
        (offer, white_key, black_key)
    }

    fn offer_state_at(
        challenger_seed: u8,
        challenge_seed: u8,
        created_at_unix_seconds: u64,
    ) -> ChallengeOfferState {
        let (offer, _, _) = fixture_at(challenger_seed, challenge_seed, created_at_unix_seconds);
        ChallengeOfferState::new(offer, Vec::new()).unwrap()
    }

    fn challenger_window(
        challenger_seed: u8,
        first_challenge_seed: u8,
        count: u8,
    ) -> ChallengeEntries {
        ChallengeEntries::new(
            (first_challenge_seed..first_challenge_seed + count)
                .map(|challenge_seed| {
                    offer_state_at(
                        challenger_seed,
                        challenge_seed,
                        40_000 + u64::from(challenge_seed),
                    )
                })
                .collect(),
        )
        .unwrap()
    }

    fn global_offer(index: usize) -> ChallengeOfferState {
        let challenger_seed = 1 + u8::try_from(index / MAX_CHALLENGE_OFFERS_PER_CHALLENGER)
            .expect("test index fits in u8");
        let challenge_seed = u8::try_from(index % MAX_CHALLENGE_OFFERS_PER_CHALLENGER).unwrap();

        offer_state_at(
            challenger_seed,
            challenge_seed,
            100_000 + u64::try_from(index).unwrap(),
        )
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
    fn order_key_round_trips_through_cbor() {
        let (offer, _, _) = fixture(53);
        let state = ChallengeOfferState::new(offer, Vec::new()).unwrap();
        let expected = state.order_key().unwrap();
        let mut encoded = Vec::new();

        ciborium::ser::into_writer(&expected, &mut encoded).unwrap();

        let decoded: ChallengeOfferOrderKey =
            ciborium::de::from_reader(encoded.as_slice()).unwrap();

        assert_eq!(decoded, expected);
    }

    #[test]
    fn order_key_is_stable_as_terminal_evidence_accumulates() {
        let (offer, white_key, black_key) = fixture(54);
        let (acceptance, _, _) = terminal_evidence(&offer, &white_key, &black_key);

        let open = ChallengeOfferState::new(offer.clone(), Vec::new()).unwrap();
        let accepted = ChallengeOfferState::new(offer, vec![acceptance]).unwrap();

        assert_eq!(open.order_key().unwrap(), accepted.order_key().unwrap());
    }

    #[test]
    fn same_challenge_id_with_different_bodies_has_distinct_order_keys() {
        let (first_offer, white_key, _) = fixture(55);
        let mut second_body = first_offer.body.clone();

        if second_body.proposal.configuration.match_length == 1 {
            second_body.proposal.configuration.match_length = 2;
        } else {
            second_body.proposal.configuration.match_length = 1;
        }

        let second_offer = sign_challenge_offer(second_body, &white_key).unwrap();
        let first = ChallengeOfferState::new(first_offer, Vec::new()).unwrap();
        let second = ChallengeOfferState::new(second_offer, Vec::new()).unwrap();
        let first_key = first.order_key().unwrap();
        let second_key = second.order_key().unwrap();

        assert_eq!(
            first_key.created_at_unix_seconds,
            second_key.created_at_unix_seconds
        );
        assert_eq!(first_key.challenger_id, second_key.challenger_id);
        assert_eq!(first_key.challenge_id, second_key.challenge_id);
        assert_ne!(first_key.body_digest, second_key.body_digest);
        assert_ne!(first_key, second_key);
    }

    #[test]
    fn order_key_prioritizes_creation_time_before_tie_breakers() {
        let earlier = ChallengeOfferOrderKey {
            created_at_unix_seconds: 100,
            challenger_id: [u8::MAX; 32],
            challenge_id: [u8::MAX; 32],
            body_digest: [u8::MAX; 32],
        };

        let later = ChallengeOfferOrderKey {
            created_at_unix_seconds: 101,
            challenger_id: [0; 32],
            challenge_id: [0; 32],
            body_digest: [0; 32],
        };

        assert!(earlier < later);
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

    #[test]
    fn per_challenger_retention_keeps_newest_sixteen_and_publishes_horizon() {
        let state = challenger_window(91, 0, 17);
        let expected_oldest = offer_state_at(91, 1, 40_001).order_key().unwrap();

        assert_eq!(state.offers.len(), MAX_CHALLENGE_OFFERS_PER_CHALLENGER);
        assert_eq!(state.offers[0].order_key().unwrap(), expected_oldest);
        assert_eq!(
            state.challenger_horizons().unwrap(),
            vec![ChallengeChallengerHorizon {
                challenger_id: expected_oldest.challenger_id,
                oldest_retained: expected_oldest,
            }]
        );
        assert_eq!(
            state.global_horizon().unwrap(),
            ChallengeRetentionHorizon::Open
        );
    }

    #[test]
    fn global_retention_keeps_newest_256_and_suppresses_pruned_history() {
        let all = (0..=MAX_CHALLENGE_OFFERS)
            .map(global_offer)
            .collect::<Vec<_>>();
        let pruned = all[0].clone();
        let mut state = ChallengeEntries::new(all).unwrap();
        let expected_oldest = global_offer(1).order_key().unwrap();
        let pruned_challenger_id = pruned.body().challenger_id;

        assert_eq!(state.offers.len(), MAX_CHALLENGE_OFFERS);
        assert_eq!(state.offers[0].order_key().unwrap(), expected_oldest);
        assert_eq!(
            state.global_horizon().unwrap(),
            ChallengeRetentionHorizon::OldestRetained(expected_oldest.clone())
        );
        assert!(state
            .challenger_horizons()
            .unwrap()
            .iter()
            .all(|horizon| horizon.challenger_id != pruned_challenger_id));

        let sender = ChallengeEntries::new(vec![pruned]).unwrap();
        let summary = state.retention_summary().unwrap();

        assert_eq!(sender.delta_from_summary(&summary).unwrap(), None);

        let (oldest_offer, _, black_key) = fixture_at(1, 1, 100_001);
        let acceptance = ChallengeTerminalEvidence::Acceptance(
            accept_challenge(&oldest_offer, &black_key, 100_002).unwrap(),
        );
        let updated_oldest = ChallengeOfferState::new(oldest_offer, vec![acceptance]).unwrap();
        assert_eq!(updated_oldest.order_key().unwrap(), expected_oldest);

        let evidence_sender = ChallengeEntries::new(vec![updated_oldest]).unwrap();
        let evidence_delta = evidence_sender
            .delta_from_summary(&summary)
            .unwrap()
            .expect("new evidence at the global horizon must be delivered");

        state.apply_challenge_delta(&evidence_delta).unwrap();
        assert_eq!(
            evidence_sender
                .delta_from_summary(&state.retention_summary().unwrap())
                .unwrap(),
            None
        );
    }

    #[test]
    fn per_challenger_trim_runs_before_global_trim() {
        let quiet = (0..240).map(global_offer).collect::<Vec<_>>();
        let quiet_oldest_key = quiet[0].order_key().unwrap();
        let busy = (0..17)
            .map(|index| offer_state_at(99, index, 200_000 + u64::from(index)))
            .collect::<Vec<_>>();

        let state = ChallengeEntries::new(quiet.into_iter().chain(busy).collect()).unwrap();
        let busy_id = SigningKey::from_bytes(&[99; 32]).verifying_key().to_bytes();

        assert_eq!(state.offers.len(), MAX_CHALLENGE_OFFERS);
        assert!(state
            .offers
            .iter()
            .any(|offer| offer.order_key().unwrap() == quiet_oldest_key));
        assert_eq!(
            state
                .offers
                .iter()
                .filter(|offer| offer.body().challenger_id == busy_id)
                .count(),
            MAX_CHALLENGE_OFFERS_PER_CHALLENGER
        );
    }

    #[test]
    fn bounded_merge_is_associative_commutative_and_idempotent() {
        let first = challenger_window(92, 0, 13);
        let second = challenger_window(92, 8, 13);
        let third = challenger_window(92, 18, 8);

        let mut left = first.clone();
        left.merge_from(&second).unwrap();
        left.merge_from(&third).unwrap();

        let mut right_group = third.clone();
        right_group.merge_from(&second).unwrap();
        let mut right = first.clone();
        right.merge_from(&right_group).unwrap();

        let mut reversed = third;
        reversed.merge_from(&second).unwrap();
        reversed.merge_from(&first).unwrap();

        let mut idempotent = left.clone();
        idempotent.merge_from(&left).unwrap();

        assert_eq!(left, right);
        assert_eq!(left, reversed);
        assert_eq!(left, idempotent);
        assert_eq!(left.offers.len(), MAX_CHALLENGE_OFFERS_PER_CHALLENGER);
        assert_eq!(left.offers[0].body().challenge_id, [10; 32]);
    }

    #[test]
    fn receiver_horizon_stops_old_window_resend_and_accepts_newer_window_once() {
        let mut receiver = challenger_window(93, 10, 16);
        let older_sender = challenger_window(93, 0, 16);
        let newer_sender = challenger_window(93, 20, 16);

        let receiver_summary = receiver.retention_summary().unwrap();
        assert_eq!(
            older_sender.delta_from_summary(&receiver_summary).unwrap(),
            None
        );

        let delta = newer_sender
            .delta_from_summary(&receiver_summary)
            .unwrap()
            .expect("newer offers must be delivered");
        assert_eq!(delta.offers.len(), 10);

        receiver.apply_challenge_delta(&delta).unwrap();
        assert_eq!(receiver, newer_sender);
        assert_eq!(
            newer_sender
                .delta_from_summary(&receiver.retention_summary().unwrap())
                .unwrap(),
            None
        );
    }

    #[test]
    fn evidence_update_at_oldest_retained_key_bypasses_horizon() {
        let mut receiver = challenger_window(94, 0, 16);
        let (offer, _, black_key) = fixture_at(94, 0, 40_000);
        let acceptance = ChallengeTerminalEvidence::Acceptance(
            accept_challenge(&offer, &black_key, 40_001).unwrap(),
        );
        let updated_offer = ChallengeOfferState::new(offer, vec![acceptance]).unwrap();
        let updated_key = updated_offer.order_key().unwrap();
        let sender = ChallengeEntries::new(vec![updated_offer]).unwrap();
        let receiver_summary = receiver.retention_summary().unwrap();

        assert_eq!(
            receiver_summary.challenger_horizons[0].oldest_retained,
            updated_key
        );

        let delta = sender
            .delta_from_summary(&receiver_summary)
            .unwrap()
            .expect("new evidence for a retained horizon key must be delivered");

        receiver.apply_challenge_delta(&delta).unwrap();

        let retained = receiver
            .offers
            .iter()
            .find(|state| state.order_key().unwrap() == updated_key)
            .unwrap();
        assert_eq!(retained.evidence_mask(), 0b001);
        assert_eq!(
            sender
                .delta_from_summary(&receiver.retention_summary().unwrap())
                .unwrap(),
            None
        );
    }

    #[test]
    fn challenge_summaries_reject_noncanonical_and_oversized_input() {
        let state = challenger_window(96, 10, 4);
        let summary = state.retention_summary().unwrap();
        summary.verify().unwrap();

        let mut out_of_order = summary.clone();
        out_of_order.offers.swap(0, 1);
        assert!(out_of_order.verify().is_err());

        let mut invalid_signature = summary.clone();
        invalid_signature.offers[0].offer_signature.clear();
        assert!(invalid_signature.verify().is_err());

        let mut duplicate_evidence_kind = summary.clone();
        let signature = duplicate_evidence_kind.offers[0].offer_signature.clone();
        duplicate_evidence_kind.offers[0].terminal_evidence = vec![
            ChallengeTerminalEvidenceSummary {
                kind: 0,
                signature: signature.clone(),
            },
            ChallengeTerminalEvidenceSummary { kind: 0, signature },
        ];
        assert!(duplicate_evidence_kind.verify().is_err());

        let mut inconsistent_global_horizon = summary.clone();
        inconsistent_global_horizon.global_horizon = ChallengeRetentionHorizon::OldestRetained(
            inconsistent_global_horizon.offers[0].key.clone(),
        );
        assert!(inconsistent_global_horizon.verify().is_err());

        let mut missing_challenger_horizon =
            challenger_window(98, 0, 16).retention_summary().unwrap();
        assert_eq!(missing_challenger_horizon.challenger_horizons.len(), 1);
        let retained_horizon = missing_challenger_horizon.challenger_horizons[0].clone();
        missing_challenger_horizon.challenger_horizons.clear();
        assert!(missing_challenger_horizon.verify().is_err());

        let mut oversized_horizons = ChallengeEntriesSummary::default();
        oversized_horizons.challenger_horizons =
            vec![retained_horizon; MAX_CHALLENGER_HORIZONS + 1];
        assert!(oversized_horizons.verify().is_err());

        let mut oversized = ChallengeEntriesSummary::default();
        oversized.offers = vec![summary.offers[0].clone(); MAX_CHALLENGE_OFFERS + 1];
        assert!(oversized.verify().is_err());
    }

    #[test]
    fn composable_challenge_summary_is_optional_and_invalid_input_falls_back_to_empty() {
        let sender = challenger_window(97, 20, 4);
        let parent = LobbyContractState::default();
        let expected_summary = sender.retention_summary().unwrap();

        assert_eq!(
            <ChallengeEntries as FreenetComposableState>::summarize(&sender, &parent, &()),
            Some(expected_summary.clone())
        );

        let legacy_delta =
            <ChallengeEntries as FreenetComposableState>::delta(&sender, &parent, &(), &None)
                .expect("legacy summary must receive the bounded challenge state");
        assert_eq!(legacy_delta.offers, sender.offers);

        let mut invalid_summary = expected_summary;
        invalid_summary.offers[0].offer_signature.clear();
        assert!(sender.delta_from_summary(&invalid_summary).is_err());

        let fallback_delta = <ChallengeEntries as FreenetComposableState>::delta(
            &sender,
            &parent,
            &(),
            &Some(invalid_summary),
        )
        .expect("invalid summary must fall back to an empty receiver summary");

        assert_eq!(fallback_delta.offers, sender.offers);
    }

    #[test]
    fn summaries_and_deltas_round_trip_through_cbor() {
        let sender = challenger_window(95, 4, 4);
        let receiver = challenger_window(95, 4, 2);
        let summary = receiver.retention_summary().unwrap();
        let delta = sender
            .delta_from_summary(&summary)
            .unwrap()
            .expect("receiver is missing two offers");

        let mut summary_bytes = Vec::new();
        ciborium::ser::into_writer(&summary, &mut summary_bytes).unwrap();
        let decoded_summary: ChallengeEntriesSummary =
            ciborium::de::from_reader(summary_bytes.as_slice()).unwrap();
        decoded_summary.verify().unwrap();

        let mut delta_bytes = Vec::new();
        ciborium::ser::into_writer(&delta, &mut delta_bytes).unwrap();
        let decoded_delta: ChallengeEntriesDelta =
            ciborium::de::from_reader(delta_bytes.as_slice()).unwrap();

        assert_eq!(decoded_summary, summary);
        assert_eq!(decoded_delta, delta);
        assert_eq!(decoded_delta.offers.len(), 2);
    }
}
