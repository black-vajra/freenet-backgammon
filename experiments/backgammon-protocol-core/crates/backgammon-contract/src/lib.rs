use backgammon_protocol::{verify_action_sequences, Action, LedgerParameters, PROTOCOL_VERSION};
use ciborium::{de::from_reader, ser::into_writer};
use freenet_scaffold_macro::composable;
use freenet_stdlib::prelude::*;
use serde::{Deserialize, Serialize};

const MAX_ACTIONS: usize = 256;
const MAX_PAYLOAD_BYTES: usize = 1024;

#[derive(Serialize, Deserialize, Clone, Default, PartialEq, Eq, Debug)]
pub struct Actions(pub Vec<Action>);

impl Actions {
    fn canonicalize(&mut self) {
        self.0.sort_by(|a, b| a.id.cmp(&b.id));
        self.0.dedup();
    }

    fn verify_inner(&self) -> Result<(), String> {
        if self.0.len() > MAX_ACTIONS {
            return Err("action limit exceeded".into());
        }
        for pair in self.0.windows(2) {
            if pair[0].id >= pair[1].id {
                return Err("actions are not in canonical unique-ID order".into());
            }
        }
        if self.0.iter().any(|a| a.payload.len() > MAX_PAYLOAD_BYTES) {
            return Err("action payload limit exceeded".into());
        }
        Ok(())
    }
}

impl ComposableState for Actions {
    type ParentState = LedgerState;
    type Summary = Vec<[u8; 32]>;
    type Delta = Vec<Action>;
    type Parameters = LedgerParameters;

    fn verify(
        &self,
        _parent: &Self::ParentState,
        parameters: &Self::Parameters,
    ) -> Result<(), String> {
        parameters.verify()?;
        self.verify_inner()?;
        verify_action_sequences(&self.0)
    }

    fn summarize(
        &self,
        _parent: &Self::ParentState,
        _parameters: &Self::Parameters,
    ) -> Self::Summary {
        self.0.iter().map(|a| a.id).collect()
    }

    fn delta(
        &self,
        _parent: &Self::ParentState,
        _parameters: &Self::Parameters,
        old: &Self::Summary,
    ) -> Option<Self::Delta> {
        let missing: Vec<_> = self
            .0
            .iter()
            .filter(|a| old.binary_search(&a.id).is_err())
            .cloned()
            .collect();
        (!missing.is_empty()).then_some(missing)
    }

    fn apply_delta(
        &mut self,
        _parent: &Self::ParentState,
        _parameters: &Self::Parameters,
        delta: &Option<Self::Delta>,
    ) -> Result<(), String> {
        if let Some(incoming) = delta {
            for action in incoming {
                if let Some(existing) = self.0.iter().find(|a| a.id == action.id) {
                    if existing != action {
                        return Err("conflicting actions share an ID".into());
                    }
                } else {
                    self.0.push(action.clone());
                }
            }
            self.canonicalize();
            self.verify_inner()?;
        }
        Ok(())
    }
}

#[composable]
#[derive(Serialize, Deserialize, Clone, Default, PartialEq, Eq, Debug)]
pub struct LedgerState {
    pub actions: Actions,
}

struct Contract;

fn decode<T: for<'de> Deserialize<'de>>(bytes: &[u8]) -> Result<T, ContractError> {
    from_reader(bytes).map_err(|e| ContractError::Deser(e.to_string()))
}

fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, ContractError> {
    let mut output = Vec::new();
    into_writer(value, &mut output).map_err(|e| ContractError::Deser(e.to_string()))?;
    Ok(output)
}

#[contract]
impl ContractInterface for Contract {
    fn validate_state(
        parameters: Parameters<'static>,
        state: State<'static>,
        _related: RelatedContracts<'static>,
    ) -> Result<ValidateResult, ContractError> {
        if state.as_ref().is_empty() {
            return Ok(ValidateResult::Valid);
        }
        let parameters: LedgerParameters = decode(parameters.as_ref())?;
        let state: LedgerState = decode(state.as_ref())?;
        state
            .verify(&state, &parameters)
            .map(|_| ValidateResult::Valid)
            .map_err(|_| ContractError::InvalidState)
    }

