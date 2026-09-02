use std::io::Cursor;

use backgammon_lobby_core::{ChallengeOfferState, LobbyContractState};
use backgammon_protocol::{
    accepted_genesis_proposal, verify_challenge_acceptance, verify_challenge_offer,
    ChallengeAcceptance, ChallengeId, ChallengeTerminalEvidence, PlayerId, SignedChallengeOffer,
};
use ciborium::{de::from_reader, ser::into_writer};
use freenet_stdlib::prelude::ContractKey;
use serde::{Deserialize, Serialize};

use crate::game_contract_publication::calculate_expected_game_contract;
use crate::incoming_challenge_acceptance_planner::IncomingChallengeAcceptancePlan;
use crate::lobby_codec::build_encoded_challenge_state_update;

/// Generous bound for one exact signed offer, recipient acceptance, full
/// contract key, and canonical contract ID.
///
/// Hostile or corrupt browser data is rejected before CBOR decoding.
pub const MAX_INCOMING_CHALLENGE_ACCEPTANCE_BYTES: usize = 8192;

const STORAGE_PREFIX: &str = "freenet-backgammon.incoming-challenge-acceptance.v1";

/// Durable recipient evidence created only after exact game-contract proof.
///
/// This record contains no private signing key. The acceptance signature is
/// retained so refresh recovery can rebuild and resubmit the identical lobby
/// update without producing a second signature.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredIncomingChallengeAcceptance {
    pub signed_offer: SignedChallengeOffer,
    pub acceptance: ChallengeAcceptance,
    pub local_player_id: PlayerId,
    pub verified_contract_key: ContractKey,
    pub contract_id: String,
}

impl StoredIncomingChallengeAcceptance {
    /// Converts a fully verified acceptance plan into durable evidence.
    pub fn new(plan: &IncomingChallengeAcceptancePlan) -> Result<Self, String> {
        let stored = Self {
            signed_offer: plan.signed_offer.clone(),
            acceptance: plan.acceptance.clone(),
            local_player_id: plan.acceptance.player_id,
            verified_contract_key: plan.verified_contract_key,
            contract_id: plan.contract_id.clone(),
        };

        stored.verify()?;

        /*
         * Rebuild every derived field and byte from authenticated evidence.
         * This prevents persistence of a plan whose offer, acceptance,
         * contract identity, proposal, or lobby update disagree.
         */
        if stored.rebuild_plan()? != *plan {
            return Err("Incoming acceptance plan does not match its authenticated \
                 durable evidence."
                .to_owned());
        }

        Ok(stored)
    }

    pub fn verify(&self) -> Result<(), String> {
        verify_challenge_offer(&self.signed_offer).map_err(|error| {
            format!(
                "Stored incoming challenge offer failed \
                     verification: {error}"
            )
        })?;

        verify_challenge_acceptance(&self.signed_offer, &self.acceptance).map_err(|error| {
            format!(
                "Stored incoming challenge acceptance failed \
                 verification: {error}"
            )
        })?;

        let recipient_id = self.signed_offer.body.recipient_id().map_err(|error| {
            format!(
                "Could not resolve stored challenge recipient: \
                         {error}"
            )
        })?;

        if self.local_player_id != recipient_id || self.acceptance.player_id != self.local_player_id
        {
            return Err("Stored incoming challenge acceptance belongs to a \
                 different local identity."
                .to_owned());
        }

        let proposal =
            accepted_genesis_proposal(&self.signed_offer, &self.acceptance).map_err(|error| {
                format!(
                    "Stored acceptance does not authenticate a valid \
                 genesis proposal: {error}"
                )
            })?;

        let expected = calculate_expected_game_contract(proposal.game_id)?;

        if self.verified_contract_key != expected.full_key {
            return Err("Stored incoming acceptance contains an unexpected \
                 full game-contract key."
                .to_owned());
        }

        if self.contract_id != expected.contract_id
            || self.verified_contract_key.id().encode() != self.contract_id
        {
            return Err("Stored incoming acceptance contains a noncanonical \
                 game-contract ID."
                .to_owned());
        }

        Ok(())
    }

