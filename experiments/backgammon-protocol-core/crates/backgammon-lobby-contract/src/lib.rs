//! Convergent Freenet lobby state.
//!
//! This crate is intentionally separate from the authoritative per-game
//! action ledger. The first state-model milestone proves deterministic
//! highest-revision presence merging and preservation of same-revision
//! equivocation evidence before Freenet interface plumbing is added.

#![forbid(unsafe_code)]

use backgammon_protocol::{
    verify_presence_announcement, PlayerId, PresenceAnnouncementBody, SignedPresenceAnnouncement,
};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;

/// Two distinct authenticated bodies at one revision are sufficient to prove
/// that a PlayerId equivocated. Additional contradictory bodies do not change
/// that semantic result, so only a deterministic two-record witness is kept.
pub const MAX_EQUIVOCATION_RECORDS: usize = 2;

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct PlayerPresenceState {
    pub player_id: PlayerId,
    pub revision: u64,
    pub records: Vec<SignedPresenceAnnouncement>,
}

#[derive(Serialize, Deserialize, Clone, Default, PartialEq, Eq, Debug)]
pub struct LobbyState {
    /// Canonical strict PlayerId order.
    pub players: Vec<PlayerPresenceState>,
}

fn body_cmp(left: &PresenceAnnouncementBody, right: &PresenceAnnouncementBody) -> Ordering {
    left.protocol_version
        .cmp(&right.protocol_version)
        .then_with(|| left.player_id.cmp(&right.player_id))
        .then_with(|| left.display_name.cmp(&right.display_name))
        .then_with(|| left.available.cmp(&right.available))
        .then_with(|| left.revision.cmp(&right.revision))
        .then_with(|| {
            left.issued_at_unix_seconds
                .cmp(&right.issued_at_unix_seconds)
        })
        .then_with(|| {
            left.expires_at_unix_seconds
                .cmp(&right.expires_at_unix_seconds)
        })
}

fn announcement_cmp(
    left: &SignedPresenceAnnouncement,
    right: &SignedPresenceAnnouncement,
) -> Ordering {
    body_cmp(&left.body, &right.body)
        .then_with(|| left.signature.as_bytes().cmp(right.signature.as_bytes()))
}

/// Produce the unique canonical body witnesses for one PlayerId/revision.
///
/// Records are ordered primarily by body fields. If more than one valid
/// signature representation ever exists for an identical body, the smallest
/// signature bytes provide a deterministic representative; identical bodies
/// themselves are not equivocation.
fn canonical_records(
    mut records: Vec<SignedPresenceAnnouncement>,
) -> Vec<SignedPresenceAnnouncement> {
    records.sort_by(announcement_cmp);

    let mut unique: Vec<SignedPresenceAnnouncement> = Vec::with_capacity(records.len());

    for record in records {
        if unique
            .last()
            .is_some_and(|existing| existing.body == record.body)
        {
            continue;
        }

        unique.push(record);
    }

    unique.truncate(MAX_EQUIVOCATION_RECORDS);
    unique
}

impl PlayerPresenceState {
    pub fn from_announcement(announcement: SignedPresenceAnnouncement) -> Result<Self, String> {
        verify_presence_announcement(&announcement)?;

        Ok(Self {
            player_id: announcement.body.player_id,
            revision: announcement.body.revision,
            records: vec![announcement],
        })
    }

    pub fn is_equivocating(&self) -> bool {
        self.records.len() > 1
    }

    pub fn verify(&self) -> Result<(), String> {
        if self.revision == 0 {
            return Err("Lobby state contains revision zero.".into());
        }

        if self.records.is_empty() {
            return Err("Lobby presence state must retain at least one record.".into());
        }

        if self.records.len() > MAX_EQUIVOCATION_RECORDS {
            return Err("Lobby presence state retains too many equivocation records.".into());
        }

        for record in &self.records {
            verify_presence_announcement(record)?;

            if record.body.player_id != self.player_id {
                return Err("Lobby presence record belongs to a different PlayerId.".into());
            }

            if record.body.revision != self.revision {
                return Err("Lobby presence record belongs to a different revision.".into());
            }
        }

        let canonical = canonical_records(self.records.clone());

        if canonical != self.records {
            return Err("Lobby presence records are not in canonical form.".into());
        }

        Ok(())
    }

