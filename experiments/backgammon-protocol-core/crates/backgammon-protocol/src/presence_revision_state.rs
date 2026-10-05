use ciborium::{de::from_reader, ser::into_writer};
use serde::{Deserialize, Serialize};

use crate::PlayerId;

pub const PRESENCE_REVISION_PROTOCOL_VERSION: u16 = 1;
pub type PresenceRevisionRequestId = [u8; 32];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresenceRevisionRequest {
    pub version: u16,
    pub request_id: PresenceRevisionRequestId,
    pub player_id: PlayerId,
    pub action: PresenceRevisionAction,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PresenceRevisionAction {
    GetReserved,
    /// The highest revision known from a verified authoritative lobby state.
    /// A reservation must be greater than both this floor and local history.
    ReserveNext {
        observed_revision: u64,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresenceRevisionResponse {
    pub version: u16,
    pub request_id: PresenceRevisionRequestId,
    pub player_id: PlayerId,
    pub result: PresenceRevisionResult,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PresenceRevisionResult {
    LastReserved(Option<u64>),
    Reserved(u64),
    Error(String),
}

pub fn next_reserved_revision(last: Option<u64>, observed: u64) -> Result<u64, String> {
    last.unwrap_or(0)
        .max(observed)
        .checked_add(1)
        .ok_or_else(|| {
            "Presence revision counter is exhausted; refusing to reuse a revision.".to_owned()
        })
}

pub fn encode_presence_revision_request(
    request: &PresenceRevisionRequest,
) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    into_writer(request, &mut bytes)
        .map_err(|error| format!("failed to encode presence revision request: {error}"))?;
    Ok(bytes)
}

pub fn decode_presence_revision_request(bytes: &[u8]) -> Result<PresenceRevisionRequest, String> {
    if bytes.len() > 256 {
        return Err("presence revision request exceeds maximum length".to_owned());
    }
    let request: PresenceRevisionRequest = from_reader(bytes)
        .map_err(|error| format!("failed to decode presence revision request: {error}"))?;
    if request.version != PRESENCE_REVISION_PROTOCOL_VERSION {
        return Err("unsupported presence revision request version".to_owned());
    }
    Ok(request)
}

pub fn encode_presence_revision_response(
    response: &PresenceRevisionResponse,
) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    into_writer(response, &mut bytes)
        .map_err(|error| format!("failed to encode presence revision response: {error}"))?;
    Ok(bytes)
}

pub fn decode_presence_revision_response(bytes: &[u8]) -> Result<PresenceRevisionResponse, String> {
    if bytes.len() > 512 {
        return Err("presence revision response exceeds maximum length".to_owned());
    }
    let response: PresenceRevisionResponse = from_reader(bytes)
        .map_err(|error| format!("failed to decode presence revision response: {error}"))?;
    if response.version != PRESENCE_REVISION_PROTOCOL_VERSION {
        return Err("unsupported presence revision response version".to_owned());
    }
    if matches!(
        &response.result,
        PresenceRevisionResult::LastReserved(Some(0)) | PresenceRevisionResult::Reserved(0)
    ) {
        return Err("presence revision zero is invalid".to_owned());
    }
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserves_above_both_durable_and_verified_network_history() {
        assert_eq!(next_reserved_revision(None, 0), Ok(1));
        assert_eq!(next_reserved_revision(Some(3), 7), Ok(8));
        assert_eq!(next_reserved_revision(Some(11), 7), Ok(12));
        assert!(next_reserved_revision(None, u64::MAX).is_err());
        assert!(next_reserved_revision(Some(u64::MAX), 0).is_err());
    }

    #[test]
    fn refuses_zero_and_unsupported_versions() {
        let response = PresenceRevisionResponse {
            version: PRESENCE_REVISION_PROTOCOL_VERSION,
            request_id: [1; 32],
            player_id: [2; 32],
            result: PresenceRevisionResult::Reserved(0),
        };
        assert!(decode_presence_revision_response(
            &encode_presence_revision_response(&response).unwrap()
        )
        .is_err());

        let request = PresenceRevisionRequest {
            version: 42,
            request_id: [1; 32],
            player_id: [2; 32],
            action: PresenceRevisionAction::GetReserved,
        };
        assert!(decode_presence_revision_request(
            &encode_presence_revision_request(&request).unwrap()
        )
        .is_err());
    }
}
