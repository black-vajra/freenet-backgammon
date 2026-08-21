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
use ciborium::{de::from_reader, ser::into_writer};
use freenet_scaffold_macro::composable;
use freenet_stdlib::prelude::*;
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

#[derive(Serialize, Deserialize, Clone, Default, PartialEq, Eq, Debug)]
pub struct LobbyEntries(pub LobbyState);

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct LobbyPlayerSummary {
    pub player_id: PlayerId,
    pub revision: u64,
    /// Canonical retained authenticated bodies at this revision.
    ///
    /// Signatures are not needed in a summary. Distinct bodies are needed so
    /// equal-revision equivocation evidence can still be synchronized.
    pub bodies: Vec<PresenceAnnouncementBody>,
}

#[derive(Serialize, Deserialize, Clone, Default, PartialEq, Eq, Debug)]
pub struct LobbyEntriesSummary(pub Vec<LobbyPlayerSummary>);

impl LobbyEntriesSummary {
    pub fn verify(&self) -> Result<(), String> {
        for player in &self.0 {
            if player.revision == 0 {
                return Err("Lobby summary contains revision zero.".into());
            }

            if player.bodies.is_empty() {
                return Err("Lobby summary must retain at least one body.".into());
            }

            if player.bodies.len() > MAX_EQUIVOCATION_RECORDS {
                return Err("Lobby summary retains too many equivocation bodies.".into());
            }

            for body in &player.bodies {
                if body.player_id != player.player_id {
                    return Err("Lobby summary body belongs to a different PlayerId.".into());
                }

                if body.revision != player.revision {
                    return Err("Lobby summary body belongs to a different revision.".into());
                }
            }

            for pair in player.bodies.windows(2) {
                if body_cmp(&pair[0], &pair[1]) != Ordering::Less {
                    return Err("Lobby summary bodies are not in canonical order.".into());
                }
            }
        }

        for pair in self.0.windows(2) {
            if pair[0].player_id >= pair[1].player_id {
                return Err(
                    "Lobby summary players are not in canonical unique-PlayerId order.".into(),
                );
            }
        }

        Ok(())
    }
}

impl ComposableState for LobbyEntries {
    type ParentState = LobbyContractState;
    type Summary = LobbyEntriesSummary;
    type Delta = LobbyState;
    type Parameters = ();

    fn verify(
        &self,
        _parent: &Self::ParentState,
        _parameters: &Self::Parameters,
    ) -> Result<(), String> {
        self.0.verify()
    }

    fn summarize(
        &self,
        _parent: &Self::ParentState,
        _parameters: &Self::Parameters,
    ) -> Self::Summary {
        LobbyEntriesSummary(
            self.0
                .players
                .iter()
                .map(|player| LobbyPlayerSummary {
                    player_id: player.player_id,
                    revision: player.revision,
                    bodies: player
                        .records
                        .iter()
                        .map(|record| record.body.clone())
                        .collect(),
                })
                .collect(),
        )
    }

    fn delta(
        &self,
        _parent: &Self::ParentState,
        _parameters: &Self::Parameters,
        old: &Self::Summary,
    ) -> Option<Self::Delta> {
        let mut changed = Vec::new();

        for player in &self.0.players {
            let include = match old
                .0
                .binary_search_by_key(&player.player_id, |summary| summary.player_id)
            {
                Err(_) => true,

                Ok(index) => {
                    let previous = &old.0[index];

                    if player.revision > previous.revision {
                        true
                    } else if player.revision < previous.revision {
                        false
                    } else {
                        let bodies: Vec<_> = player
                            .records
                            .iter()
                            .map(|record| record.body.clone())
                            .collect();

                        bodies != previous.bodies
                    }
                }
            };

            if include {
                changed.push(player.clone());
            }
        }

        (!changed.is_empty()).then_some(LobbyState { players: changed })
    }

    fn apply_delta(
        &mut self,
        _parent: &Self::ParentState,
        _parameters: &Self::Parameters,
        delta: &Option<Self::Delta>,
    ) -> Result<(), String> {
        if let Some(incoming) = delta {
            self.0.merge_from(incoming)?;
        }

        self.0.verify()
    }
}

#[composable]
#[derive(Serialize, Deserialize, Clone, Default, PartialEq, Eq, Debug)]
pub struct LobbyContractState {
    pub lobby: LobbyEntries,
}

struct Contract;

