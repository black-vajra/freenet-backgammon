//! Browser transport boundary for the published Freenet lobby contract.
//!
//! This module is deliberately separate from the game-ledger transport. It
//! requests and classifies lobby traffic, and it exposes complete lobby state
//! only after hostile network bytes pass `decode_verified_lobby_state()`.
//!
//! Presence and challenge publication are intentionally outside this
//! retrieval-only milestone.

use backgammon_lobby_core::LobbyContractState;

use crate::lobby_codec::decode_verified_lobby_state;

#[cfg(target_arch = "wasm32")]
use crate::transport::SubscriptionStatus;

/// Published challenge-capable lobby contract independently retrieved across
/// two Freenet nodes at the August 2026 publication milestone.
pub const LOBBY_CONTRACT_ID: &str = "CuzYmHzg94LwEpQP9sXTXhHHsAKB6pYC5uABt42CHR8K";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LobbyContractStatus {
    WaitingForConnection,
    Requesting,
    Refreshing,
    Retrieved {
        bytes: usize,
        presence_records: usize,
        challenge_offers: usize,
    },
    NotFound,
    Failed(String),
}

impl LobbyContractStatus {
    pub fn label(&self) -> String {
        match self {
            Self::WaitingForConnection => "Waiting for Freenet".to_owned(),
            Self::Requesting => "Retrieving published lobby".to_owned(),
            Self::Refreshing => "Refreshing authoritative lobby state".to_owned(),
            Self::Retrieved { bytes, .. } => {
                format!("Verified lobby retrieved — {bytes} bytes")
            }
            Self::NotFound => "Published lobby not found".to_owned(),
            Self::Failed(_) => "Lobby retrieval failed".to_owned(),
        }
    }

    pub fn detail(&self) -> String {
        match self {
            Self::WaitingForConnection | Self::Requesting => "Pending".to_owned(),
            Self::Refreshing => {
                "A lobby delta was received; requesting complete authoritative state.".to_owned()
            }
            Self::Retrieved {
                presence_records,
                challenge_offers,
                ..
            } => format!(
                "{presence_records} verified presence record{}, \
                 {challenge_offers} verified challenge offer{}",
                if *presence_records == 1 { "" } else { "s" },
                if *challenge_offers == 1 { "" } else { "s" },
            ),
            Self::NotFound => "No lobby state was returned.".to_owned(),
            Self::Failed(error) => error.clone(),
        }
    }
}

#[cfg(any(test, target_arch = "wasm32"))]
fn verified_state(bytes: &[u8]) -> (LobbyContractStatus, Option<LobbyContractState>) {
    match decode_verified_lobby_state(bytes) {
        Ok(state) => {
            let presence_records = state
                .lobby
                .0
                .players
                .iter()
                .map(|player| player.records.len())
                .sum();

            let challenge_offers = state.challenges.offers.len();

            (
                LobbyContractStatus::Retrieved {
                    bytes: bytes.len(),
                    presence_records,
                    challenge_offers,
                },
                Some(state),
            )
        }
        Err(error) => (
            LobbyContractStatus::Failed(format!(
                "Retrieved lobby state failed verification: {error}"
            )),
            None,
        ),
    }
}

pub fn host_result_error_status(error: impl std::fmt::Display) -> LobbyContractStatus {
    LobbyContractStatus::Failed(format!("Freenet lobby operation failed: {error}"))
}

#[cfg(target_arch = "wasm32")]
pub struct ClassifiedLobbyResponse {
    pub contract_status: Option<LobbyContractStatus>,
    pub subscription_status: Option<SubscriptionStatus>,
    pub contract_key: Option<freenet_stdlib::prelude::ContractKey>,
    pub authoritative_state: Option<LobbyContractState>,
    pub refresh_required: bool,
}

#[cfg(target_arch = "wasm32")]
pub async fn request_lobby_contract(
    api: &mut freenet_stdlib::client_api::WebApi,
) -> Result<(), String> {
    use freenet_stdlib::client_api::{ClientRequest, ContractRequest};
    use freenet_stdlib::prelude::ContractInstanceId;

    let key = ContractInstanceId::try_from(LOBBY_CONTRACT_ID.to_owned())
        .map_err(|error| format!("Invalid published lobby contract ID: {error}"))?;

    if key.encode() != LOBBY_CONTRACT_ID {
        return Err("Published lobby contract ID is not canonically encoded.".to_owned());
    }

    api.send(ClientRequest::ContractOp(ContractRequest::Get {
        key,
        return_contract_code: false,
        subscribe: true,
        blocking_subscribe: true,
    }))
    .await
    .map_err(|error| format!("Could not request the published lobby: {error:?}"))
}

