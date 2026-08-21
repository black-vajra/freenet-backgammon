use crate::{PlayerId, ED25519_SIGNATURE_BYTES, MAX_DISPLAY_NAME_BYTES};
use ciborium::ser::into_writer;
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};

/*
 * Lobby presence is versioned independently from both challenge negotiation
 * and replicated game actions.
 */
pub const LOBBY_PROTOCOL_VERSION: u16 = 1;

/*
 * This is a protocol abuse/staleness ceiling, not the eventual refresh
 * interval. The browser/Freenet integration may use a much shorter normal
 * presence lifetime after real network testing.
 */
pub const MAX_PRESENCE_LIFETIME_SECONDS: u64 = 60 * 60;

const PRESENCE_SIGNATURE_DOMAIN_V1: &[u8] = b"freenet-backgammon/lobby-presence/v1";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresenceSignature(pub Vec<u8>);

impl PresenceSignature {
    fn from_bytes(bytes: [u8; ED25519_SIGNATURE_BYTES]) -> Self {
        Self(bytes.to_vec())
    }

    pub fn verify_structure(&self) -> Result<(), String> {
        if self.0.len() != ED25519_SIGNATURE_BYTES {
            return Err(format!(
                "Invalid presence signature length: expected \
                 {ED25519_SIGNATURE_BYTES} bytes, got {}.",
                self.0.len()
            ));
        }

        Ok(())
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

/// One identity's signed lobby-presence statement.
///
/// `revision` is the ordering authority for announcements from the same
/// PlayerId. Wall-clock timestamps are only liveness metadata and MUST NOT be
/// used to decide which signed announcement is newer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresenceAnnouncementBody {
    pub protocol_version: u16,
    pub player_id: PlayerId,
    pub display_name: String,
    pub available: bool,
    pub revision: u64,
    pub issued_at_unix_seconds: u64,
    pub expires_at_unix_seconds: u64,
}

impl PresenceAnnouncementBody {
    pub fn new(
        player_id: PlayerId,
        display_name: String,
        available: bool,
        revision: u64,
        issued_at_unix_seconds: u64,
        expires_at_unix_seconds: u64,
    ) -> Self {
        Self {
            protocol_version: LOBBY_PROTOCOL_VERSION,
            player_id,
            display_name,
            available,
            revision,
            issued_at_unix_seconds,
            expires_at_unix_seconds,
        }
    }

    pub fn verify(&self) -> Result<(), String> {
        if self.protocol_version != LOBBY_PROTOCOL_VERSION {
            return Err(format!(
                "Lobby protocol version mismatch: expected {}, got {}.",
                LOBBY_PROTOCOL_VERSION, self.protocol_version
            ));
        }

        validate_display_name(&self.display_name)?;

        if self.revision == 0 {
            return Err("Presence revision must be greater than zero.".to_owned());
        }

        if self.expires_at_unix_seconds <= self.issued_at_unix_seconds {
            return Err("Presence expiration must be later than its issue time.".to_owned());
        }

        let lifetime = self
            .expires_at_unix_seconds
            .checked_sub(self.issued_at_unix_seconds)
            .ok_or_else(|| "Presence lifetime underflowed.".to_owned())?;

        if lifetime > MAX_PRESENCE_LIFETIME_SECONDS {
            return Err(format!(
                "Presence lifetime exceeds the maximum of \
                 {MAX_PRESENCE_LIFETIME_SECONDS} seconds."
            ));
        }

        Ok(())
    }

