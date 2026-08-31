//! Pure construction of authenticated lobby-presence publications.
//!
//! Callers supply a previously reserved monotonic revision and an advisory
//! observation time. This module performs no browser storage, clock, Freenet,
//! or interface operations.

use backgammon_protocol::{
    sign_presence_announcement, PresenceAnnouncementBody, SignedPresenceAnnouncement,
    MAX_PRESENCE_LIFETIME_SECONDS,
};
use ed25519_dalek::SigningKey;

use crate::lobby_codec::build_encoded_presence_state_update;

/// Normal browser presence lifetime.
///
/// This is deliberately shorter than the protocol abuse/staleness ceiling.
/// Refresh policy remains outside this pure planner.
pub const PRESENCE_LEASE_SECONDS: u64 = 10 * 60;

const _: () = assert!(PRESENCE_LEASE_SECONDS <= MAX_PRESENCE_LIFETIME_SECONDS);

pub struct LobbyPresencePlannerInput<'a> {
    pub signing_key: &'a SigningKey,
    pub display_name: &'a str,
    pub available: bool,
    pub revision: u64,
    pub issued_at_unix_seconds: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LobbyPresencePlan {
    pub announcement: SignedPresenceAnnouncement,
    pub encoded_state_update: Vec<u8>,
}

/// Signs and encodes one minimal mergeable lobby-presence update.
///
/// Revisions are the ordering authority. The supplied timestamp only defines
/// this announcement's advisory liveness window.
pub fn plan_lobby_presence(
    input: LobbyPresencePlannerInput<'_>,
) -> Result<LobbyPresencePlan, String> {
    let expires_at_unix_seconds = input
        .issued_at_unix_seconds
        .checked_add(PRESENCE_LEASE_SECONDS)
        .ok_or_else(|| "Presence expiration exceeds the supported Unix range.".to_owned())?;

    let body = PresenceAnnouncementBody::new(
        input.signing_key.verifying_key().to_bytes(),
        input.display_name.to_owned(),
        input.available,
        input.revision,
        input.issued_at_unix_seconds,
        expires_at_unix_seconds,
    );

    let announcement = sign_presence_announcement(body, input.signing_key)
        .map_err(|error| format!("Could not sign lobby presence: {error}"))?;

    let encoded_state_update = build_encoded_presence_state_update(announcement.clone())
        .map_err(|error| format!("Could not encode lobby presence update: {error}"))?;

    Ok(LobbyPresencePlan {
        announcement,
        encoded_state_update,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lobby_codec::decode_verified_lobby_state;

    const ISSUED: u64 = 100_000;

    fn key() -> SigningKey {
        SigningKey::from_bytes(&[91; 32])
    }

    fn plan_for(available: bool) -> LobbyPresencePlan {
        let signing_key = key();

        plan_lobby_presence(LobbyPresencePlannerInput {
            signing_key: &signing_key,
            display_name: "Alice",
            available,
            revision: 7,
            issued_at_unix_seconds: ISSUED,
        })
        .unwrap()
    }

    #[test]
    fn available_and_unavailable_presence_round_trip_exactly() {
        for available in [true, false] {
            let plan = plan_for(available);
            let decoded = decode_verified_lobby_state(&plan.encoded_state_update).unwrap();

            assert_eq!(
                plan.announcement.body.player_id,
                key().verifying_key().to_bytes()
            );
            assert_eq!(plan.announcement.body.display_name, "Alice");
            assert_eq!(plan.announcement.body.available, available);
            assert_eq!(plan.announcement.body.revision, 7);
            assert_eq!(plan.announcement.body.issued_at_unix_seconds, ISSUED);
            assert_eq!(
                plan.announcement.body.expires_at_unix_seconds,
                ISSUED + PRESENCE_LEASE_SECONDS
            );

            assert_eq!(decoded.lobby.0.players.len(), 1);
            assert_eq!(decoded.lobby.0.players[0].records, vec![plan.announcement]);
            assert!(decoded.challenges.offers.is_empty());
        }
    }

    #[test]
    fn identical_inputs_produce_identical_publications() {
        assert_eq!(plan_for(true), plan_for(true));
    }

    #[test]
    fn invalid_revision_and_display_name_are_rejected() {
        let signing_key = key();

        assert!(plan_lobby_presence(LobbyPresencePlannerInput {
            signing_key: &signing_key,
            display_name: "Alice",
            available: true,
            revision: 0,
            issued_at_unix_seconds: ISSUED,
        })
        .is_err());

        assert!(plan_lobby_presence(LobbyPresencePlannerInput {
            signing_key: &signing_key,
            display_name: "   ",
            available: true,
            revision: 1,
            issued_at_unix_seconds: ISSUED,
        })
        .is_err());
    }

    #[test]
    fn expiration_overflow_is_rejected() {
        let signing_key = key();

        assert!(plan_lobby_presence(LobbyPresencePlannerInput {
            signing_key: &signing_key,
            display_name: "Alice",
            available: true,
            revision: 1,
            issued_at_unix_seconds: u64::MAX,
        })
        .is_err());
    }
}