    /// Rebuilds the exact signed plan after refresh without signing again.
    pub fn rebuild_plan(&self) -> Result<IncomingChallengeAcceptancePlan, String> {
        self.verify()?;

        let accepted_proposal = accepted_genesis_proposal(&self.signed_offer, &self.acceptance)
            .map_err(|error| format!("Could not rebuild accepted genesis proposal: {error}"))?;

        let accepted_state = ChallengeOfferState::new(
            self.signed_offer.clone(),
            vec![ChallengeTerminalEvidence::Acceptance(
                self.acceptance.clone(),
            )],
        )
        .map_err(|error| format!("Could not rebuild accepted challenge state: {error}"))?;

        let encoded_lobby_state_update = build_encoded_challenge_state_update(accepted_state)
            .map_err(|error| {
                format!(
                    "Could not rebuild acceptance lobby update: \
                         {error}"
                )
            })?;

        Ok(IncomingChallengeAcceptancePlan {
            signed_offer: self.signed_offer.clone(),
            acceptance: self.acceptance.clone(),
            accepted_proposal,
            verified_contract_key: self.verified_contract_key,
            contract_id: self.contract_id.clone(),
            encoded_lobby_state_update,
        })
    }

    /// Returns true only when complete, independently verified authoritative
    /// lobby state contains this exact signed offer and exact acceptance.
    pub fn is_exact_acceptance_authoritative(
        &self,
        state: &LobbyContractState,
    ) -> Result<bool, String> {
        self.verify()?;

        state.lobby.0.verify().map_err(|error| {
            format!(
                "Authoritative lobby presence failed verification: \
                 {error}"
            )
        })?;

        state.challenges.verify_state().map_err(|error| {
            format!(
                "Authoritative lobby challenges failed verification: \
                 {error}"
            )
        })?;

        Ok(state.challenges.offers.iter().any(|entry| {
            entry.offer == self.signed_offer
                && entry.terminal_evidence.iter().any(|evidence| {
                    evidence == &ChallengeTerminalEvidence::Acceptance(self.acceptance.clone())
                })
        }))
    }

    pub fn challenge_id(&self) -> ChallengeId {
        self.signed_offer.body.challenge_id
    }

    pub fn encode(&self) -> Result<Vec<u8>, String> {
        self.verify()?;

        let mut encoded = Vec::new();

        into_writer(self, &mut encoded).map_err(|error| {
            format!(
                "Could not encode stored incoming challenge \
                 acceptance: {error:?}"
            )
        })?;

        if encoded.len() > MAX_INCOMING_CHALLENGE_ACCEPTANCE_BYTES {
            return Err(format!(
                "Encoded incoming challenge acceptance exceeds {} \
                 bytes.",
                MAX_INCOMING_CHALLENGE_ACCEPTANCE_BYTES,
            ));
        }

        Ok(encoded)
    }

    pub fn decode(encoded: &[u8]) -> Result<Self, String> {
        if encoded.len() > MAX_INCOMING_CHALLENGE_ACCEPTANCE_BYTES {
            return Err(format!(
                "Stored incoming challenge acceptance exceeds {} \
                 bytes.",
                MAX_INCOMING_CHALLENGE_ACCEPTANCE_BYTES,
            ));
        }

        let mut cursor = Cursor::new(encoded);

        let decoded: Self = from_reader(&mut cursor).map_err(|error| {
            format!(
                "Could not decode stored incoming challenge \
                     acceptance: {error:?}"
            )
        })?;

        if cursor.position() != encoded.len() as u64 {
            return Err("Stored incoming challenge acceptance contains \
                 trailing noncanonical data."
                .to_owned());
        }

        decoded.verify()?;

        if decoded.encode()? != encoded {
            return Err("Stored incoming challenge acceptance is not \
                 canonically encoded."
                .to_owned());
        }

        Ok(decoded)
    }
}

pub fn incoming_challenge_acceptance_storage_key(local_player_id: &PlayerId) -> String {
    format!("{STORAGE_PREFIX}.{}", encode_hex(local_player_id),)
}