#[derive(Clone, Debug, PartialEq)]
enum DecodedUpdate {
    Delta(LobbyContractStateDelta),
    State(LobbyContractState),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ApplyUpdatesError {
    InvalidUpdate,
    InvalidState,
}

fn apply_decoded_updates(
    mut current: LobbyContractState,
    updates: impl IntoIterator<Item = DecodedUpdate>,
) -> Result<LobbyContractState, ApplyUpdatesError> {
    for update in updates {
        match update {
            DecodedUpdate::Delta(delta) => {
                let parent = current.clone();

                current
                    .apply_delta(&parent, &(), &Some(delta))
                    .map_err(|_| ApplyUpdatesError::InvalidUpdate)?;
            }

            DecodedUpdate::State(incoming) => {
                let parent = current.clone();

                current
                    .merge(&parent, &(), &incoming)
                    .map_err(|_| ApplyUpdatesError::InvalidUpdate)?;
            }
        }
    }

    current
        .verify(&current, &())
        .map_err(|_| ApplyUpdatesError::InvalidState)?;

    Ok(current)
}

fn decode<T: for<'de> Deserialize<'de>>(bytes: &[u8]) -> Result<T, ContractError> {
    from_reader(bytes).map_err(|error| ContractError::Deser(error.to_string()))
}

fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, ContractError> {
    let mut output = Vec::new();
    into_writer(value, &mut output).map_err(|error| ContractError::Deser(error.to_string()))?;
    Ok(output)
}

fn decode_parameters(parameters: Parameters<'_>) -> Result<(), ContractError> {
    decode(parameters.as_ref())
}

#[contract]
impl ContractInterface for Contract {
    fn validate_state(
        parameters: Parameters<'static>,
        state: State<'static>,
        _related: RelatedContracts<'static>,
    ) -> Result<ValidateResult, ContractError> {
        decode_parameters(parameters)?;

        if state.as_ref().is_empty() {
            return Ok(ValidateResult::Valid);
        }

        let state: LobbyContractState = decode(state.as_ref())?;

        state
            .verify(&state, &())
            .map(|_| ValidateResult::Valid)
            .map_err(|_| ContractError::InvalidState)
    }

    fn update_state(
        parameters: Parameters<'static>,
        state: State<'static>,
        data: Vec<UpdateData<'static>>,
    ) -> Result<UpdateModification<'static>, ContractError> {
        decode_parameters(parameters)?;

        let current: LobbyContractState = decode(state.as_ref())?;
        let mut updates = Vec::with_capacity(data.len());

        for update in data {
            match update {
                UpdateData::Delta(bytes) => {
                    updates.push(DecodedUpdate::Delta(decode(bytes.as_ref())?));
                }

                UpdateData::State(bytes) => {
                    updates.push(DecodedUpdate::State(decode(bytes.as_ref())?));
                }

                _ => return Err(ContractError::InvalidUpdate),
            }
        }

        let current = apply_decoded_updates(current, updates).map_err(|error| match error {
            ApplyUpdatesError::InvalidUpdate => ContractError::InvalidUpdate,
            ApplyUpdatesError::InvalidState => ContractError::InvalidState,
        })?;

        Ok(UpdateModification::valid(State::from(encode(&current)?)))
    }

    fn summarize_state(
        parameters: Parameters<'static>,
        state: State<'static>,
    ) -> Result<StateSummary<'static>, ContractError> {
        decode_parameters(parameters)?;

        if state.as_ref().is_empty() {
            return Ok(StateSummary::from(Vec::new()));
        }

        let state: LobbyContractState = decode(state.as_ref())?;
        state
            .verify(&state, &())
            .map_err(|_| ContractError::InvalidState)?;

