use std::collections::BTreeSet;

use backgammon_protocol::PlayerId;

use crate::lobby::{resolve_player_presence, PresenceResolution, SignedPresenceAnnouncement};

/// One identity that is currently safe to present as challengeable.
///
/// Display names are presentation metadata only. PlayerId remains the
/// authoritative identity, so duplicate display names are intentionally
/// preserved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AvailablePlayer {
    pub player_id: PlayerId,
    pub display_name: String,
    pub revision: u64,
    pub expires_at_unix_seconds: u64,
}

/// Projects an unordered collection of signed presence announcements into the
/// deterministic list of currently available opponents.
///
/// The projection is deliberately transport-independent. Input may arrive in
/// any order and may contain duplicates, stale records, malformed signatures,
/// expired state, or authenticated equivocation.
///
/// The local identity is never returned as an opponent.
pub fn project_available_players(
    local_player_id: PlayerId,
    announcements: &[SignedPresenceAnnouncement],
    now_unix_seconds: u64,
) -> Vec<AvailablePlayer> {
    /*
     * BTreeSet gives us a deterministic set of candidate identities before
     * resolving each identity's complete announcement history.
     */
    let candidate_ids: BTreeSet<PlayerId> = announcements
        .iter()
        .map(|announcement| announcement.body.player_id)
        .filter(|player_id| *player_id != local_player_id)
        .collect();

    let mut players = Vec::new();

    for player_id in candidate_ids {
        let PresenceResolution::Available {
            player_id,
            display_name,
            revision,
            expires_at_unix_seconds,
        } = resolve_player_presence(player_id, announcements, now_unix_seconds)
        else {
            continue;
        };

        players.push(AvailablePlayer {
            player_id,
            display_name,
            revision,
            expires_at_unix_seconds,
        });
    }

    /*
     * User-facing order is deterministic and readable:
     * display name first, PlayerId as the stable tie-breaker.
     *
     * We intentionally do not collapse equal display names.
     */
    players.sort_by(|left, right| {
        left.display_name
            .cmp(&right.display_name)
            .then_with(|| left.player_id.cmp(&right.player_id))
    });

    players
}

#[cfg(test)]
mod tests {
    use ed25519_dalek::SigningKey;

    use super::*;
    use crate::lobby::{sign_presence_announcement, PresenceAnnouncementBody, PresenceSignature};

    const ISSUED: u64 = 90_000;
    const EXPIRES: u64 = 90_600;
    const NOW: u64 = 90_001;

    fn key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn signed(
        signing_key: &SigningKey,
        name: &str,
        available: bool,
        revision: u64,
    ) -> SignedPresenceAnnouncement {
        sign_presence_announcement(
            PresenceAnnouncementBody::new(
                signing_key.verifying_key().to_bytes(),
                name.to_owned(),
                available,
                revision,
                ISSUED,
                EXPIRES,
            ),
            signing_key,
        )
        .unwrap()
    }

    #[test]
    fn live_available_opponents_are_projected() {
        let local = key(1);
        let alice = key(2);
        let bob = key(3);

        let players = project_available_players(
            local.verifying_key().to_bytes(),
            &[
                signed(&alice, "Alice", true, 1),
                signed(&bob, "Bob", true, 1),
            ],
            NOW,
        );

        assert_eq!(players.len(), 2);
        assert_eq!(players[0].display_name, "Alice");
        assert_eq!(players[1].display_name, "Bob");
    }

    #[test]
    fn local_identity_is_never_projected() {
        let local = key(4);

        let players = project_available_players(
            local.verifying_key().to_bytes(),
            &[signed(&local, "Me", true, 1)],
            NOW,
        );

        assert!(players.is_empty());
    }

    #[test]
    fn unavailable_and_expired_players_are_suppressed() {
        let local = key(5);
        let unavailable = key(6);
        let expired = key(7);

        let unavailable_announcement = signed(&unavailable, "Unavailable", false, 1);

        let mut expired_body = PresenceAnnouncementBody::new(
            expired.verifying_key().to_bytes(),
            "Expired".to_owned(),
            true,
            1,
            ISSUED,
            ISSUED + 1,
        );

        let expired_announcement =
            sign_presence_announcement(expired_body.clone(), &expired).unwrap();

        /*
         * Keep this mutable assignment explicit so the fixture documents the
         * exact expiry boundary being tested.
         */
        expired_body.expires_at_unix_seconds = ISSUED + 1;

        let players = project_available_players(
            local.verifying_key().to_bytes(),
            &[unavailable_announcement, expired_announcement],
            ISSUED + 1,
        );

        assert!(players.is_empty());
    }

