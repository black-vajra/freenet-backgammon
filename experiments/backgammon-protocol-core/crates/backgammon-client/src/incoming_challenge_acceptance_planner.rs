use backgammon_lobby_core::ChallengeOfferState;
use backgammon_protocol::{
    accept_challenge, accepted_genesis_proposal_at, resolve_challenge_at,
    verify_challenge_acceptance_at, ChallengeAcceptance, ChallengeResolution,
    ChallengeTerminalEvidence, GenesisProposal, PlayerId, SignedChallengeOffer,
};
use ed25519_dalek::SigningKey;
use freenet_stdlib::prelude::ContractKey;

use crate::game_contract_publication::calculate_expected_game_contract;
use crate::lobby_codec::{build_encoded_challenge_state_update, decode_verified_lobby_state};

/// Unsigned proof target prepared from complete authoritative challenge
/// evidence.
///
/// Constructing this value never creates an acceptance signature. The browser
/// may use `contract_id` to request the game contract, but must retain the full
/// expected key and canonical empty state for exact response verification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IncomingChallengeContractProbe {
    pub signed_offer: SignedChallengeOffer,
    pub local_player_id: PlayerId,
    pub expected_contract_key: ContractKey,
    pub contract_id: String,
    pub expected_empty_state: Vec<u8>,
}

/// Signed acceptance and exact minimal lobby update produced only after the
/// independently reconstructed contract proof succeeds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IncomingChallengeAcceptancePlan {
    pub signed_offer: SignedChallengeOffer,
    pub acceptance: ChallengeAcceptance,
    pub accepted_proposal: GenesisProposal,
    pub verified_contract_key: ContractKey,
    pub contract_id: String,
    pub encoded_lobby_state_update: Vec<u8>,
}

/// Validates an exact authoritative open challenge and prepares the unsigned
/// per-game contract proof target.
pub fn prepare_incoming_challenge_contract_probe(
    challenge: &ChallengeOfferState,
    local_player_id: PlayerId,
    now_unix_seconds: u64,
) -> Result<IncomingChallengeContractProbe, String> {
    challenge
        .verify()
        .map_err(|error| format!("Incoming challenge state failed verification: {error}"))?;

    let resolution = resolve_challenge_at(
        &challenge.offer,
        &challenge.terminal_evidence,
        now_unix_seconds,
    )
    .map_err(|error| format!("Could not resolve incoming challenge: {error}"))?;

    if resolution != ChallengeResolution::Open {
        return Err(format!("Incoming challenge is not open: {resolution:?}."));
    }

    let recipient_id = challenge
        .offer
        .body
        .recipient_id()
        .map_err(|error| format!("Could not resolve challenge recipient: {error}"))?;

    if recipient_id != local_player_id {
        return Err("Incoming challenge is not addressed to the local identity.".to_owned());
    }

    let game_id = challenge.offer.body.proposal.game_id;
    let expected = calculate_expected_game_contract(game_id)?;

    if expected.game_id != game_id {
        return Err("Calculated contract proof changed the signed game ID.".to_owned());
    }

    Ok(IncomingChallengeContractProbe {
        signed_offer: challenge.offer.clone(),
        local_player_id,
        expected_contract_key: expected.full_key,
        contract_id: expected.contract_id,
        expected_empty_state: expected.empty_state_bytes,
    })
}

fn verify_probe_is_current(
    probe: &IncomingChallengeContractProbe,
    current_challenge: &ChallengeOfferState,
    now_unix_seconds: u64,
) -> Result<(), String> {
    let refreshed = prepare_incoming_challenge_contract_probe(
        current_challenge,
        probe.local_player_id,
        now_unix_seconds,
    )?;

    if refreshed != *probe {
        return Err("Authoritative challenge evidence changed while the game \
             contract was being verified."
            .to_owned());
    }

    Ok(())
}