#[cfg(target_arch = "wasm32")]
pub fn store_new_incoming_challenge_acceptance(
    stored: &StoredIncomingChallengeAcceptance,
) -> Result<(), String> {
    stored.verify()?;

    let storage = browser_storage()?;
    let key = incoming_challenge_acceptance_storage_key(&stored.local_player_id);

    if let Some(existing_hex) = storage.get_item(&key).map_err(|error| {
        format!(
            "Could not inspect pending incoming acceptance \
                 storage: {error:?}"
        )
    })? {
        let existing_bytes =
            decode_hex_bounded(&existing_hex, MAX_INCOMING_CHALLENGE_ACCEPTANCE_BYTES)?;

        let existing = StoredIncomingChallengeAcceptance::decode(&existing_bytes)?;

        if existing == *stored {
            return Ok(());
        }

        return Err("A different incoming challenge acceptance is already \
             pending for this identity."
            .to_owned());
    }

    persist_exact(&storage, &key, stored)
}

#[cfg(target_arch = "wasm32")]
pub fn load_incoming_challenge_acceptance(
    local_player_id: &PlayerId,
) -> Result<Option<StoredIncomingChallengeAcceptance>, String> {
    let storage = browser_storage()?;
    let key = incoming_challenge_acceptance_storage_key(local_player_id);

    let Some(encoded_hex) = storage.get_item(&key).map_err(|error| {
        format!(
            "Could not read pending incoming challenge \
                 acceptance: {error:?}"
        )
    })?
    else {
        return Ok(None);
    };

    let encoded = decode_hex_bounded(&encoded_hex, MAX_INCOMING_CHALLENGE_ACCEPTANCE_BYTES)?;

    let stored = StoredIncomingChallengeAcceptance::decode(&encoded)?;

    if stored.local_player_id != *local_player_id {
        return Err("Stored incoming challenge acceptance belongs to a \
             different local identity."
            .to_owned());
    }

    if encode_hex(&encoded) != encoded_hex {
        return Err("Stored incoming challenge acceptance hexadecimal \
             encoding is noncanonical."
            .to_owned());
    }

    Ok(Some(stored))
}

#[cfg(target_arch = "wasm32")]
pub fn remove_incoming_challenge_acceptance(
    local_player_id: &PlayerId,
    expected_challenge_id: &ChallengeId,
) -> Result<(), String> {
    let storage = browser_storage()?;
    let key = incoming_challenge_acceptance_storage_key(local_player_id);

    let Some(existing) = load_incoming_challenge_acceptance(local_player_id)? else {
        return Ok(());
    };

    if existing.challenge_id() != *expected_challenge_id {
        return Err("Refusing to remove a different pending incoming \
             challenge acceptance."
            .to_owned());
    }

    storage.remove_item(&key).map_err(|error| {
        format!(
            "Could not remove pending incoming challenge \
             acceptance: {error:?}"
        )
    })?;

    if storage
        .get_item(&key)
        .map_err(|error| {
            format!(
                "Could not verify incoming acceptance removal: \
                 {error:?}"
            )
        })?
        .is_some()
    {
        return Err("Browser storage retained the incoming acceptance after \
             removal."
            .to_owned());
    }

    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn persist_exact(
    storage: &web_sys::Storage,
    key: &str,
    stored: &StoredIncomingChallengeAcceptance,
) -> Result<(), String> {
    let encoded = stored.encode()?;
    let encoded_hex = encode_hex(&encoded);

    storage.set_item(key, &encoded_hex).map_err(|error| {
        format!(
            "Could not persist incoming challenge acceptance: \
                 {error:?}"
        )
    })?;

    let persisted_hex = storage
        .get_item(key)
        .map_err(|error| {
            format!(
                "Could not verify persisted incoming challenge \
                 acceptance: {error:?}"
            )
        })?
        .ok_or_else(|| {
            "Browser storage did not retain the incoming challenge \
             acceptance."
                .to_owned()
        })?;

    let persisted_bytes =
        decode_hex_bounded(&persisted_hex, MAX_INCOMING_CHALLENGE_ACCEPTANCE_BYTES)?;

    let persisted = StoredIncomingChallengeAcceptance::decode(&persisted_bytes)?;

    if persisted != *stored
        || persisted_bytes != encoded
        || encode_hex(&persisted_bytes) != persisted_hex
    {
        let _ = storage.remove_item(key);

        return Err("Persisted incoming challenge acceptance failed exact \
             round-trip verification."
            .to_owned());
    }

    Ok(())
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";

    let mut encoded = String::with_capacity(bytes.len() * 2);

    for byte in bytes {
        encoded.push(HEX[usize::from(byte >> 4)] as char);
        encoded.push(HEX[usize::from(byte & 0x0f)] as char);
    }

    encoded
}

#[cfg(target_arch = "wasm32")]
fn decode_hex_bounded(encoded: &str, maximum_bytes: usize) -> Result<Vec<u8>, String> {
    if encoded.len() > maximum_bytes.saturating_mul(2) {
        return Err("Stored incoming acceptance hexadecimal data is \
             oversized."
            .to_owned());
    }

    if encoded.len() % 2 != 0 {
        return Err("Stored incoming acceptance hexadecimal data has odd \
             length."
            .to_owned());
    }

    let bytes = encoded.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len() / 2);

    for pair in bytes.chunks_exact(2) {
        let high = decode_lower_hex_nibble(pair[0])?;
        let low = decode_lower_hex_nibble(pair[1])?;
        decoded.push((high << 4) | low);
    }

    if encode_hex(&decoded) != encoded {
        return Err("Stored incoming acceptance hexadecimal data is \
             noncanonical."
            .to_owned());
    }

    Ok(decoded)
}

