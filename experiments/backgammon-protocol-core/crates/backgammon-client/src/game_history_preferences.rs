//! Private list preferences, separate from signed games and recovery records.
use std::collections::BTreeSet;
use serde::{Deserialize, Serialize};
use backgammon_protocol::{contract_durable_scope, GameId};

const MAX_SAVED_GAMES: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryAction { Hide, Archive, Restore }

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameHistoryPreferences {
    version: u16,
    player_id: [u8; 32],
    pub hidden: BTreeSet<GameId>,
    pub archived: BTreeSet<GameId>,
}

// Preserve the exact old wire format when checking a legacy record.
#[derive(Serialize, Deserialize)]
struct LegacyPreferences {
    version: u16,
    player_id: [u8; 32],
    hidden: BTreeSet<GameId>,
}

impl GameHistoryPreferences {
    pub fn new(player_id: [u8; 32]) -> Self {
        Self { version: 2, player_id, hidden: BTreeSet::new(), archived: BTreeSet::new() }
    }

    pub fn apply(&mut self, game_id: GameId, action: HistoryAction) -> Result<(), String> {
        // Validate a candidate before replacing the last valid preference set.
        let mut next = self.clone();
        next.hidden.remove(&game_id);
        next.archived.remove(&game_id);
        match action {
            HistoryAction::Hide => { next.hidden.insert(game_id); }
            HistoryAction::Archive => { next.archived.insert(game_id); }
            HistoryAction::Restore => {}
        }
        next.encode()?;
        *self = next;
        Ok(())
    }

    pub fn encode(&self) -> Result<Vec<u8>, String> {
        if self.version != 2 || !self.hidden.is_disjoint(&self.archived) {
            return Err("Invalid game history preferences.".to_owned());
        }
        if self.hidden.len() + self.archived.len() > MAX_SAVED_GAMES {
            return Err("Saved game limit reached; restore a game first.".to_owned());
        }
        let mut bytes = Vec::new();
        ciborium::ser::into_writer(self, &mut bytes).map_err(|e| e.to_string())?;
        if bytes.len() > backgammon_protocol::MAX_DURABLE_VALUE_BYTES {
            return Err("Game history preferences exceed the size limit.".to_owned());
        }
        Ok(bytes)
    }

    pub fn decode(bytes: &[u8], player_id: [u8; 32]) -> Result<Self, String> {
        if bytes.len() > backgammon_protocol::MAX_DURABLE_VALUE_BYTES {
            return Err("Game history preferences exceed the size limit.".to_owned());
        }
        if let Ok(record) = ciborium::de::from_reader::<Self, _>(bytes) {
            if record.version == 2 && record.player_id == player_id && record.encode()? == bytes {
                return Ok(record);
            }
        }
        let old: LegacyPreferences = ciborium::de::from_reader(bytes).map_err(|e| e.to_string())?;
        let mut canonical = Vec::new();
        ciborium::ser::into_writer(&old, &mut canonical).map_err(|e| e.to_string())?;
        if old.version != 1 || old.player_id != player_id || canonical != bytes {
            return Err("Invalid game history preferences for this player.".to_owned());
        }
        let mut record = Self::new(player_id);
        record.hidden = old.hidden;
        record.encode()?;
        Ok(record)
    }
}

/// Keep the original domain so existing hidden records can be read and migrated.
/// The opaque LocalRole slot is separate from every game's recovery scope.
pub fn history_preferences_scope(player_id: [u8; 32], lobby: &str) -> [u8; 32] {
    let player = player_id.iter().map(|b| format!("{b:02x}")).collect::<String>();
    contract_durable_scope(&format!("private-game-history:v1:{player}:{}:{lobby}", lobby.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preferences_are_identity_scoped_and_canonical() {
        let mut record = GameHistoryPreferences::new([1; 32]);
        record.apply([2; 32], HistoryAction::Archive).unwrap();
        let bytes = record.encode().unwrap();
        assert_eq!(GameHistoryPreferences::decode(&bytes, [1; 32]).unwrap(), record);
        assert!(GameHistoryPreferences::decode(&bytes, [3; 32]).is_err());
        let mut trailing = bytes;
        trailing.push(0);
        assert!(GameHistoryPreferences::decode(&trailing, [1; 32]).is_err());
    }

    #[test]
    fn legacy_hidden_records_are_preserved_and_migrate_on_write() {
        let old = LegacyPreferences { version: 1, player_id: [1; 32],
            hidden: BTreeSet::from([[2; 32], [3; 32]]) };
        let mut bytes = Vec::new();
        ciborium::ser::into_writer(&old, &mut bytes).unwrap();
        let mut record = GameHistoryPreferences::decode(&bytes, [1; 32]).unwrap();
        assert_eq!(record.hidden, old.hidden);
        assert!(record.archived.is_empty());
        assert!(GameHistoryPreferences::decode(&bytes, [9; 32]).is_err());
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(GameHistoryPreferences::decode(&trailing, [1; 32]).is_err());
        record.apply([2; 32], HistoryAction::Archive).unwrap();
        assert_eq!(record.hidden, BTreeSet::from([[3; 32]]));
        assert_eq!(record.archived, BTreeSet::from([[2; 32]]));
        let replacement = record.encode().unwrap();
        assert_ne!(replacement, bytes);
        assert_eq!(GameHistoryPreferences::decode(&replacement, [1; 32]).unwrap(), record);
    }

    #[test]
    fn moving_between_lists_and_restoring_is_exclusive_and_idempotent() {
        let mut record = GameHistoryPreferences::new([1; 32]);
        record.apply([2; 32], HistoryAction::Hide).unwrap();
        record.apply([2; 32], HistoryAction::Archive).unwrap();
        let archived = record.encode().unwrap();
        record.apply([2; 32], HistoryAction::Archive).unwrap();
        assert_eq!(record.encode().unwrap(), archived);
        assert!(record.hidden.is_empty());
        record.apply([2; 32], HistoryAction::Restore).unwrap();
        assert!(record.hidden.is_empty() && record.archived.is_empty());
        record.apply([2; 32], HistoryAction::Archive).unwrap();
        record.apply([2; 32], HistoryAction::Hide).unwrap();
        assert!(record.archived.is_empty());
        assert!(record.hidden.contains(&[2; 32]));
    }

    #[test]
    fn invalid_overlap_and_overflow_are_rejected_without_mutating() {
        let mut record = GameHistoryPreferences::new([1; 32]);
        for n in 0..MAX_SAVED_GAMES {
            let mut id = [0; 32];
            id[..8].copy_from_slice(&(n as u64).to_le_bytes());
            record.hidden.insert(id);
        }
        let before = record.clone();
        assert!(record.apply([255; 32], HistoryAction::Archive).is_err());
        assert_eq!(record, before);
        // Moving a saved game at the limit does not consume another slot.
        record.apply([0; 32], HistoryAction::Archive).unwrap();
        record.hidden.insert([0; 32]);
        assert!(record.encode().is_err());
        let mut malformed = Vec::new();
        ciborium::ser::into_writer(&record, &mut malformed).unwrap();
        assert!(GameHistoryPreferences::decode(&malformed, [1; 32]).is_err());
    }

    #[test]
    fn preferences_do_not_share_recovery_or_other_players_scopes() {
        let scope = history_preferences_scope([1; 32], "lobby");
        assert_ne!(scope, history_preferences_scope([2; 32], "lobby"));
        assert_ne!(scope, history_preferences_scope([1; 32], "other"));
        assert_ne!(scope, contract_durable_scope("lobby"));
        assert_ne!(scope, [1; 32]);
    }
}
