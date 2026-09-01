use std::io::Cursor;

use backgammon_contract::LedgerState;
use backgammon_protocol::{GameId, LedgerParameters};
use ciborium::{de::from_reader, ser::into_writer};
use serde::{de::DeserializeOwned, Serialize};

/// Versioned Freenet package built from the protocol-v4 game contract.
///
/// This tracked asset makes creation of a new game independent of any
/// previously published game-ledger instance.
pub const GAME_CONTRACT_PACKAGE: &[u8] = include_bytes!("../assets/backgammon_contract_v4");

/// Operational SHA-256 pin for the exact tracked package asset.
pub const GAME_CONTRACT_PACKAGE_SHA256: &str =
    "fc0f6675d20dea4e59fd53a6d54ac1c0c7f94fe4edfffb326a0cdadd5a6f06dc";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameContractPublicationInputs {
    pub game_id: GameId,
    pub parameter_bytes: Vec<u8>,
    pub state_bytes: Vec<u8>,
}

fn encode_canonical<T: Serialize>(value: &T, label: &str) -> Result<Vec<u8>, String> {
    let mut encoded = Vec::new();

    into_writer(value, &mut encoded)
        .map_err(|error| format!("Could not encode {label}: {error:?}"))?;

    Ok(encoded)
}

fn decode_exact<T: DeserializeOwned>(encoded: &[u8], label: &str) -> Result<T, String> {
    let mut cursor = Cursor::new(encoded);

    let decoded =
        from_reader(&mut cursor).map_err(|error| format!("Could not decode {label}: {error:?}"))?;

    if cursor.position() != encoded.len() as u64 {
        return Err(format!("{label} contains trailing noncanonical data."));
    }

    Ok(decoded)
}

/// Builds the exact typed Freenet parameters and canonical empty ledger for a
/// unique game-contract instance.
///
/// The signed protocol `game_id` is used byte-for-byte as the Freenet
/// instance nonce. Both challenge participants can therefore calculate the
/// same expected contract identity from authenticated challenge evidence.
/// The zero game ID remains reserved for the historical shared test ledger.
pub fn prepare_game_contract_publication(
    game_id: GameId,
) -> Result<GameContractPublicationInputs, String> {
    if game_id == [0_u8; 32] {
        return Err("A per-game contract requires a nonzero instance nonce.".to_owned());
    }

    let parameters = LedgerParameters::for_instance(game_id);

    parameters
        .verify()
        .map_err(|error| format!("Game parameters failed verification: {error}"))?;

    let parameter_bytes = encode_canonical(&parameters, "game contract parameters")?;

    let decoded_parameters: LedgerParameters =
        decode_exact(&parameter_bytes, "game contract parameters")?;

    if decoded_parameters != parameters {
        return Err("Game contract parameters failed exact typed round-trip.".to_owned());
    }

    decoded_parameters
        .verify()
        .map_err(|error| format!("Decoded game parameters failed verification: {error}"))?;

    let state = LedgerState::default();
    let state_bytes = encode_canonical(&state, "empty game ledger")?;

    let decoded_state: LedgerState = decode_exact(&state_bytes, "empty game ledger")?;

    if decoded_state != state {
        return Err("Empty game ledger failed exact typed round-trip.".to_owned());
    }

    let canonical_parameter_bytes =
        encode_canonical(&decoded_parameters, "decoded game parameters")?;

    if canonical_parameter_bytes != parameter_bytes {
        return Err("Game contract parameter encoding is not canonical.".to_owned());
    }

    let canonical_state_bytes = encode_canonical(&decoded_state, "decoded empty game ledger")?;

    if canonical_state_bytes != state_bytes {
        return Err("Empty game-ledger encoding is not canonical.".to_owned());
    }

    Ok(GameContractPublicationInputs {
        game_id,
        parameter_bytes,
        state_bytes,
    })
}

#[cfg(target_arch = "wasm32")]
#[derive(Clone, Debug)]
pub struct SubmittedGameContractPublication {
    pub game_id: GameId,
    pub expected_key: freenet_stdlib::prelude::ContractKey,
    pub contract_id: String,
}

