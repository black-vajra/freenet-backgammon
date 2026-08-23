//! Freenet contract adapter for the shared convergent lobby state.

#![forbid(unsafe_code)]

pub use backgammon_lobby_core::*;

use ciborium::{de::from_reader, ser::into_writer};
use freenet_scaffold::ComposableState;
use freenet_stdlib::prelude::*;
use serde::{Deserialize, Serialize};

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
    use backgammon_protocol::{
        sign_presence_announcement, PresenceAnnouncementBody, SignedPresenceAnnouncement,
    };
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
}
