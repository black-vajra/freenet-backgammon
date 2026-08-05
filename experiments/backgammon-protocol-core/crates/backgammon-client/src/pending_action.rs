use backgammon_contract::LedgerStateDelta;
use backgammon_protocol::{ActionId, GameActionRecord, GameId, StateHash, GENESIS_STATE_HASH};
use ciborium::{de::from_reader, ser::into_writer};
use serde::{Deserialize, Serialize};

use crate::ledger_codec::decode_verified_ledger;

pub const PENDING_ACTION_VERSION: u16 = 1;
pub const MAX_PENDING_DELTA_BYTES: usize = 64 * 1024;
pub const MAX_CONTRACT_ID_BYTES: usize = 128;

/// Exact browser-owned action material retained until authoritative acceptance.
///
/// The encoded delta is the retry unit. A retry must reuse these exact bytes;
/// it must not regenerate an action ID, secret, payload, sequence, or hash.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingAction {
    pub version: u16,
    pub contract_id: String,
    pub game_id: GameId,
    pub action_id: ActionId,
    pub sequence: u64,
    pub previous_state_hash: StateHash,
    pub resulting_state_hash: StateHash,
    pub delta: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PendingActionResolution {
    /// The authoritative ledger still ends at the exact parent state.
    /// Resubmitting the stored delta byte-for-byte is safe.
    Pending,

    /// The authoritative ledger contains the exact stored action.
    Accepted,
}

impl PendingAction {
    pub fn new(
        contract_id: impl Into<String>,
        record: &GameActionRecord,
        delta: Vec<u8>,
    ) -> Result<Self, String> {
        let pending = Self {
            version: PENDING_ACTION_VERSION,
            contract_id: contract_id.into(),
            game_id: record.game_id,
            action_id: record.action_id,
            sequence: record.sequence,
            previous_state_hash: record.previous_state_hash,
            resulting_state_hash: record.resulting_state_hash,
            delta,
        };

        pending.verify()?;

        Ok(pending)
    }

    /// Verifies that the stored metadata describes the one and only action
    /// inside the exact encoded contract delta.
    pub fn verify(&self) -> Result<GameActionRecord, String> {
        if self.version != PENDING_ACTION_VERSION {
            return Err(format!(
                "Unsupported pending-action version {}; expected {}.",
                self.version, PENDING_ACTION_VERSION,
            ));
        }

        if self.contract_id.is_empty() {
            return Err("Pending action has an empty contract ID.".to_owned());
        }

        if self.contract_id.len() > MAX_CONTRACT_ID_BYTES {
            return Err(format!(
                "Pending contract ID exceeds {} bytes.",
                MAX_CONTRACT_ID_BYTES,
            ));
        }

        if self.delta.is_empty() {
            return Err("Pending action contains an empty delta.".to_owned());
        }

        if self.delta.len() > MAX_PENDING_DELTA_BYTES {
            return Err(format!(
                "Pending action delta exceeds {} bytes.",
                MAX_PENDING_DELTA_BYTES,
            ));
        }

        let decoded: LedgerStateDelta = from_reader(self.delta.as_slice())
            .map_err(|error| format!("Pending delta failed CBOR decoding: {error}"))?;

        let actions = decoded
            .actions
            .ok_or_else(|| "Pending delta contains no action component.".to_owned())?;

        if actions.len() != 1 {
            return Err(format!(
                "Pending delta must contain exactly one action; found {}.",
                actions.len(),
            ));
        }

        let record = actions[0]
            .to_game_action_record()
            .map_err(|error| format!("Pending action failed typed decoding: {error}"))?;

        if record.game_id != self.game_id {
            return Err("Pending action game ID does not match its delta.".to_owned());
        }

        if record.action_id != self.action_id {
            return Err("Pending action ID does not match its delta.".to_owned());
        }

        if record.sequence != self.sequence {
            return Err("Pending action sequence does not match its delta.".to_owned());
        }

        if record.previous_state_hash != self.previous_state_hash {
            return Err("Pending action parent-state hash does not match its delta.".to_owned());
        }

        if record.resulting_state_hash != self.resulting_state_hash {
            return Err("Pending action resulting-state hash does not match its delta.".to_owned());
        }

        Ok(record)
    }

    /// Compares the exact pending action with a complete verified ledger.
    ///
    /// This fails closed if another action already occupies the sequence, if
    /// the ledger has advanced beyond the pending parent, or if the parent hash
    /// no longer matches.
    pub fn reconcile(&self, authoritative_state: &[u8]) -> Result<PendingActionResolution, String> {
        let pending_record = self.verify()?;
        let ledger = decode_verified_ledger(authoritative_state)?;

        if let Some(existing) = ledger
            .typed_actions()
            .iter()
            .find(|record| record.action_id == self.action_id)
        {
            if existing == &pending_record {
                return Ok(PendingActionResolution::Accepted);
            }

            return Err(
                "Authoritative ledger contains conflicting content for the pending action ID."
                    .to_owned(),
            );
        }

        if let Some(existing) = ledger
            .typed_actions()
            .iter()
            .find(|record| record.sequence == self.sequence)
        {
            return Err(format!(
                "Authoritative sequence {} is occupied by action {:02x?}, not the pending action.",
                self.sequence, existing.action_id,
            ));
        }

        let authoritative_next = u64::try_from(ledger.action_count())
            .map_err(|_| "Authoritative action count exceeds u64.".to_owned())?;

        if authoritative_next != self.sequence {
            return Err(format!(
                "Pending action sequence {} no longer extends authoritative sequence {}.",
                self.sequence, authoritative_next,
            ));
        }

        let authoritative_parent_hash = ledger
            .typed_actions()
            .last()
            .map(|record| record.resulting_state_hash)
            .unwrap_or(GENESIS_STATE_HASH);

        if authoritative_parent_hash != self.previous_state_hash {
            return Err(
                "Pending action no longer extends the authoritative state hash.".to_owned(),
            );
        }

        if self.sequence > 0 {
            let game_id = ledger
                .typed_actions()
                .first()
                .map(|record| record.game_id)
                .ok_or_else(|| {
                    "Non-genesis pending action cannot extend an empty ledger.".to_owned()
                })?;

            if game_id != self.game_id {
                return Err(
                    "Pending action game ID differs from the authoritative ledger.".to_owned(),
                );
            }
        }

        Ok(PendingActionResolution::Pending)
    }