    fn update_state(
        parameters: Parameters<'static>,
        state: State<'static>,
        data: Vec<UpdateData<'static>>,
    ) -> Result<UpdateModification<'static>, ContractError> {
        let parameters: LedgerParameters = decode(parameters.as_ref())?;
        let mut current: LedgerState = decode(state.as_ref())?;
        for update in data {
            match update {
                UpdateData::Delta(bytes) => {
                    let delta: LedgerStateDelta = decode(bytes.as_ref())?;
                    let parent = current.clone();
                    current
                        .apply_delta(&parent, &parameters, &Some(delta))
                        .map_err(|_| ContractError::InvalidUpdate)?;
                }
                UpdateData::State(bytes) => {
                    let incoming: LedgerState = decode(bytes.as_ref())?;
                    let parent = current.clone();
                    current
                        .merge(&parent, &parameters, &incoming)
                        .map_err(|_| ContractError::InvalidUpdate)?;
                }
                _ => return Err(ContractError::InvalidUpdate),
            }
        }
        current
            .verify(&current, &parameters)
            .map_err(|_| ContractError::InvalidState)?;
        Ok(UpdateModification::valid(encode(&current)?.into()))
    }

    fn summarize_state(
        parameters: Parameters<'static>,
        state: State<'static>,
    ) -> Result<StateSummary<'static>, ContractError> {
        if state.as_ref().is_empty() {
            return Ok(StateSummary::from(Vec::new()));
        }
        let parameters: LedgerParameters = decode(parameters.as_ref())?;
        let state: LedgerState = decode(state.as_ref())?;
        Ok(StateSummary::from(encode(
            &state.summarize(&state, &parameters),
        )?))
    }

    fn get_state_delta(
        parameters: Parameters<'static>,
        state: State<'static>,
        summary: StateSummary<'static>,
    ) -> Result<StateDelta<'static>, ContractError> {
        let parameters: LedgerParameters = decode(parameters.as_ref())?;
        let state: LedgerState = decode(state.as_ref())?;
        let summary: LedgerStateSummary = decode(summary.as_ref())?;
        Ok(StateDelta::from(encode(&state.delta(
            &state,
            &parameters,
            &summary,
        ))?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> LedgerParameters {
        LedgerParameters {
            protocol_version: PROTOCOL_VERSION,
        }
    }

    fn action(id: u8, sequence: u32) -> Action {
        Action {
            id: [id; 32],
            sequence,
            payload: vec![id],
        }
    }

    fn encoded<T: Serialize>(value: &T) -> Vec<u8> {
        encode(value).unwrap()
    }

    #[test]
    fn opposite_update_orders_converge() {
        let p = params();
        let empty = LedgerState::default();
        let da = LedgerStateDelta {
            actions: Some(vec![action(1, 0)]),
        };
        let db = LedgerStateDelta {
            actions: Some(vec![action(2, 1)]),
        };
        let mut ab = empty.clone();
        ab.apply_delta(&ab.clone(), &p, &Some(da.clone())).unwrap();
        ab.apply_delta(&ab.clone(), &p, &Some(db.clone())).unwrap();
        let mut ba = empty;
        ba.apply_delta(&ba.clone(), &p, &Some(db)).unwrap();
        ba.apply_delta(&ba.clone(), &p, &Some(da)).unwrap();
        assert_eq!(ab, ba);
    }

    #[test]
    fn duplicate_is_idempotent() {
        let p = params();
        let d = LedgerStateDelta {
            actions: Some(vec![action(1, 0)]),
        };
        let mut state = LedgerState::default();
        state
            .apply_delta(&state.clone(), &p, &Some(d.clone()))
            .unwrap();
        let once = state.clone();
        state.apply_delta(&state.clone(), &p, &Some(d)).unwrap();
        assert_eq!(state, once);
    }

    #[test]
    fn same_id_with_different_content_is_rejected() {
        let p = params();
        let mut state = LedgerState::default();
        state
            .apply_delta(
                &state.clone(),
                &p,
                &Some(LedgerStateDelta {
                    actions: Some(vec![action(1, 0)]),
                }),
            )
            .unwrap();
        let mut conflicting = action(1, 99);
        conflicting.payload = vec![9];
        assert!(state
            .apply_delta(
                &state.clone(),
                &p,
                &Some(LedgerStateDelta {
                    actions: Some(vec![conflicting])
                }),
            )
            .is_err());
    }

    #[test]
    fn malformed_cbor_is_rejected() {
        let malformed = [0x9f, 0x01];
        assert!(decode::<LedgerState>(&malformed).is_err());
    }

    #[test]
    fn unsupported_protocol_version_is_rejected() {
        let state = LedgerState::default();
        let unsupported = LedgerParameters {
            protocol_version: PROTOCOL_VERSION + 1,
        };
        assert_eq!(
            state.verify(&state, &unsupported),
            Err("unsupported protocol version".into())
        );
    }

    #[test]
    fn final_state_rejects_sequence_starting_after_zero() {
        let state = LedgerState {
            actions: Actions(vec![action(1, 1)]),
        };

        assert_eq!(
            state.verify(&state, &params()),
            Err("action sequence gap".into())
        );
    }

    #[test]
    fn final_state_rejects_duplicate_sequences() {
        let state = LedgerState {
            actions: Actions(vec![action(1, 0), action(2, 0)]),
        };

        assert_eq!(
            state.verify(&state, &params()),
            Err("duplicate action sequence".into())
        );
    }

    #[test]
    fn final_state_rejects_sequence_gaps() {
        let state = LedgerState {
            actions: Actions(vec![action(1, 0), action(3, 2)]),
        };

        assert_eq!(
            state.verify(&state, &params()),
            Err("action sequence gap".into())
        );
    }

    #[test]
    fn final_state_accepts_contiguous_sequences_in_id_order() {
        let state = LedgerState {
            actions: Actions(vec![action(1, 1), action(2, 0)]),
        };

        assert_eq!(state.verify(&state, &params()), Ok(()));
    }

    #[test]
    fn oversized_payload_is_rejected() {
        let mut state = LedgerState::default();
        let mut oversized = action(1, 0);
        oversized.payload = vec![0; MAX_PAYLOAD_BYTES + 1];
        state.actions.0.push(oversized);
        assert_eq!(
            state.verify(&state, &params()),
            Err("action payload limit exceeded".into())
        );
    }

    #[test]
    fn oversized_ledger_is_rejected() {
        let mut state = LedgerState::default();
        for n in 0..=MAX_ACTIONS {
            let mut id = [0_u8; 32];
            id[28..].copy_from_slice(&(n as u32).to_be_bytes());
            state.actions.0.push(Action {
                id,
                sequence: n as u32,
                payload: Vec::new(),
            });
        }
        assert_eq!(
            state.verify(&state, &params()),
            Err("action limit exceeded".into())
        );
    }

    #[test]
    fn noncanonical_state_is_rejected() {
        let state = LedgerState {
            actions: Actions(vec![action(2, 1), action(1, 0)]),
        };
        assert_eq!(
            state.verify(&state, &params()),
            Err("actions are not in canonical unique-ID order".into())
        );
    }

    #[test]
    fn full_state_merge_orders_converge() {
        let p = params();
        let left = LedgerState {
            actions: Actions(vec![action(1, 0)]),
        };
        let right = LedgerState {
            actions: Actions(vec![action(2, 1)]),
        };

        let mut left_then_right = left.clone();
        left_then_right
            .merge(&left_then_right.clone(), &p, &right)
            .unwrap();

        let mut right_then_left = right;
        right_then_left
            .merge(&right_then_left.clone(), &p, &left)
            .unwrap();

        assert_eq!(left_then_right, right_then_left);
        assert_eq!(left_then_right.actions.0.len(), 2);
    }

    #[test]
    fn cbor_round_trip_is_stable() {
        let state = LedgerState {
            actions: Actions(vec![action(1, 0), action(2, 1)]),
        };
        let first = encoded(&state);
        let decoded: LedgerState = decode(&first).unwrap();
        let second = encoded(&decoded);
        assert_eq!(decoded, state);
        assert_eq!(first, second);
    }
}