        Ok(StateSummary::from(encode(&state.summarize(&state, &()))?))
    }

    fn get_state_delta(
        parameters: Parameters<'static>,
        state: State<'static>,
        summary: StateSummary<'static>,
    ) -> Result<StateDelta<'static>, ContractError> {
        decode_parameters(parameters)?;

        let state: LobbyContractState = decode(state.as_ref())?;
        state
            .verify(&state, &())
            .map_err(|_| ContractError::InvalidState)?;

        let summary: LobbyContractStateSummary = decode(summary.as_ref())?;

        Ok(StateDelta::from(encode(&state.delta(
            &state,
            &(),
            &summary,
        ))?))
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

    fn contract_parameters() -> Parameters<'static> {
        Parameters::from(encoded(&()))
    }

    fn encoded<T: Serialize>(value: &T) -> Vec<u8> {
        encode(value).unwrap()
    }

    fn contract_state(state: &LobbyContractState) -> State<'static> {
        State::from(encoded(state))
    }

    #[test]
    fn contract_validate_state_accepts_valid_lobby_state() {
        let alice = key(30);
        let lobby = LobbyContractState {
            lobby: LobbyEntries(state(signed(&alice, "Alice", true, 1))),
        };

        assert_eq!(
            Contract::validate_state(
                contract_parameters(),
                contract_state(&lobby),
                RelatedContracts::new(),
            )
            .unwrap(),
            ValidateResult::Valid
        );
    }

    #[test]
    fn contract_validate_state_rejects_forged_presence() {
        let alice = key(31);
        let mut forged = signed(&alice, "Alice", true, 1);
        forged.body.revision = 99;

        let lobby = LobbyContractState {
            lobby: LobbyEntries(LobbyState {
                players: vec![PlayerPresenceState {
                    player_id: forged.body.player_id,
                    revision: forged.body.revision,
                    records: vec![forged],
                }],
            }),
        };

        assert!(Contract::validate_state(
            contract_parameters(),
            contract_state(&lobby),
            RelatedContracts::new(),
        )
        .is_err());
    }

    #[test]
    fn contract_validate_state_rejects_malformed_cbor() {
        assert!(Contract::validate_state(
            contract_parameters(),
            State::from(vec![0x9f, 0x01]),
            RelatedContracts::new(),
        )
        .is_err());
    }

    #[test]
    fn contract_state_update_merges_valid_presence() {
        let alice = key(32);

        let current = LobbyContractState::default();
        let incoming = LobbyContractState {
            lobby: LobbyEntries(state(signed(&alice, "Alice", true, 1))),
        };

        let modification = Contract::update_state(
            contract_parameters(),
            contract_state(&current),
            vec![UpdateData::State(contract_state(&incoming))],
        )
        .unwrap();

        let result: LobbyContractState = decode(modification.unwrap_valid().as_ref()).unwrap();

        assert_eq!(result, incoming);
    }

    #[test]
    fn contract_delta_update_merges_valid_presence() {
        let alice = key(33);

        let current = LobbyContractState::default();
        let incoming = LobbyContractState {
            lobby: LobbyEntries(state(signed(&alice, "Alice", true, 2))),
        };

        let parent = incoming.clone();
        let empty_summary =
            LobbyContractState::default().summarize(&LobbyContractState::default(), &());

        let delta = incoming
            .delta(&parent, &(), &empty_summary)
            .expect("incoming presence must produce a delta");

        let modification = Contract::update_state(
            contract_parameters(),
            contract_state(&current),
            vec![UpdateData::Delta(StateDelta::from(encoded(&delta)))],
        )
        .unwrap();

        let result: LobbyContractState = decode(modification.unwrap_valid().as_ref()).unwrap();

        assert_eq!(result, incoming);
    }

    #[test]
    fn contract_summary_and_delta_sync_equivocation_evidence() {
        let alice = key(34);

        let receiver = LobbyContractState {
            lobby: LobbyEntries(state(signed(&alice, "Alice", true, 3))),
        };

        let mut source_state = state(signed(&alice, "Alice", true, 3));
        source_state
            .merge_from(&state(signed(&alice, "Alice", false, 3)))
            .unwrap();

        let source = LobbyContractState {
            lobby: LobbyEntries(source_state),
        };

        let summary =
            Contract::summarize_state(contract_parameters(), contract_state(&receiver)).unwrap();

        let delta =
            Contract::get_state_delta(contract_parameters(), contract_state(&source), summary)
                .unwrap();

        let modification = Contract::update_state(
            contract_parameters(),
            contract_state(&receiver),
            vec![UpdateData::Delta(delta)],
        )
        .unwrap();

        let result: LobbyContractState = decode(modification.unwrap_valid().as_ref()).unwrap();

        assert_eq!(result, source);
        assert!(result.lobby.0.players[0].is_equivocating());
    }

    #[test]
    fn opposite_contract_delta_orders_converge() {
        let alice = key(35);

        let first = LobbyContractState {
            lobby: LobbyEntries(state(signed(&alice, "Alice", true, 4))),
        };

        let second = LobbyContractState {
            lobby: LobbyEntries(state(signed(&alice, "Alice", false, 4))),
        };

        let empty = LobbyContractState::default();
        let empty_summary = empty.summarize(&empty, &());

        let first_delta = first.delta(&first, &(), &empty_summary).unwrap();
        let second_delta = second.delta(&second, &(), &empty_summary).unwrap();

        let left = Contract::update_state(
            contract_parameters(),
            contract_state(&empty),
            vec![
                UpdateData::Delta(StateDelta::from(encoded(&first_delta))),
                UpdateData::Delta(StateDelta::from(encoded(&second_delta))),
            ],
        )
        .unwrap();

        let right = Contract::update_state(
            contract_parameters(),
            contract_state(&empty),
            vec![
                UpdateData::Delta(StateDelta::from(encoded(&second_delta))),
                UpdateData::Delta(StateDelta::from(encoded(&first_delta))),
            ],
        )
        .unwrap();

        let left: LobbyContractState = decode(left.unwrap_valid().as_ref()).unwrap();
        let right: LobbyContractState = decode(right.unwrap_valid().as_ref()).unwrap();

        assert_eq!(left, right);
        assert!(left.lobby.0.players[0].is_equivocating());
    }

    #[test]
    fn current_summary_produces_no_delta() {
        let alice = key(20);
        let entries = LobbyEntries(state(signed(&alice, "Alice", true, 3)));
        let parent = LobbyContractState {
            lobby: entries.clone(),
        };

        let summary = entries.summarize(&parent, &());

        assert!(summary.verify().is_ok());
        assert!(entries.delta(&parent, &(), &summary).is_none());
    }

    #[test]
    fn missing_player_is_returned_in_delta() {
        let alice = key(21);
        let entries = LobbyEntries(state(signed(&alice, "Alice", true, 4)));
        let parent = LobbyContractState {
            lobby: entries.clone(),
        };

        let delta = entries
            .delta(&parent, &(), &LobbyEntriesSummary::default())
            .unwrap();

        assert_eq!(delta, entries.0);
    }

    #[test]
    fn newer_revision_is_returned_in_delta() {
        let alice = key(22);

        let old_entries = LobbyEntries(state(signed(&alice, "Alice", true, 5)));
        let old_parent = LobbyContractState {
            lobby: old_entries.clone(),
        };
        let old_summary = old_entries.summarize(&old_parent, &());

        let new_entries = LobbyEntries(state(signed(&alice, "Alice", false, 6)));
        let new_parent = LobbyContractState {
            lobby: new_entries.clone(),
        };

        let delta = new_entries.delta(&new_parent, &(), &old_summary).unwrap();

        assert_eq!(delta.players.len(), 1);
        assert_eq!(delta.players[0].revision, 6);
    }

    #[test]
    fn same_revision_equivocation_evidence_is_returned_in_delta() {
        let alice = key(23);

        let one = LobbyEntries(state(signed(&alice, "Alice", true, 7)));
        let one_parent = LobbyContractState { lobby: one.clone() };
        let one_summary = one.summarize(&one_parent, &());

        let mut conflicted_state = state(signed(&alice, "Alice", true, 7));
        conflicted_state
            .merge_from(&state(signed(&alice, "Alice", false, 7)))
            .unwrap();

        let conflicted = LobbyEntries(conflicted_state);
        let conflicted_parent = LobbyContractState {
            lobby: conflicted.clone(),
        };

        let delta = conflicted
            .delta(&conflicted_parent, &(), &one_summary)
            .unwrap();

        assert_eq!(delta.players.len(), 1);
        assert_eq!(delta.players[0].records.len(), 2);
        assert!(delta.players[0].is_equivocating());
    }

    #[test]
    fn composable_delta_application_is_idempotent() {
        let alice = key(24);

        let source = LobbyEntries(state(signed(&alice, "Alice", true, 8)));
        let source_parent = LobbyContractState {
            lobby: source.clone(),
        };

        let empty = LobbyEntries::default();
        let empty_parent = LobbyContractState {
            lobby: empty.clone(),
        };
        let empty_summary = empty.summarize(&empty_parent, &());

        let delta = source.delta(&source_parent, &(), &empty_summary);

        let mut target = LobbyEntries::default();
        let target_parent = LobbyContractState {
            lobby: target.clone(),
        };

        target.apply_delta(&target_parent, &(), &delta).unwrap();

        let once = target.clone();

        let target_parent = LobbyContractState {
            lobby: target.clone(),
        };
        target.apply_delta(&target_parent, &(), &delta).unwrap();

        assert_eq!(target, once);
        assert_eq!(target, source);
    }

    #[test]
    fn opposite_composable_delta_orders_converge() {
        let alice = key(25);

        let first = LobbyEntries(state(signed(&alice, "Alice", true, 9)));
        let second = LobbyEntries(state(signed(&alice, "Alice", false, 9)));

        let empty = LobbyEntries::default();
        let empty_parent = LobbyContractState {
            lobby: empty.clone(),
        };
        let empty_summary = empty.summarize(&empty_parent, &());

        let first_parent = LobbyContractState {
            lobby: first.clone(),
        };
        let second_parent = LobbyContractState {
            lobby: second.clone(),
        };

        let first_delta = first.delta(&first_parent, &(), &empty_summary);
        let second_delta = second.delta(&second_parent, &(), &empty_summary);

        let mut left = LobbyEntries::default();
        let parent = LobbyContractState {
            lobby: left.clone(),
        };
        left.apply_delta(&parent, &(), &first_delta).unwrap();

        let parent = LobbyContractState {
            lobby: left.clone(),
        };
        left.apply_delta(&parent, &(), &second_delta).unwrap();

        let mut right = LobbyEntries::default();
        let parent = LobbyContractState {
            lobby: right.clone(),
        };
        right.apply_delta(&parent, &(), &second_delta).unwrap();

        let parent = LobbyContractState {
            lobby: right.clone(),
        };
        right.apply_delta(&parent, &(), &first_delta).unwrap();

        assert_eq!(left, right);
        assert!(left.0.players[0].is_equivocating());
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
