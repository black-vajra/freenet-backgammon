use ciborium::{de::from_reader, ser::into_writer};
use serde::{Deserialize, Serialize};

use crate::{validate_display_name, PlayerId};

pub const PROFILE_STATE_PROTOCOL_VERSION: u16 = 1;
pub type ProfileRequestId = [u8; 32];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileStateRequest {
    pub version: u16,
    pub request_id: ProfileRequestId,
    pub player_id: PlayerId,
    pub action: ProfileStateAction,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProfileStateAction {
    Get,
    Store(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileStateResponse {
    pub version: u16,
    pub request_id: ProfileRequestId,
    pub player_id: PlayerId,
    pub result: ProfileStateResult,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProfileStateResult {
    DisplayName(Option<String>),
    Stored,
    Error(String),
}

pub fn encode_profile_state_request(request: &ProfileStateRequest) -> Result<Vec<u8>, String> {
    let mut encoded = Vec::new();
    into_writer(request, &mut encoded)
        .map_err(|error| format!("failed to encode profile request: {error}"))?;
    Ok(encoded)
}

pub fn decode_profile_state_request(bytes: &[u8]) -> Result<ProfileStateRequest, String> {
    if bytes.len() > 512 {
        return Err("profile request exceeds maximum length".to_owned());
    }
    let request: ProfileStateRequest =
        from_reader(bytes).map_err(|error| format!("failed to decode profile request: {error}"))?;
    if request.version != PROFILE_STATE_PROTOCOL_VERSION {
        return Err("unsupported profile request version".to_owned());
    }
    if let ProfileStateAction::Store(name) = &request.action {
        validate_display_name(name)?;
    }
    Ok(request)
}

pub fn encode_profile_state_response(response: &ProfileStateResponse) -> Result<Vec<u8>, String> {
    let mut encoded = Vec::new();
    into_writer(response, &mut encoded)
        .map_err(|error| format!("failed to encode profile response: {error}"))?;
    Ok(encoded)
}

pub fn decode_profile_state_response(bytes: &[u8]) -> Result<ProfileStateResponse, String> {
    if bytes.len() > 512 {
        return Err("profile response exceeds maximum length".to_owned());
    }
    let response: ProfileStateResponse = from_reader(bytes)
        .map_err(|error| format!("failed to decode profile response: {error}"))?;
    if response.version != PROFILE_STATE_PROTOCOL_VERSION {
        return Err("unsupported profile response version".to_owned());
    }
    if let ProfileStateResult::DisplayName(Some(name)) = &response.result {
        validate_display_name(name)?;
    }
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_store_before_persistence() {
        let request = ProfileStateRequest {
            version: PROFILE_STATE_PROTOCOL_VERSION,
            request_id: [1; 32],
            player_id: [2; 32],
            action: ProfileStateAction::Store(" ".into()),
        };
        let encoded = encode_profile_state_request(&request).unwrap();
        assert!(decode_profile_state_request(&encoded).is_err());
    }

    #[test]
    fn rejects_invalid_stored_name_on_readback() {
        let response = ProfileStateResponse {
            version: PROFILE_STATE_PROTOCOL_VERSION,
            request_id: [1; 32],
            player_id: [2; 32],
            result: ProfileStateResult::DisplayName(Some(" ".into())),
        };
        let encoded = encode_profile_state_response(&response).unwrap();
        assert!(decode_profile_state_response(&encoded).is_err());
    }
}
