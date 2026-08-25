//! Verified encoding and decoding at the browser/lobby-contract boundary.
//!
//! This module contains no browser or Freenet API calls. Network state is
//! treated as hostile: complete lobby states are decoded, structurally
//! verified, and authenticated before callers may project or display them.

use backgammon_lobby_core::{
    ChallengeEntries, ChallengeOfferState, LobbyContractState, LobbyEntries, LobbyState,
};
use backgammon_protocol::SignedPresenceAnnouncement;
use ciborium::{de::from_reader, ser::into_writer};

/// Independently verifies every composable component of a decoded lobby state.
pub fn verify_lobby_state(state: &LobbyContractState) -> Result<(), String> {
    state
        .lobby
        .0
        .verify()
        .map_err(|error| format!("lobby presence failed verification: {error}"))?;

    state
        .challenges
        .verify_state()
        .map_err(|error| format!("lobby challenges failed verification: {error}"))
}

/// Decodes and independently verifies a complete authoritative lobby state.
pub fn decode_verified_lobby_state(bytes: &[u8]) -> Result<LobbyContractState, String> {
    if bytes.is_empty() {
        return Ok(LobbyContractState::default());
    }

    let state: LobbyContractState =
        from_reader(bytes).map_err(|error| format!("failed to decode lobby state: {error}"))?;

    verify_lobby_state(&state)?;
    Ok(state)
}

/// Encodes a verified lobby state and requires an exact typed round trip.
///
/// The resulting bytes may represent an authoritative state or a minimal
/// mergeable `UpdateData::State` payload.
pub fn encode_verified_lobby_state(state: &LobbyContractState) -> Result<Vec<u8>, String> {
    verify_lobby_state(state)?;

    let mut encoded = Vec::new();
    into_writer(state, &mut encoded)
        .map_err(|error| format!("failed to encode lobby state: {error}"))?;

    let decoded = decode_verified_lobby_state(&encoded)?;

    if decoded != *state {
        return Err("Encoded lobby state did not round-trip exactly.".to_owned());
    }

    Ok(encoded)
}

/// Builds a minimal parent-state update containing one authenticated presence
/// announcement and no challenge change.
pub fn build_encoded_presence_state_update(
    announcement: SignedPresenceAnnouncement,
) -> Result<Vec<u8>, String> {
    let lobby = LobbyState::from_announcement(announcement)?;

    encode_verified_lobby_state(&LobbyContractState {
        lobby: LobbyEntries(lobby),
        challenges: ChallengeEntries::default(),
    })
}

