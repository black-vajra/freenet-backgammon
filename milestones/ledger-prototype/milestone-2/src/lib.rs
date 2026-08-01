use ciborium::{de::from_reader, ser::into_writer};
use freenet_scaffold_macro::composable;
use freenet_stdlib::prelude::*;
use serde::{Deserialize, Serialize};

const PROTOCOL_VERSION: u16 = 1;
const MAX_ACTIONS: usize = 256;
const MAX_PAYLOAD_BYTES: usize = 1024;

#[derive(Serialize, Deserialize, Clone, Default, PartialEq, Eq, Debug)]
pub struct LedgerParameters {
    pub protocol_version: u16,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct Action {
    pub id: [u8; 32],
    pub sequence: u32,
    pub payload: Vec<u8>,
}

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
        if parameters.protocol_version != PROTOCOL_VERSION {
            return Err("unsupported protocol version".into());
        }
        self.verify_inner()
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
        Ok(StateSummary::from(encode(&state.summarize(&state, &parameters))?))
    }

    fn get_state_delta(
        parameters: Parameters<'static>,
        state: State<'static>,
        summary: StateSummary<'static>,
    ) -> Result<StateDelta<'static>, ContractError> {
        let parameters: LedgerParameters = decode(parameters.as_ref())?;
        let state: LedgerState = decode(state.as_ref())?;
        let summary: LedgerStateSummary = decode(summary.as_ref())?;
        Ok(StateDelta::from(encode(&state.delta(&state, &parameters, &summary))?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> LedgerParameters {
        LedgerParameters { protocol_version: PROTOCOL_VERSION }
    }

    fn action(id: u8, sequence: u32) -> Action {
        Action { id: [id; 32], sequence, payload: vec![id] }
    }

    #[test]
    fn opposite_update_orders_converge() {
        let p = params();
        let empty = LedgerState::default();
        let da = LedgerStateDelta { actions: Some(vec![action(1, 0)]) };
        let db = LedgerStateDelta { actions: Some(vec![action(2, 1)]) };
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
        let d = LedgerStateDelta { actions: Some(vec![action(1, 0)]) };
        let mut state = LedgerState::default();
        state.apply_delta(&state.clone(), &p, &Some(d.clone())).unwrap();
        let once = state.clone();
        state.apply_delta(&state.clone(), &p, &Some(d)).unwrap();
        assert_eq!(state, once);
    }

    #[test]
    fn same_id_with_different_content_is_rejected() {
        let p = params();
        let mut state = LedgerState::default();
        state.apply_delta(
            &state.clone(),
            &p,
            &Some(LedgerStateDelta { actions: Some(vec![action(1, 0)]) }),
        ).unwrap();
        let mut conflicting = action(1, 99);
        conflicting.payload = vec![9];
        assert!(state.apply_delta(
            &state.clone(),
            &p,
            &Some(LedgerStateDelta { actions: Some(vec![conflicting]) }),
        ).is_err());
    }
}