    pub fn verify_not_expired_at(&self, now_unix_seconds: u64) -> Result<(), String> {
        self.verify()?;

        if now_unix_seconds >= self.expires_at_unix_seconds {
            return Err("Presence announcement has expired.".to_owned());
        }

        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedPresenceAnnouncement {
    pub body: PresenceAnnouncementBody,
    pub signature: PresenceSignature,
}

/// Deterministic interpretation of all valid signed announcements currently
/// known for one PlayerId.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PresenceResolution {
    Absent,

    Available {
        player_id: PlayerId,
        display_name: String,
        revision: u64,
        expires_at_unix_seconds: u64,
    },

    Unavailable {
        revision: u64,
    },

    Expired {
        revision: u64,
    },

    Equivocation {
        revision: u64,
    },
}

/// Matches the existing game-protocol PlayerDescriptor display-name rule.
///
/// Names are presentation metadata, not identities. Duplicate display names
/// are therefore allowed; PlayerId remains authoritative.
pub fn validate_display_name(display_name: &str) -> Result<(), String> {
    if display_name.trim().is_empty() {
        return Err("Display name must not be empty.".to_owned());
    }

    /*
     * String::len() is the UTF-8 byte count, matching the game protocol's
     * MAX_DISPLAY_NAME_BYTES rule exactly.
     */
    if display_name.len() > MAX_DISPLAY_NAME_BYTES {
        return Err(format!(
            "Display name exceeds the maximum of \
             {MAX_DISPLAY_NAME_BYTES} UTF-8 bytes."
        ));
    }

    Ok(())
}

fn presence_signing_message(body: &PresenceAnnouncementBody) -> Result<Vec<u8>, String> {
    let mut encoded = Vec::new();

    into_writer(body, &mut encoded)
        .map_err(|error| format!("Could not encode presence signing body: {error}"))?;

    let mut message = Vec::with_capacity(PRESENCE_SIGNATURE_DOMAIN_V1.len() + 1 + encoded.len());

    message.extend_from_slice(PRESENCE_SIGNATURE_DOMAIN_V1);
    message.push(0);
    message.extend_from_slice(&encoded);

    Ok(message)
}

pub fn sign_presence_announcement(
    body: PresenceAnnouncementBody,
    signing_key: &SigningKey,
) -> Result<SignedPresenceAnnouncement, String> {
    body.verify()?;

    let player_id = signing_key.verifying_key().to_bytes();

    if player_id != body.player_id {
        return Err("Presence announcement cannot be signed by a different identity.".to_owned());
    }

    let message = presence_signing_message(&body)?;

    let signed = SignedPresenceAnnouncement {
        body,
        signature: PresenceSignature::from_bytes(signing_key.sign(&message).to_bytes()),
    };

    /*
     * Verify locally generated output through the exact hostile-input path.
     */
    verify_presence_announcement(&signed)?;

    Ok(signed)
}

/// Cryptographically verifies a presence announcement without applying local
/// wall-clock expiry policy.
pub fn verify_presence_announcement(
    announcement: &SignedPresenceAnnouncement,
) -> Result<(), String> {
    announcement.body.verify()?;
    announcement.signature.verify_structure()?;

    let signature_bytes: [u8; ED25519_SIGNATURE_BYTES] = announcement
        .signature
        .as_bytes()
        .try_into()
        .map_err(|_| "Presence signature has an invalid byte length.".to_owned())?;

    let verifying_key = VerifyingKey::from_bytes(&announcement.body.player_id)
        .map_err(|error| format!("Invalid Ed25519 presence identity: {error}"))?;

    let signature = Signature::from_bytes(&signature_bytes);
    let message = presence_signing_message(&announcement.body)?;

    verifying_key
        .verify_strict(&message, &signature)
        .map_err(|error| format!("Invalid presence signature: {error}"))
}

/// Live-processing verifier. Expiration affects current discoverability but
/// does not retroactively invalidate the cryptographic signature.
pub fn verify_presence_announcement_at(
    announcement: &SignedPresenceAnnouncement,
    now_unix_seconds: u64,
) -> Result<(), String> {
    verify_presence_announcement(announcement)?;
    announcement.body.verify_not_expired_at(now_unix_seconds)
}

/// Resolves all announcements for one PlayerId.
///
/// Invalid or forged records are ignored instead of poisoning the player's
/// entire lobby projection. A cryptographically valid same-revision conflict,
/// however, proves that the identity itself signed contradictory state and is
/// surfaced as Equivocation.
///
/// The highest valid revision always wins. If that revision has expired, older
/// revisions are NOT revived; a fresh higher revision is required.
pub fn resolve_player_presence(
    player_id: PlayerId,
    announcements: &[SignedPresenceAnnouncement],
    now_unix_seconds: u64,
) -> PresenceResolution {
    let mut highest_revision: Option<u64> = None;
    let mut highest_bodies: Vec<&PresenceAnnouncementBody> = Vec::new();

    for announcement in announcements {
        if announcement.body.player_id != player_id {
            continue;
        }

        if verify_presence_announcement(announcement).is_err() {
            continue;
        }

        let revision = announcement.body.revision;

        match highest_revision {
            None => {
                highest_revision = Some(revision);
                highest_bodies.push(&announcement.body);
            }

            Some(current) if revision > current => {
                highest_revision = Some(revision);
                highest_bodies.clear();
                highest_bodies.push(&announcement.body);
            }

            Some(current) if revision == current => {
                if !highest_bodies
                    .iter()
                    .any(|body| **body == announcement.body)
                {
                    highest_bodies.push(&announcement.body);
                }
            }

            Some(_) => {}
        }
    }

    let Some(revision) = highest_revision else {
        return PresenceResolution::Absent;
    };

    if highest_bodies.len() != 1 {
        return PresenceResolution::Equivocation { revision };
    }

    let body = highest_bodies[0];

    if now_unix_seconds >= body.expires_at_unix_seconds {
        return PresenceResolution::Expired { revision };
    }

    if body.available {
        PresenceResolution::Available {
            player_id,
            display_name: body.display_name.clone(),
            revision,
            expires_at_unix_seconds: body.expires_at_unix_seconds,
        }
    } else {
        PresenceResolution::Unavailable { revision }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{GameConfiguration, PlayerDescriptor};

    const ISSUED: u64 = 50_000;
    const EXPIRES: u64 = 50_600;

    fn key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn body(
        signing_key: &SigningKey,
        name: &str,
        available: bool,
        revision: u64,
    ) -> PresenceAnnouncementBody {
        PresenceAnnouncementBody::new(
            signing_key.verifying_key().to_bytes(),
            name.to_owned(),
            available,
            revision,
            ISSUED,
            EXPIRES,
        )
    }

    fn signed(
        signing_key: &SigningKey,
        name: &str,
        available: bool,
        revision: u64,
    ) -> SignedPresenceAnnouncement {
        sign_presence_announcement(body(signing_key, name, available, revision), signing_key)
            .unwrap()
    }

    #[test]
    fn lobby_name_rule_matches_game_configuration_rule() {
        let white_key = key(81);
        let black_key = key(82);

        for name in ["Alice".to_owned(), "é".repeat(MAX_DISPLAY_NAME_BYTES / 2)] {
            assert_eq!(validate_display_name(&name), Ok(()));

            assert_eq!(
                GameConfiguration {
                    white: PlayerDescriptor {
                        id: white_key.verifying_key().to_bytes(),
                        display_name: name,
                    },
                    black: PlayerDescriptor {
                        id: black_key.verifying_key().to_bytes(),
                        display_name: "Bob".to_owned(),
                    },
                    match_length: 1,
                }
                .verify(),
                Ok(())
            );
        }

        for name in ["   ".to_owned(), "x".repeat(MAX_DISPLAY_NAME_BYTES + 1)] {
            assert!(validate_display_name(&name).is_err());

            assert!(GameConfiguration {
                white: PlayerDescriptor {
                    id: white_key.verifying_key().to_bytes(),
                    display_name: name,
                },
                black: PlayerDescriptor {
                    id: black_key.verifying_key().to_bytes(),
                    display_name: "Bob".to_owned(),
                },
                match_length: 1,
            }
            .verify()
            .is_err());
        }
    }

    #[test]
    fn identity_can_sign_canonical_presence() {
        let signing_key = key(83);
        let body = body(&signing_key, "Alice", true, 1);

        let announcement = sign_presence_announcement(body.clone(), &signing_key).unwrap();

        assert_eq!(announcement.body, body);
        assert_eq!(verify_presence_announcement(&announcement), Ok(()));
        assert_eq!(
            verify_presence_announcement_at(&announcement, ISSUED),
            Ok(())
        );
    }

    #[test]
    fn wrong_identity_cannot_sign_presence() {
        let alice = key(84);
        let bob = key(85);

        assert!(sign_presence_announcement(body(&alice, "Alice", true, 1), &bob,).is_err());
    }

    #[test]
    fn signed_presence_does_not_survive_mutation() {
        let signing_key = key(86);
        let mut announcement = signed(&signing_key, "Alice", true, 1);

        announcement.body.display_name = "Mallory".to_owned();

        assert!(verify_presence_announcement(&announcement).is_err());
    }

    #[test]
    fn invalid_revision_or_lifetime_is_rejected() {
        let signing_key = key(87);

        let mut invalid = body(&signing_key, "Alice", true, 0);
        assert!(invalid.verify().is_err());

        invalid.revision = 1;
        invalid.expires_at_unix_seconds = invalid.issued_at_unix_seconds;
        assert!(invalid.verify().is_err());

        invalid.expires_at_unix_seconds =
            invalid.issued_at_unix_seconds + MAX_PRESENCE_LIFETIME_SECONDS + 1;

        assert!(invalid.verify().is_err());
    }

    #[test]
    fn live_presence_expires_at_boundary() {
        let signing_key = key(88);
        let announcement = signed(&signing_key, "Alice", true, 1);

        assert_eq!(
            verify_presence_announcement_at(&announcement, EXPIRES - 1),
            Ok(())
        );

        assert!(verify_presence_announcement_at(&announcement, EXPIRES).is_err());

        /*
         * Cryptographic validity itself does not disappear with time.
         */
        assert_eq!(verify_presence_announcement(&announcement), Ok(()));
    }

    #[test]
    fn higher_revision_wins_regardless_of_delivery_order() {
        let signing_key = key(89);
        let player_id = signing_key.verifying_key().to_bytes();

        let first = signed(&signing_key, "Alice", true, 1);
        let second = signed(&signing_key, "Alice Two", true, 2);

        let expected = PresenceResolution::Available {
            player_id,
            display_name: "Alice Two".to_owned(),
            revision: 2,
            expires_at_unix_seconds: EXPIRES,
        };

        assert_eq!(
            resolve_player_presence(player_id, &[first.clone(), second.clone()], ISSUED + 1,),
            expected
        );

        assert_eq!(
            resolve_player_presence(player_id, &[second, first], ISSUED + 1,),
            expected
        );
    }

    #[test]
    fn newer_unavailable_revision_suppresses_old_available_revision() {
        let signing_key = key(90);
        let player_id = signing_key.verifying_key().to_bytes();

        let available = signed(&signing_key, "Alice", true, 1);
        let unavailable = signed(&signing_key, "Alice", false, 2);

        assert_eq!(
            resolve_player_presence(player_id, &[available, unavailable], ISSUED + 1,),
            PresenceResolution::Unavailable { revision: 2 }
        );
    }

    #[test]
    fn expired_highest_revision_does_not_revive_older_presence() {
        let signing_key = key(91);
        let player_id = signing_key.verifying_key().to_bytes();

        let older = signed(&signing_key, "Alice", true, 1);

        let mut newer_body = body(&signing_key, "Alice", true, 2);
        newer_body.expires_at_unix_seconds = ISSUED + 10;

        let newer = sign_presence_announcement(newer_body, &signing_key).unwrap();

        assert_eq!(
            resolve_player_presence(player_id, &[older, newer], ISSUED + 10,),
            PresenceResolution::Expired { revision: 2 }
        );
    }

    #[test]
    fn duplicate_highest_revision_is_idempotent() {
        let signing_key = key(92);
        let player_id = signing_key.verifying_key().to_bytes();

        let announcement = signed(&signing_key, "Alice", true, 7);

        assert_eq!(
            resolve_player_presence(player_id, &[announcement.clone(), announcement], ISSUED + 1,),
            PresenceResolution::Available {
                player_id,
                display_name: "Alice".to_owned(),
                revision: 7,
                expires_at_unix_seconds: EXPIRES,
            }
        );
    }

    #[test]
    fn contradictory_same_revision_is_equivocation() {
        let signing_key = key(93);
        let player_id = signing_key.verifying_key().to_bytes();

        let available = signed(&signing_key, "Alice", true, 8);
        let unavailable = signed(&signing_key, "Alice", false, 8);

        for announcements in [
            vec![available.clone(), unavailable.clone()],
            vec![unavailable.clone(), available.clone()],
        ] {
            assert_eq!(
                resolve_player_presence(player_id, &announcements, ISSUED + 1,),
                PresenceResolution::Equivocation { revision: 8 }
            );
        }
    }

    #[test]
    fn malformed_forged_record_does_not_poison_valid_presence() {
        let signing_key = key(94);
        let player_id = signing_key.verifying_key().to_bytes();

        let valid = signed(&signing_key, "Alice", true, 3);
        let mut forged = valid.clone();
        forged.body.revision = 999;

        assert!(verify_presence_announcement(&forged).is_err());

        assert_eq!(
            resolve_player_presence(player_id, &[forged, valid], ISSUED + 1,),
            PresenceResolution::Available {
                player_id,
                display_name: "Alice".to_owned(),
                revision: 3,
                expires_at_unix_seconds: EXPIRES,
            }
        );
    }

    #[test]
    fn unrelated_player_announcements_are_ignored() {
        let alice = key(95);
        let bob = key(96);

        let alice_id = alice.verifying_key().to_bytes();

        assert_eq!(
            resolve_player_presence(alice_id, &[signed(&bob, "Bob", true, 50)], ISSUED + 1,),
            PresenceResolution::Absent
        );
    }
}