/// Submits a uniquely parameterized, canonical empty protocol-v4 ledger.
///
/// Successful return means only that the request was accepted by the local
/// WebSocket API. The caller must retain the returned expected key and wait
/// for an exact matching `PutResponse` before treating the contract as live.
#[cfg(target_arch = "wasm32")]
pub async fn submit_game_contract_publication(
    api: &mut freenet_stdlib::client_api::WebApi,
    game_id: GameId,
    mut on_prepared: impl FnMut(Option<&SubmittedGameContractPublication>),
) -> Result<SubmittedGameContractPublication, String> {
    use freenet_stdlib::client_api::{ClientRequest, ContractRequest};
    use freenet_stdlib::prelude::{ContractContainer, Parameters, RelatedContracts, WrappedState};

    let inputs = prepare_game_contract_publication(game_id)?;

    let parameters = Parameters::from(inputs.parameter_bytes);
    let state = WrappedState::from(inputs.state_bytes);

    let contract = ContractContainer::try_from((GAME_CONTRACT_PACKAGE.to_vec(), &parameters))
        .map_err(|error| {
            format!(
                "Could not load pinned game contract package \
             {GAME_CONTRACT_PACKAGE_SHA256}: {error}"
            )
        })?;

    let expected_key = contract.key();
    let contract_id = expected_key.id().encode();

    if contract_id.is_empty() {
        return Err("Calculated game contract ID is unexpectedly empty.".to_owned());
    }

    let submitted = SubmittedGameContractPublication {
        game_id,
        expected_key,
        contract_id,
    };

    // Arm response routing before the request can produce a PutResponse.
    on_prepared(Some(&submitted));

    let send_result = api
        .send(ClientRequest::ContractOp(ContractRequest::Put {
            contract,
            state,
            related_contracts: RelatedContracts::new(),
            subscribe: true,
            blocking_subscribe: true,
        }))
        .await;

    if let Err(error) = send_result {
        // A failed send cannot have a valid pending publication response.
        on_prepared(None);

        return Err(format!(
            "Could not submit the new game contract publication: {error:?}"
        ));
    }

    Ok(submitted)
}

/// Recognizes only a publication response whose complete key exactly matches
/// the key calculated before submission.
///
/// `Ok(None)` means the response is unrelated to contract publication.
/// A mismatched `PutResponse` fails closed.
#[cfg(target_arch = "wasm32")]
pub fn confirm_game_contract_publication(
    response: &freenet_stdlib::client_api::HostResponse,
    expected_key: &freenet_stdlib::prelude::ContractKey,
) -> Result<Option<freenet_stdlib::prelude::ContractKey>, String> {
    use freenet_stdlib::client_api::{ContractResponse, HostResponse};

    let HostResponse::ContractResponse(ContractResponse::PutResponse { key }) = response else {
        return Ok(None);
    };

    if key != expected_key {
        return Err(format!(
            "Game contract publication returned an unexpected key: \
             expected {}, received {}.",
            expected_key.id().encode(),
            key.id().encode(),
        ));
    }

    Ok(Some(key.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const EMPTY_LEDGER_CBOR: &[u8] = &[0xa1, 0x67, b'a', b'c', b't', b'i', b'o', b'n', b's', 0x80];

    #[test]
    fn zero_nonce_is_reserved_and_rejected() {
        assert!(prepare_game_contract_publication([0_u8; 32]).is_err());
    }

    #[test]
    fn signed_game_id_is_the_exact_contract_instance_nonce() {
        let game_id = [23_u8; 32];

        let inputs = prepare_game_contract_publication(game_id).unwrap();

        let parameters: LedgerParameters = from_reader(inputs.parameter_bytes.as_slice()).unwrap();

        assert_eq!(inputs.game_id, game_id);
        assert_eq!(parameters.instance_nonce, game_id);
        assert_eq!(parameters.verify(), Ok(()));
    }

    #[test]
    fn identical_nonce_produces_identical_publication_inputs() {
        let first = prepare_game_contract_publication([17_u8; 32]).unwrap();

        let second = prepare_game_contract_publication([17_u8; 32]).unwrap();

        assert_eq!(first, second);
    }

    #[test]
    fn distinct_nonces_change_only_contract_parameters() {
        let first = prepare_game_contract_publication([17_u8; 32]).unwrap();

        let second = prepare_game_contract_publication([18_u8; 32]).unwrap();

        assert_ne!(first.parameter_bytes, second.parameter_bytes);
        assert_eq!(first.state_bytes, second.state_bytes);
        assert_eq!(first.state_bytes, EMPTY_LEDGER_CBOR);
    }

    #[test]
    fn package_asset_has_expected_versioned_wasm_shape() {
        assert_eq!(GAME_CONTRACT_PACKAGE.len(), 402_842);
        assert_eq!(&GAME_CONTRACT_PACKAGE[..8], &[0_u8; 8]);
        assert_eq!(&GAME_CONTRACT_PACKAGE[40..44], b"\0asm");

        assert_eq!(
            GAME_CONTRACT_PACKAGE_SHA256,
            "fc0f6675d20dea4e59fd53a6d54ac1c0c7f94fe4edfffb326a0cdadd5a6f06dc",
        );
    }
}