    fn merge_from(&mut self, incoming: &Self) -> Result<(), String> {
        self.verify()?;
        incoming.verify()?;

        if self.player_id != incoming.player_id {
            return Err("Cannot merge lobby presence for different PlayerIds.".into());
        }

        match incoming.revision.cmp(&self.revision) {
            Ordering::Greater => {
                *self = incoming.clone();
            }

            Ordering::Less => {}

            Ordering::Equal => {
                let mut combined = self.records.clone();
                combined.extend(incoming.records.iter().cloned());
                self.records = canonical_records(combined);
            }
        }

        self.verify()
    }
}

impl LobbyState {
    pub fn from_announcement(announcement: SignedPresenceAnnouncement) -> Result<Self, String> {
        Ok(Self {
            players: vec![PlayerPresenceState::from_announcement(announcement)?],
        })
    }

    pub fn verify(&self) -> Result<(), String> {
        for player in &self.players {
            player.verify()?;
        }

        for pair in self.players.windows(2) {
            if pair[0].player_id >= pair[1].player_id {
                return Err("Lobby players are not in canonical unique-PlayerId order.".into());
            }
        }

        Ok(())
    }

    /// Associative/commutative/idempotent merge over already-valid lobby
    /// states. Revision numbers, never timestamps, determine replacement.
    pub fn merge_from(&mut self, incoming: &Self) -> Result<(), String> {
        self.verify()?;
        incoming.verify()?;

        for incoming_player in &incoming.players {
            match self
                .players
                .binary_search_by_key(&incoming_player.player_id, |player| player.player_id)
            {
                Ok(index) => {
                    self.players[index].merge_from(incoming_player)?;
                }

                Err(index) => {
                    self.players.insert(index, incoming_player.clone());
                }
            }
        }

        self.verify()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use backgammon_protocol::{sign_presence_announcement, PresenceAnnouncementBody};
    use ed25519_dalek::SigningKey;

    const ISSUED: u64 = 100_000;
    const EXPIRES: u64 = 100_600;

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

    fn state(record: SignedPresenceAnnouncement) -> LobbyState {
        LobbyState::from_announcement(record).unwrap()
    }

    #[test]
    fn higher_revision_wins_in_both_orders() {
        let alice = key(1);

        let older = state(signed(&alice, "Alice", true, 1));
        let newer = state(signed(&alice, "Alice Two", false, 2));

        let mut left = older.clone();
        left.merge_from(&newer).unwrap();

        let mut right = newer.clone();
        right.merge_from(&older).unwrap();

        assert_eq!(left, right);
        assert_eq!(left.players[0].revision, 2);
        assert_eq!(left.players[0].records[0].body.display_name, "Alice Two");
        assert!(!left.players[0].records[0].body.available);
    }

    #[test]
    fn duplicate_state_is_idempotent() {
        let alice = key(2);
        let original = state(signed(&alice, "Alice", true, 7));

        let mut merged = original.clone();
        merged.merge_from(&original).unwrap();

        assert_eq!(merged, original);
        assert_eq!(merged.players[0].records.len(), 1);
    }

    #[test]
    fn same_revision_conflict_converges_to_equivocation() {
        let alice = key(3);

        let available = state(signed(&alice, "Alice", true, 8));
        let unavailable = state(signed(&alice, "Alice", false, 8));

        let mut first = available.clone();
        first.merge_from(&unavailable).unwrap();

        let mut second = unavailable;
        second.merge_from(&available).unwrap();

        assert_eq!(first, second);
        assert!(first.players[0].is_equivocating());
        assert_eq!(first.players[0].records.len(), 2);
    }

    #[test]
    fn three_same_revision_conflicts_keep_deterministic_two_record_witness() {
        let alice = key(4);

        let alpha = state(signed(&alice, "Alpha", true, 9));
        let middle = state(signed(&alice, "Middle", false, 9));
        let zulu = state(signed(&alice, "Zulu", true, 9));

        let mut first = zulu.clone();
        first.merge_from(&middle).unwrap();
        first.merge_from(&alpha).unwrap();

        let mut second = alpha.clone();
        second.merge_from(&zulu).unwrap();
        second.merge_from(&middle).unwrap();

        assert_eq!(first, second);
        assert_eq!(first.players[0].records.len(), 2);

        let names: Vec<_> = first.players[0]
            .records
            .iter()
            .map(|record| record.body.display_name.as_str())
            .collect();

        assert_eq!(names, vec!["Alpha", "Middle"]);
    }

    #[test]
    fn higher_revision_discards_old_equivocation_evidence() {
        let alice = key(5);

        let mut conflicted = state(signed(&alice, "Alice", true, 10));
        conflicted
            .merge_from(&state(signed(&alice, "Alice", false, 10)))
            .unwrap();

        assert!(conflicted.players[0].is_equivocating());

        conflicted
            .merge_from(&state(signed(&alice, "Alice", true, 11)))
            .unwrap();

        assert_eq!(conflicted.players[0].revision, 11);
        assert_eq!(conflicted.players[0].records.len(), 1);
        assert!(!conflicted.players[0].is_equivocating());
    }

    #[test]
    fn multiple_players_converge_to_canonical_player_order() {
        let alice = key(6);
        let bob = key(7);

        let alice_state = state(signed(&alice, "Alice", true, 1));
        let bob_state = state(signed(&bob, "Bob", true, 1));

        let mut first = alice_state.clone();
        first.merge_from(&bob_state).unwrap();

        let mut second = bob_state;
        second.merge_from(&alice_state).unwrap();

        assert_eq!(first, second);
        assert!(first.players[0].player_id < first.players[1].player_id);
    }

    #[test]
    fn forged_record_is_rejected_before_entering_state() {
        let alice = key(8);

        let mut forged = signed(&alice, "Alice", true, 1);
        forged.body.revision = 999;

        assert!(LobbyState::from_announcement(forged).is_err());
    }

    #[test]
    fn noncanonical_record_order_is_rejected() {
        let alice = key(9);

        let mut combined = state(signed(&alice, "Alpha", true, 12));
        combined
            .merge_from(&state(signed(&alice, "Zulu", false, 12)))
            .unwrap();

        combined.players[0].records.reverse();

        assert!(combined.verify().is_err());
    }

    #[test]
    fn noncanonical_player_order_is_rejected() {
        let alice = key(10);
        let bob = key(11);

        let mut combined = state(signed(&alice, "Alice", true, 1));
        combined
            .merge_from(&state(signed(&bob, "Bob", true, 1)))
            .unwrap();

        combined.players.reverse();

        assert!(combined.verify().is_err());
    }

    #[test]
    fn merge_is_associative_across_revision_and_equivocation_updates() {
        let alice = key(12);
        let bob = key(13);

        let a = state(signed(&alice, "Alice", true, 5));
        let b = state(signed(&alice, "Alice", false, 5));
        let c = state(signed(&alice, "Alice New", true, 6));
        let d = state(signed(&bob, "Bob", true, 1));

        let mut left_group = a.clone();
        left_group.merge_from(&b).unwrap();

        let mut right_group = c.clone();
        right_group.merge_from(&d).unwrap();

        let mut left = left_group.clone();
        left.merge_from(&right_group).unwrap();

        let mut right_tail = b.clone();
        right_tail.merge_from(&c).unwrap();
        right_tail.merge_from(&d).unwrap();

        let mut right = a;
        right.merge_from(&right_tail).unwrap();

        assert_eq!(left, right);
        assert_eq!(left.players.len(), 2);

        let alice_state = left
            .players
            .iter()
            .find(|player| player.player_id == alice.verifying_key().to_bytes())
            .unwrap();

        assert_eq!(alice_state.revision, 6);
        assert_eq!(alice_state.records.len(), 1);
        assert_eq!(alice_state.records[0].body.display_name, "Alice New");
        assert!(!alice_state.is_equivocating());
    }
}