/// Builds a minimal parent-state update containing one authenticated challenge
/// offer state and no presence change.
pub fn build_encoded_challenge_state_update(offer: ChallengeOfferState) -> Result<Vec<u8>, String> {
    let challenges = ChallengeEntries::new(vec![offer])?;

    encode_verified_lobby_state(&LobbyContractState {
        lobby: LobbyEntries::default(),
        challenges,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use backgammon_protocol::{
        sign_challenge_offer, sign_presence_announcement, ChallengeOfferBody, GameConfiguration,
        GenesisProposal, PlayerDescriptor, PresenceAnnouncementBody,
    };
    use ed25519_dalek::SigningKey;
    use serde::Serialize;

    fn signed_presence(
        signing_key: &SigningKey,
        display_name: &str,
        available: bool,
        revision: u64,
    ) -> SignedPresenceAnnouncement {
        sign_presence_announcement(
            PresenceAnnouncementBody::new(
                signing_key.verifying_key().to_bytes(),
                display_name.to_owned(),
                available,
                revision,
                100_000,
                100_600,
            ),
            signing_key,
        )
        .unwrap()
    }

    fn challenge_offer_state(challenge_seed: u8) -> ChallengeOfferState {
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
            20_000,
            20_600,
            proposal,
        );

        let offer = sign_challenge_offer(body, &white_key).unwrap();
        ChallengeOfferState::new(offer, Vec::new()).unwrap()
    }

    fn raw_encode<T: Serialize>(value: &T) -> Vec<u8> {
        let mut encoded = Vec::new();
        into_writer(value, &mut encoded).unwrap();
        encoded
    }

    #[derive(Serialize)]
    struct LegacyPresenceOnlyLobbyState {
        lobby: LobbyEntries,
    }

    #[test]
    fn presence_update_round_trips_as_minimal_verified_parent_state() {
        let signing_key = SigningKey::from_bytes(&[31; 32]);
        let announcement = signed_presence(&signing_key, "Alice", true, 7);

        let encoded = build_encoded_presence_state_update(announcement.clone()).unwrap();
        let decoded = decode_verified_lobby_state(&encoded).unwrap();

        assert_eq!(decoded.lobby.0.players.len(), 1);
        assert_eq!(decoded.lobby.0.players[0].records, vec![announcement]);
        assert_eq!(decoded.challenges, ChallengeEntries::default());
    }

    #[test]
    fn challenge_update_round_trips_as_minimal_verified_parent_state() {
        let offer = challenge_offer_state(41);

        let encoded = build_encoded_challenge_state_update(offer.clone()).unwrap();
        let decoded = decode_verified_lobby_state(&encoded).unwrap();

        assert_eq!(decoded.lobby, LobbyEntries::default());
        assert_eq!(decoded.challenges.offers, vec![offer]);
    }

    #[test]
    fn combined_authoritative_state_round_trips_exactly() {
        let signing_key = SigningKey::from_bytes(&[32; 32]);
        let announcement = signed_presence(&signing_key, "Alice", true, 8);
        let offer = challenge_offer_state(42);

        let state = LobbyContractState {
            lobby: LobbyEntries(LobbyState::from_announcement(announcement).unwrap()),
            challenges: ChallengeEntries::new(vec![offer]).unwrap(),
        };

        let encoded = encode_verified_lobby_state(&state).unwrap();
        let decoded = decode_verified_lobby_state(&encoded).unwrap();

        assert_eq!(decoded, state);
    }

    #[test]
    fn forged_presence_is_rejected_after_decoding() {
        let signing_key = SigningKey::from_bytes(&[33; 32]);
        let announcement = signed_presence(&signing_key, "Alice", true, 9);
        let mut lobby = LobbyState::from_announcement(announcement).unwrap();

        lobby.players[0].records[0].body.revision += 1;

        let forged = LobbyContractState {
            lobby: LobbyEntries(lobby),
            challenges: ChallengeEntries::default(),
        };

        assert!(decode_verified_lobby_state(&raw_encode(&forged)).is_err());
    }

    #[test]
    fn forged_challenge_is_rejected_after_decoding() {
        let mut offer = challenge_offer_state(43);
        offer.offer.body.created_at_unix_seconds += 1;

        let forged = LobbyContractState {
            lobby: LobbyEntries::default(),
            challenges: ChallengeEntries {
                offers: vec![offer],
            },
        };

        assert!(decode_verified_lobby_state(&raw_encode(&forged)).is_err());
    }

    #[test]
    fn empty_contract_state_decodes_as_verified_default() {
        assert_eq!(
            decode_verified_lobby_state(&[]).unwrap(),
            LobbyContractState::default()
        );
    }

    #[test]
    fn malformed_cbor_is_rejected() {
        assert!(decode_verified_lobby_state(&[0x9f, 0x01]).is_err());
    }

    #[test]
    fn legacy_presence_only_state_defaults_challenges() {
        let signing_key = SigningKey::from_bytes(&[34; 32]);
        let announcement = signed_presence(&signing_key, "Alice", true, 10);

        let legacy = LegacyPresenceOnlyLobbyState {
            lobby: LobbyEntries(LobbyState::from_announcement(announcement).unwrap()),
        };

        let decoded = decode_verified_lobby_state(&raw_encode(&legacy)).unwrap();

        assert_eq!(decoded.lobby, legacy.lobby);
        assert_eq!(decoded.challenges, ChallengeEntries::default());
    }
}