#[cfg(target_arch = "wasm32")]
fn decode_lower_hex_nibble(value: u8) -> Result<u8, String> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        _ => Err("Stored incoming acceptance contains noncanonical \
             hexadecimal data."
            .to_owned()),
    }
}

#[cfg(target_arch = "wasm32")]
fn browser_storage() -> Result<web_sys::Storage, String> {
    let window = web_sys::window().ok_or_else(|| "Browser window is unavailable.".to_owned())?;

    window
        .local_storage()
        .map_err(|error| format!("Browser storage is unavailable: {error:?}"))?
        .ok_or_else(|| "Browser local storage is disabled.".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    use backgammon_lobby_core::{ChallengeEntries, LobbyEntries};
    use ed25519_dalek::SigningKey;

    use crate::challenge_offer_planner::{plan_outbound_challenge, OutboundChallengePlannerInput};
    use crate::incoming_challenge_acceptance_planner::{
        finalize_incoming_challenge_acceptance, prepare_incoming_challenge_contract_probe,
    };

    const CREATED: u64 = 800_000;
    const NOW: u64 = CREATED + 1;
    const EXPIRES: u64 = CREATED + 600;

    fn key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn acceptance_plan() -> IncomingChallengeAcceptancePlan {
        let challenger = key(31);
        let recipient = key(32);

        let outbound = plan_outbound_challenge(OutboundChallengePlannerInput {
            signing_key: &challenger,
            challenger_display_name: "Alice",
            recipient_id: recipient.verifying_key().to_bytes(),
            recipient_display_name: "Bob",
            match_length: 3,
            challenge_id: [81_u8; 32],
            game_id: [82_u8; 32],
            genesis_action_id: [83_u8; 32],
            created_at_unix_seconds: CREATED,
            expires_at_unix_seconds: EXPIRES,
        })
        .unwrap();

        let open = ChallengeOfferState::new(outbound.signed_offer, Vec::new()).unwrap();

        let probe = prepare_incoming_challenge_contract_probe(
            &open,
            recipient.verifying_key().to_bytes(),
            NOW,
        )
        .unwrap();

        finalize_incoming_challenge_acceptance(
            &probe,
            &open,
            &probe.expected_contract_key,
            &probe.expected_empty_state,
            &recipient,
            NOW,
        )
        .unwrap()
    }

    #[test]
    fn stored_acceptance_round_trips_canonically() {
        let stored = StoredIncomingChallengeAcceptance::new(&acceptance_plan()).unwrap();

        let encoded = stored.encode().unwrap();
        let decoded = StoredIncomingChallengeAcceptance::decode(&encoded).unwrap();

        assert_eq!(decoded, stored);
        assert_eq!(decoded.encode().unwrap(), encoded);
    }

    #[test]
    fn durable_evidence_rebuilds_the_exact_plan() {
        let plan = acceptance_plan();
        let stored = StoredIncomingChallengeAcceptance::new(&plan).unwrap();

        assert_eq!(stored.rebuild_plan().unwrap(), plan);
    }

    #[test]
    fn identity_acceptance_and_contract_tampering_are_rejected() {
        let original = StoredIncomingChallengeAcceptance::new(&acceptance_plan()).unwrap();

        let mut wrong_identity = original.clone();
        wrong_identity.local_player_id = [91_u8; 32];
        assert!(wrong_identity.verify().is_err());

        let mut forged_acceptance = original.clone();
        forged_acceptance.acceptance.signature.0[0] ^= 0xff;
        assert!(forged_acceptance.verify().is_err());

        let mut wrong_contract = original.clone();
        wrong_contract.verified_contract_key = calculate_expected_game_contract([92_u8; 32])
            .unwrap()
            .full_key;
        assert!(wrong_contract.verify().is_err());

        let mut wrong_id = original;
        wrong_id.contract_id.push('1');
        assert!(wrong_id.verify().is_err());
    }

    #[test]
    fn only_exact_authoritative_acceptance_is_recognized() {
        let plan = acceptance_plan();
        let stored = StoredIncomingChallengeAcceptance::new(&plan).unwrap();

        let open = ChallengeOfferState::new(plan.signed_offer.clone(), Vec::new()).unwrap();

        let open_state = LobbyContractState {
            lobby: LobbyEntries::default(),
            challenges: ChallengeEntries::new(vec![open]).unwrap(),
        };

        assert!(!stored
            .is_exact_acceptance_authoritative(&open_state)
            .unwrap());

        let accepted = ChallengeOfferState::new(
            plan.signed_offer.clone(),
            vec![ChallengeTerminalEvidence::Acceptance(
                plan.acceptance.clone(),
            )],
        )
        .unwrap();

        let accepted_state = LobbyContractState {
            lobby: LobbyEntries::default(),
            challenges: ChallengeEntries::new(vec![accepted]).unwrap(),
        };

        assert!(stored
            .is_exact_acceptance_authoritative(&accepted_state,)
            .unwrap());
    }

    #[test]
    fn storage_key_is_identity_scoped_and_canonical() {
        let first = incoming_challenge_acceptance_storage_key(&[0x11_u8; 32]);

        let second = incoming_challenge_acceptance_storage_key(&[0x22_u8; 32]);

        assert_ne!(first, second);
        assert_eq!(first, format!("{STORAGE_PREFIX}.{}", "11".repeat(32)),);
    }

    #[test]
    fn malformed_oversized_and_trailing_data_are_rejected() {
        let stored = StoredIncomingChallengeAcceptance::new(&acceptance_plan()).unwrap();

        let encoded = stored.encode().unwrap();

        let mut trailing = encoded.clone();
        trailing.push(0);

        assert!(StoredIncomingChallengeAcceptance::decode(&trailing,).is_err());

        assert!(StoredIncomingChallengeAcceptance::decode(&vec![
            0_u8;
            MAX_INCOMING_CHALLENGE_ACCEPTANCE_BYTES
                + 1
        ],)
        .is_err());

        let mut forged = stored;
        forged.acceptance.signature.0[0] ^= 0xff;

        let mut forged_bytes = Vec::new();
        into_writer(&forged, &mut forged_bytes).unwrap();

        assert!(StoredIncomingChallengeAcceptance::decode(&forged_bytes,).is_err());
    }
}