    #[test]
    fn highest_revision_controls_projection() {
        let local = key(8);
        let alice = key(9);

        let players = project_available_players(
            local.verifying_key().to_bytes(),
            &[
                signed(&alice, "Old Alice", true, 1),
                signed(&alice, "New Alice", true, 2),
            ],
            NOW,
        );

        assert_eq!(
            players,
            vec![AvailablePlayer {
                player_id: alice.verifying_key().to_bytes(),
                display_name: "New Alice".to_owned(),
                revision: 2,
                expires_at_unix_seconds: EXPIRES,
            }]
        );
    }

    #[test]
    fn newer_unavailable_state_removes_old_available_state() {
        let local = key(10);
        let alice = key(11);

        let players = project_available_players(
            local.verifying_key().to_bytes(),
            &[
                signed(&alice, "Alice", true, 1),
                signed(&alice, "Alice", false, 2),
            ],
            NOW,
        );

        assert!(players.is_empty());
    }

    #[test]
    fn authenticated_equivocation_is_suppressed() {
        let local = key(12);
        let alice = key(13);

        let players = project_available_players(
            local.verifying_key().to_bytes(),
            &[
                signed(&alice, "Alice", true, 5),
                signed(&alice, "Alice", false, 5),
            ],
            NOW,
        );

        assert!(players.is_empty());
    }

    #[test]
    fn forged_high_revision_does_not_hide_valid_player() {
        let local = key(14);
        let alice = key(15);

        let valid = signed(&alice, "Alice", true, 3);

        let mut forged = valid.clone();
        forged.body.revision = 999;

        let players =
            project_available_players(local.verifying_key().to_bytes(), &[forged, valid], NOW);

        assert_eq!(players.len(), 1);
        assert_eq!(players[0].display_name, "Alice");
        assert_eq!(players[0].revision, 3);
    }

    #[test]
    fn malformed_signature_does_not_poison_other_players() {
        let local = key(16);
        let alice = key(17);
        let bob = key(18);

        let mut malformed = signed(&alice, "Alice", true, 1);
        malformed.signature = PresenceSignature(vec![0_u8; 3]);

        let players = project_available_players(
            local.verifying_key().to_bytes(),
            &[malformed, signed(&bob, "Bob", true, 1)],
            NOW,
        );

        assert_eq!(players.len(), 1);
        assert_eq!(players[0].display_name, "Bob");
    }

    #[test]
    fn delivery_order_does_not_change_projection() {
        let local = key(19);
        let alice = key(20);
        let bob = key(21);

        let alice_old = signed(&alice, "Alice Old", true, 1);
        let alice_new = signed(&alice, "Alice", true, 2);
        let bob_live = signed(&bob, "Bob", true, 4);

        let forward = project_available_players(
            local.verifying_key().to_bytes(),
            &[alice_old.clone(), bob_live.clone(), alice_new.clone()],
            NOW,
        );

        let reverse = project_available_players(
            local.verifying_key().to_bytes(),
            &[alice_new, bob_live, alice_old],
            NOW,
        );

        assert_eq!(forward, reverse);
    }

    #[test]
    fn duplicate_delivery_is_idempotent() {
        let local = key(22);
        let alice = key(23);

        let announcement = signed(&alice, "Alice", true, 7);

        let once = project_available_players(
            local.verifying_key().to_bytes(),
            &[announcement.clone()],
            NOW,
        );

        let duplicated = project_available_players(
            local.verifying_key().to_bytes(),
            &[announcement.clone(), announcement],
            NOW,
        );

        assert_eq!(once, duplicated);
        assert_eq!(once.len(), 1);
    }

    #[test]
    fn duplicate_display_names_remain_distinct_identities() {
        let local = key(24);
        let first = key(25);
        let second = key(26);

        let players = project_available_players(
            local.verifying_key().to_bytes(),
            &[
                signed(&first, "Alex", true, 1),
                signed(&second, "Alex", true, 1),
            ],
            NOW,
        );

        assert_eq!(players.len(), 2);
        assert_eq!(players[0].display_name, "Alex");
        assert_eq!(players[1].display_name, "Alex");
        assert_ne!(players[0].player_id, players[1].player_id);
    }

    #[test]
    fn projection_has_stable_name_then_identity_order() {
        let local = key(27);
        let zed = key(28);
        let alex_high = key(30);
        let alex_low = key(29);

        let players = project_available_players(
            local.verifying_key().to_bytes(),
            &[
                signed(&zed, "Zed", true, 1),
                signed(&alex_high, "Alex", true, 1),
                signed(&alex_low, "Alex", true, 1),
            ],
            NOW,
        );

        assert_eq!(players.len(), 3);
        assert_eq!(players[0].display_name, "Alex");
        assert_eq!(players[1].display_name, "Alex");
        assert_eq!(players[2].display_name, "Zed");

        assert!(players[0].player_id < players[1].player_id);
    }
}