    pub fn encode(&self) -> Result<Vec<u8>, String> {
        self.verify()?;

        let mut encoded = Vec::new();

        into_writer(self, &mut encoded)
            .map_err(|error| format!("Could not encode pending action: {error}"))?;

        Ok(encoded)
    }

    pub fn decode(encoded: &[u8]) -> Result<Self, String> {
        let pending: Self = from_reader(encoded)
            .map_err(|error| format!("Could not decode pending action: {error}"))?;

        pending.verify()?;

        let canonical = pending.encode()?;

        if canonical != encoded {
            return Err("Pending action is not canonically encoded.".to_owned());
        }

        Ok(pending)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use backgammon_contract::{LedgerState, LedgerStateDelta};
    use backgammon_core::Player;
    use backgammon_protocol::GameActionPayload;
    use ciborium::{de::from_reader, ser::into_writer};

    use crate::ledger_codec::build_encoded_action_delta;

    const ONE_ACTION_STATE: &[u8] = include_bytes!("../fixtures/expected-one-action-state.cbor");

    fn pending_resignation(action_id: ActionId) -> PendingAction {
        let (record, delta) = build_encoded_action_delta(
            ONE_ACTION_STATE,
            action_id,
            GameActionPayload::Resign {
                player: Player::White,
            },
        )
        .unwrap();

        PendingAction::new("test-contract", &record, delta).unwrap()
    }

    fn state_with_delta(state_bytes: &[u8], delta_bytes: &[u8]) -> Vec<u8> {
        let mut state: LedgerState = from_reader(state_bytes).unwrap();
        let delta: LedgerStateDelta = from_reader(delta_bytes).unwrap();

        state
            .actions
            .0
            .extend(delta.actions.expect("delta must contain actions"));

        state
            .actions
            .0
            .sort_by(|left, right| left.id.cmp(&right.id));

        let mut encoded = Vec::new();
        into_writer(&state, &mut encoded).unwrap();
        encoded
    }

    #[test]
    fn pending_record_round_trips_canonically() {
        let pending = pending_resignation([42_u8; 32]);
        let encoded = pending.encode().unwrap();
        let decoded = PendingAction::decode(&encoded).unwrap();

        assert_eq!(decoded, pending);
    }

    #[test]
    fn exact_delta_metadata_is_verified() {
        let pending = pending_resignation([42_u8; 32]);
        let record = pending.verify().unwrap();

        assert_eq!(record.action_id, pending.action_id);
        assert_eq!(record.sequence, pending.sequence);
        assert_eq!(record.game_id, pending.game_id);
        assert_eq!(record.previous_state_hash, pending.previous_state_hash,);
        assert_eq!(record.resulting_state_hash, pending.resulting_state_hash,);
    }

    #[test]
    fn action_is_pending_while_authoritative_parent_is_unchanged() {
        let pending = pending_resignation([42_u8; 32]);

        assert_eq!(
            pending.reconcile(ONE_ACTION_STATE),
            Ok(PendingActionResolution::Pending),
        );
    }

    #[test]
    fn exact_authoritative_action_resolves_as_accepted() {
        let pending = pending_resignation([42_u8; 32]);
        let accepted_state = state_with_delta(ONE_ACTION_STATE, &pending.delta);

        assert_eq!(
            pending.reconcile(&accepted_state),
            Ok(PendingActionResolution::Accepted),
        );
    }

    #[test]
    fn different_action_at_pending_sequence_is_rejected() {
        let pending = pending_resignation([42_u8; 32]);
        let competing = pending_resignation([43_u8; 32]);

        let competing_state = state_with_delta(ONE_ACTION_STATE, &competing.delta);

        assert!(pending.reconcile(&competing_state).is_err());
    }

    #[test]
    fn tampered_pending_metadata_is_rejected() {
        let mut pending = pending_resignation([42_u8; 32]);
        pending.action_id = [99_u8; 32];

        assert!(pending.verify().is_err());
        assert!(pending.encode().is_err());
    }

    #[test]
    fn malformed_delta_is_rejected() {
        let mut pending = pending_resignation([42_u8; 32]);
        pending.delta = vec![0xff, 0x00];

        assert!(pending.verify().is_err());
    }

    #[test]
    fn empty_contract_id_is_rejected() {
        let mut pending = pending_resignation([42_u8; 32]);
        pending.contract_id.clear();

        assert!(pending.verify().is_err());
    }
}
