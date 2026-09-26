use ciborium::{de::from_reader, ser::into_writer};
use serde::{Deserialize, Serialize};

pub const LOCAL_STATE_PROTOCOL_VERSION: u16 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LocalStateRequest {
    GetIdentity,
    StoreIdentity([u8; 32]),
    ClearIdentity,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LocalStateResponse {
    Identity(Option<[u8; 32]>),
    IdentityStored,
    IdentityCleared,
    Error(String),
}

pub fn encode_local_state_request(request: &LocalStateRequest) -> Result<Vec<u8>, String> {
    let mut encoded = Vec::new();

    into_writer(request, &mut encoded)
        .map_err(|error| format!("failed to encode local-state request: {error}"))?;

    Ok(encoded)
}

pub fn decode_local_state_request(bytes: &[u8]) -> Result<LocalStateRequest, String> {
    from_reader(bytes)
        .map_err(|error| format!("failed to decode local-state request: {error}"))
}

pub fn encode_local_state_response(response: &LocalStateResponse) -> Result<Vec<u8>, String> {
    let mut encoded = Vec::new();

    into_writer(response, &mut encoded)
        .map_err(|error| format!("failed to encode local-state response: {error}"))?;

    Ok(encoded)
}

pub fn decode_local_state_response(bytes: &[u8]) -> Result<LocalStateResponse, String> {
    from_reader(bytes)
        .map_err(|error| format!("failed to decode local-state response: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_request_round_trip() {
        let request = LocalStateRequest::StoreIdentity([0x5a; 32]);
        let encoded = encode_local_state_request(&request).unwrap();
        let decoded = decode_local_state_request(&encoded).unwrap();

        assert_eq!(decoded, request);
    }

    #[test]
    fn identity_response_round_trip() {
        let response = LocalStateResponse::Identity(Some([0xa5; 32]));
        let encoded = encode_local_state_response(&response).unwrap();
        let decoded = decode_local_state_response(&encoded).unwrap();

        assert_eq!(decoded, response);
    }
}