/// Creates the recipient signature only after all of the following hold:
///
/// - the latest authoritative challenge remains the exact same open offer;
/// - the retrieved full key equals the locally reconstructed full key;
/// - the retrieved state exactly equals the canonical empty ledger;
/// - the persistent signing identity is the challenged recipient.
///
/// No partial plan or acceptance signature is returned on failure.
pub fn finalize_incoming_challenge_acceptance(
    probe: &IncomingChallengeContractProbe,
    current_challenge: &ChallengeOfferState,
    retrieved_contract_key: &ContractKey,
    retrieved_state: &[u8],
    signing_key: &SigningKey,
    now_unix_seconds: u64,
) -> Result<IncomingChallengeAcceptancePlan, String> {
    verify_probe_is_current(probe, current_challenge, now_unix_seconds)?;

    let local_player_id = signing_key.verifying_key().to_bytes();

    if local_player_id != probe.local_player_id {
        return Err("Persistent signing identity does not match the probed \
             challenge recipient."
            .to_owned());
    }

    if retrieved_contract_key != &probe.expected_contract_key {
        return Err(
            "Retrieved game contract full key does not match the pinned \
             package and signed game ID."
                .to_owned(),
        );
    }

    if retrieved_contract_key.id().encode() != probe.contract_id {
        return Err("Retrieved game contract ID is not the canonical expected ID.".to_owned());
    }

    if retrieved_state != probe.expected_empty_state {
        return Err("Retrieved game contract is not the exact canonical empty \
             ledger required before acceptance."
            .to_owned());
    }

    /*
     * This is intentionally the first signature-producing operation in the
     * finalization path. Every authoritative contract proof above has already
     * succeeded.
     */
    let acceptance = accept_challenge(&probe.signed_offer, signing_key, now_unix_seconds)
        .map_err(|error| format!("Could not sign challenge acceptance: {error}"))?;

    verify_challenge_acceptance_at(&probe.signed_offer, &acceptance, now_unix_seconds)
        .map_err(|error| format!("New challenge acceptance failed live verification: {error}"))?;

    let accepted_proposal =
        accepted_genesis_proposal_at(&probe.signed_offer, &acceptance, now_unix_seconds).map_err(
            |error| format!("Could not recover the authenticated genesis proposal: {error}"),
        )?;

    if accepted_proposal.game_id != probe.signed_offer.body.proposal.game_id {
        return Err("Accepted genesis proposal changed the signed game ID.".to_owned());
    }

    let accepted_state = ChallengeOfferState::new(
        probe.signed_offer.clone(),
        vec![ChallengeTerminalEvidence::Acceptance(acceptance.clone())],
    )
    .map_err(|error| format!("Could not construct accepted challenge state: {error}"))?;

    let encoded_lobby_state_update =
        build_encoded_challenge_state_update(accepted_state.clone())
            .map_err(|error| format!("Could not encode accepted challenge update: {error}"))?;

    let decoded_update = decode_verified_lobby_state(&encoded_lobby_state_update)
        .map_err(|error| format!("Could not verify encoded acceptance update: {error}"))?;

    if decoded_update.challenges.offers != vec![accepted_state]
        || decoded_update.lobby != backgammon_lobby_core::LobbyEntries::default()
    {
        return Err("Encoded acceptance update was not the exact minimal \
             challenge-only state."
            .to_owned());
    }

    Ok(IncomingChallengeAcceptancePlan {
        signed_offer: probe.signed_offer.clone(),
        acceptance,
        accepted_proposal,
        verified_contract_key: *retrieved_contract_key,
        contract_id: probe.contract_id.clone(),
        encoded_lobby_state_update,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::challenge_offer_planner::{plan_outbound_challenge, OutboundChallengePlannerInput};

    const CREATED: u64 = 700_000;
    const NOW: u64 = CREATED + 1;
    const EXPIRES: u64 = CREATED + 600;

    fn key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn open_offer(challenger: &SigningKey, recipient: &SigningKey) -> ChallengeOfferState {
        let plan = plan_outbound_challenge(OutboundChallengePlannerInput {
            signing_key: challenger,
            challenger_display_name: "Alice",
            recipient_id: recipient.verifying_key().to_bytes(),
            recipient_display_name: "Bob",
            match_length: 3,
            challenge_id: [41_u8; 32],
            game_id: [42_u8; 32],
            genesis_action_id: [43_u8; 32],
            created_at_unix_seconds: CREATED,
            expires_at_unix_seconds: EXPIRES,
        })
        .unwrap();

        ChallengeOfferState::new(plan.signed_offer, Vec::new()).unwrap()
    }

    #[test]
    fn probe_binds_exact_offer_full_key_and_empty_state() {
        let challenger = key(1);
        let recipient = key(2);
        let challenge = open_offer(&challenger, &recipient);
        let recipient_id = recipient.verifying_key().to_bytes();

        let first =
            prepare_incoming_challenge_contract_probe(&challenge, recipient_id, NOW).unwrap();

        let second =
            prepare_incoming_challenge_contract_probe(&challenge, recipient_id, NOW).unwrap();

        assert_eq!(first, second);
        assert_eq!(first.signed_offer, challenge.offer);
        assert_eq!(first.expected_contract_key.id().encode(), first.contract_id,);
        assert_eq!(
            first.expected_empty_state,
            [0xa1, 0x67, b'a', b'c', b't', b'i', b'o', b'n', b's', 0x80,],
        );
    }

    #[test]
    fn nonrecipient_expired_and_terminal_challenges_are_rejected() {
        let challenger = key(3);
        let recipient = key(4);
        let observer = key(5);
        let open = open_offer(&challenger, &recipient);

        assert!(prepare_incoming_challenge_contract_probe(
            &open,
            observer.verifying_key().to_bytes(),
            NOW,
        )
        .is_err());

        assert!(prepare_incoming_challenge_contract_probe(
            &open,
            recipient.verifying_key().to_bytes(),
            EXPIRES,
        )
        .is_err());

        let acceptance = accept_challenge(&open.offer, &recipient, NOW).unwrap();

        let accepted = ChallengeOfferState::new(
            open.offer,
            vec![ChallengeTerminalEvidence::Acceptance(acceptance)],
        )
        .unwrap();

        assert!(prepare_incoming_challenge_contract_probe(
            &accepted,
            recipient.verifying_key().to_bytes(),
            NOW,
        )
        .is_err());
    }

    #[test]
    fn finalization_rejects_wrong_key_or_nonempty_state() {
        let challenger = key(6);
        let recipient = key(7);
        let challenge = open_offer(&challenger, &recipient);

        let probe = prepare_incoming_challenge_contract_probe(
            &challenge,
            recipient.verifying_key().to_bytes(),
            NOW,
        )
        .unwrap();

        let wrong_contract = calculate_expected_game_contract([99_u8; 32]).unwrap();

        assert!(finalize_incoming_challenge_acceptance(
            &probe,
            &challenge,
            &wrong_contract.full_key,
            &probe.expected_empty_state,
            &recipient,
            NOW,
        )
        .is_err());

        let mut nonempty_state = probe.expected_empty_state.clone();
        nonempty_state.push(0);

        assert!(finalize_incoming_challenge_acceptance(
            &probe,
            &challenge,
            &probe.expected_contract_key,
            &nonempty_state,
            &recipient,
            NOW,
        )
        .is_err());
    }

    #[test]
    fn changed_authoritative_evidence_blocks_finalization() {
        let challenger = key(8);
        let recipient = key(9);
        let open = open_offer(&challenger, &recipient);

        let probe = prepare_incoming_challenge_contract_probe(
            &open,
            recipient.verifying_key().to_bytes(),
            NOW,
        )
        .unwrap();

        let acceptance = accept_challenge(&open.offer, &recipient, NOW).unwrap();

        let already_accepted = ChallengeOfferState::new(
            open.offer.clone(),
            vec![ChallengeTerminalEvidence::Acceptance(acceptance)],
        )
        .unwrap();

        assert!(finalize_incoming_challenge_acceptance(
            &probe,
            &already_accepted,
            &probe.expected_contract_key,
            &probe.expected_empty_state,
            &recipient,
            NOW,
        )
        .is_err());
    }

    #[test]
    fn exact_contract_proof_produces_verified_minimal_acceptance() {
        let challenger = key(10);
        let recipient = key(11);
        let challenge = open_offer(&challenger, &recipient);

        let probe = prepare_incoming_challenge_contract_probe(
            &challenge,
            recipient.verifying_key().to_bytes(),
            NOW,
        )
        .unwrap();

        let plan = finalize_incoming_challenge_acceptance(
            &probe,
            &challenge,
            &probe.expected_contract_key,
            &probe.expected_empty_state,
            &recipient,
            NOW,
        )
        .unwrap();

        verify_challenge_acceptance_at(&plan.signed_offer, &plan.acceptance, NOW).unwrap();

        assert_eq!(plan.accepted_proposal, challenge.offer.body.proposal,);
        assert_eq!(plan.verified_contract_key, probe.expected_contract_key,);
        assert_eq!(plan.contract_id, probe.contract_id);

        let decoded = decode_verified_lobby_state(&plan.encoded_lobby_state_update).unwrap();

        assert_eq!(decoded.challenges.offers.len(), 1);
        assert_eq!(
            resolve_challenge_at(
                &decoded.challenges.offers[0].offer,
                &decoded.challenges.offers[0].terminal_evidence,
                NOW,
            )
            .unwrap(),
            ChallengeResolution::Accepted {
                proposal: plan.accepted_proposal,
            },
        );
    }
}