/// Classifies one host response without consuming it.
///
/// Non-lobby responses return `None` and remain available to the existing
/// game-ledger classifier.
#[cfg(target_arch = "wasm32")]
pub fn classify_lobby_response(
    response: &freenet_stdlib::client_api::HostResponse,
) -> Option<ClassifiedLobbyResponse> {
    use freenet_stdlib::client_api::{ContractResponse, HostResponse};
    use freenet_stdlib::prelude::UpdateData;

    let HostResponse::ContractResponse(response) = response else {
        return None;
    };

    match response {
        ContractResponse::GetResponse { key, state, .. } => {
            if key.id().encode() != LOBBY_CONTRACT_ID {
                return None;
            }

            let (contract_status, authoritative_state) = verified_state(state.as_ref());

            let contract_key = authoritative_state.as_ref().map(|_| key.clone());

            Some(ClassifiedLobbyResponse {
                contract_status: Some(contract_status),
                subscription_status: Some(SubscriptionStatus::Active),
                contract_key,
                authoritative_state,
                refresh_required: false,
            })
        }

        ContractResponse::UpdateNotification { key, update } => {
            if key.id().encode() != LOBBY_CONTRACT_ID {
                return None;
            }

            match update {
                UpdateData::State(state) => {
                    let (contract_status, authoritative_state) = verified_state(state.as_ref());

                    let contract_key = authoritative_state.as_ref().map(|_| key.clone());

                    Some(ClassifiedLobbyResponse {
                        contract_status: Some(contract_status),
                        subscription_status: Some(SubscriptionStatus::Active),
                        contract_key,
                        authoritative_state,
                        refresh_required: false,
                    })
                }

                UpdateData::Delta(delta) => {
                    let (contract_status, refresh_required) = if delta.as_ref().is_empty() {
                        (
                            LobbyContractStatus::Failed(
                                "Received an empty lobby delta.".to_owned(),
                            ),
                            false,
                        )
                    } else {
                        (LobbyContractStatus::Refreshing, true)
                    };

                    Some(ClassifiedLobbyResponse {
                        contract_status: Some(contract_status),
                        subscription_status: Some(SubscriptionStatus::Active),
                        contract_key: None,
                        authoritative_state: None,
                        refresh_required,
                    })
                }

                _ => Some(ClassifiedLobbyResponse {
                    contract_status: Some(LobbyContractStatus::Failed(
                        "Received an unsupported lobby update type.".to_owned(),
                    )),
                    subscription_status: Some(SubscriptionStatus::Active),
                    contract_key: None,
                    authoritative_state: None,
                    refresh_required: false,
                }),
            }
        }

        ContractResponse::SubscribeResponse { key, subscribed } => {
            if key.id().encode() != LOBBY_CONTRACT_ID {
                return None;
            }

            Some(ClassifiedLobbyResponse {
                contract_status: None,
                subscription_status: Some(if *subscribed {
                    SubscriptionStatus::Active
                } else {
                    SubscriptionStatus::Inactive
                }),
                contract_key: None,
                authoritative_state: None,
                refresh_required: false,
            })
        }

        ContractResponse::NotFound { instance_id } => {
            if instance_id.encode() != LOBBY_CONTRACT_ID {
                return None;
            }

            Some(ClassifiedLobbyResponse {
                contract_status: Some(LobbyContractStatus::NotFound),
                subscription_status: Some(SubscriptionStatus::Inactive),
                contract_key: None,
                authoritative_state: None,
                refresh_required: false,
            })
        }

        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CANONICAL_INITIAL_LOBBY_CBOR: &[u8] = &[
        0xa2, 0x65, b'l', b'o', b'b', b'b', b'y', 0xa1, 0x67, b'p', b'l', b'a', b'y', b'e', b'r',
        b's', 0x80, 0x6a, b'c', b'h', b'a', b'l', b'l', b'e', b'n', b'g', b'e', b's', 0xa1, 0x66,
        b'o', b'f', b'f', b'e', b'r', b's', 0x80,
    ];

    #[test]
    fn published_lobby_contract_id_is_stable() {
        assert_eq!(
            LOBBY_CONTRACT_ID,
            "CuzYmHzg94LwEpQP9sXTXhHHsAKB6pYC5uABt42CHR8K"
        );
    }

    #[test]
    fn canonical_published_initial_state_is_verified() {
        assert_eq!(CANONICAL_INITIAL_LOBBY_CBOR.len(), 37);

        let (status, state) = verified_state(CANONICAL_INITIAL_LOBBY_CBOR);

        assert!(matches!(
            status,
            LobbyContractStatus::Retrieved {
                bytes: 37,
                presence_records: 0,
                challenge_offers: 0,
            }
        ));

        assert_eq!(state, Some(LobbyContractState::default()));
    }

    #[test]
    fn malformed_state_is_never_exposed() {
        let (status, state) = verified_state(&[0x9f, 0x01]);

        assert!(matches!(status, LobbyContractStatus::Failed(_)));
        assert!(state.is_none());
    }

    #[test]
    fn lobby_status_labels_are_stable() {
        assert_eq!(
            LobbyContractStatus::WaitingForConnection.label(),
            "Waiting for Freenet"
        );
        assert_eq!(
            LobbyContractStatus::Refreshing.label(),
            "Refreshing authoritative lobby state"
        );
        assert_eq!(
            LobbyContractStatus::NotFound.detail(),
            "No lobby state was returned."
        );
    }

    #[test]
    fn host_operation_errors_are_lobby_scoped() {
        let status = host_result_error_status("subscription failed");

        assert!(matches!(status, LobbyContractStatus::Failed(_)));
        assert!(status.detail().contains("subscription failed"));
    }
}
