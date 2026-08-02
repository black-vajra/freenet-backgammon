use std::sync::Arc;

use ciborium::{de::from_reader, ser::into_writer};
use freenet_stdlib::prelude::*;
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use super::super::Runtime;
use crate::contract::storages::Storage;
use crate::wasm_runtime::{ContractStore, DelegateStore, SecretsStore};
use crate::wasm_runtime::contract::ContractRuntimeInterface;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct LedgerParameters {
    protocol_version: u16,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct Action {
    id: [u8; 32],
    sequence: u32,
    payload: Vec<u8>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct Actions(Vec<Action>);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct LedgerState {
    actions: Actions,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct LedgerStateDelta {
    actions: Option<Vec<Action>>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct LedgerStateSummary {
    actions: Vec<[u8; 32]>,
}

fn encode<T: Serialize>(
    value: &T,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut bytes = Vec::new();
    into_writer(value, &mut bytes)?;
    Ok(bytes)
}

fn decode<T: DeserializeOwned>(
    bytes: &[u8],
) -> Result<T, Box<dyn std::error::Error>> {
    Ok(from_reader(bytes)?)
}

fn action(id: u8, sequence: u32) -> Action {
    Action {
        id: [id; 32],
        sequence,
        payload: vec![id],
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn ledger_package_runs_through_production_wasm_runtime(
) -> Result<(), Box<dyn std::error::Error>> {
    let package_path = std::env::var("LEDGER_WASM_PACKAGE")
        .expect("LEDGER_WASM_PACKAGE must name the packaged ledger contract");
    let module_bytes = std::fs::read(&package_path)?;

    assert!(!module_bytes.is_empty(), "contract package is empty");

    let wrapped = WrappedContract::new(
        Arc::new(ContractCode::from(module_bytes)),
        vec![].into(),
    );
    let contract =
        ContractContainer::Wasm(ContractWasmAPIVersion::V1(wrapped));
    let contract_key = contract.key();

    let temp_dir = crate::util::tests::get_temp_dir();
    let database = Storage::new(temp_dir.path()).await?;

    let mut contract_store = ContractStore::new(
        temp_dir.path().join("contract"),
        10_000,
        database.clone(),
    )?;
    let delegate_store = DelegateStore::new(
        temp_dir.path().join("delegate"),
        10_000,
        database.clone(),
    )?;
    let secrets_store = SecretsStore::new(
        temp_dir.path().join("secrets"),
        Default::default(),
        database,
    )?;

    contract_store.store_contract(contract)?;

    let mut runtime = Runtime::build(
        contract_store,
        delegate_store,
        secrets_store,
        false,
    )?;

    let parameters = Parameters::from(encode(&LedgerParameters {
        protocol_version: 1,
    })?);

    let empty = LedgerState {
        actions: Actions(Vec::new()),
    };
    let empty_bytes = encode(&empty)?;
    let empty_state = WrappedState::new(empty_bytes);

    let abi_probe_state = WrappedState::new(Vec::new());
    let abi_probe = runtime.validate_state(
        &contract_key,
        &parameters,
        &abi_probe_state,
        &RelatedContracts::default(),
    )?;
    assert_eq!(abi_probe, ValidateResult::Valid);
    let validation = runtime.validate_state(
        &contract_key,
        &parameters,
        &empty_state,
        &RelatedContracts::default(),
    )?;
    assert_eq!(validation, ValidateResult::Valid);
    let first_delta = LedgerStateDelta {
        actions: Some(vec![action(1, 0)]),
    };
    let first_update: UpdateData<'static> =
        StateDelta::from(encode(&first_delta)?).into();

    let state_after_first = runtime
        .update_state(
            &contract_key,
            &parameters,
            &empty_state,
            &[first_update],
        )?
        .unwrap_valid();

    let state_after_first =
        WrappedState::new(state_after_first.as_ref().to_vec());

    let expected_first = LedgerState {
        actions: Actions(vec![action(1, 0)]),
    };
    let expected_bytes = encode(&expected_first)?;
    assert_eq!(state_after_first.as_ref(), expected_bytes.as_slice());

    let decoded_first: LedgerState =
        decode(state_after_first.as_ref())?;
    assert_eq!(decoded_first.actions.0, vec![action(1, 0)]);

    let duplicate_update: UpdateData<'static> =
        StateDelta::from(encode(&first_delta)?).into();

    let state_after_duplicate = runtime
        .update_state(
            &contract_key,
            &parameters,
            &state_after_first,
            &[duplicate_update],
        )?
        .unwrap_valid();

    assert_eq!(
        state_after_duplicate.as_ref(),
        state_after_first.as_ref(),
        "duplicate action must be idempotent"
    );

    let conflicting_delta = LedgerStateDelta {
        actions: Some(vec![Action {
            id: [1; 32],
            sequence: 99,
            payload: vec![9],
        }]),
    };
    let conflicting_update: UpdateData<'static> =
        StateDelta::from(encode(&conflicting_delta)?).into();

    assert!(
        runtime
            .update_state(
                &contract_key,
                &parameters,
                &state_after_first,
                &[conflicting_update],
            )
            .is_err(),
        "same ID with different content must be rejected"
    );

    let summary = runtime.summarize_state(
        &contract_key,
        &parameters,
        &state_after_first,
    )?;
    
            let decoded_summary: LedgerStateSummary =
        decode(summary.as_ref())?;
    assert_eq!(decoded_summary.actions, vec![[1; 32]]);

    let empty_summary = StateSummary::from(encode(
        &LedgerStateSummary {
            actions: Vec::new(),
        },
    )?);
    let reconstructed_delta = runtime.get_state_delta(
        &contract_key,
        &parameters,
        &state_after_first,
        &empty_summary,
    )?;
    let decoded_delta: LedgerStateDelta =
        decode(reconstructed_delta.as_ref())?;
    assert_eq!(
        decoded_delta.actions,
        Some(vec![action(1, 0)])
    );

    let malformed = WrappedState::new(vec![0x9f, 0x01]);
    assert!(
        runtime
            .validate_state(
                &contract_key,
                &parameters,
                &malformed,
                &RelatedContracts::default(),
            )
            .is_err(),
        "malformed CBOR must be rejected"
    );

    Ok(())
}
