#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConnectionStatus {
    Connecting,
    Connected,
    Disconnected,
    Failed(String),
}

impl ConnectionStatus {
    pub fn label(&self) -> &str {
        match self {
            Self::Connecting => "Connecting to Freenet",
            Self::Connected => "Freenet connected",
            Self::Disconnected => "Freenet disconnected",
            Self::Failed(_) => "Freenet connection failed",
        }
    }

    pub fn detail(&self) -> &str {
        match self {
            Self::Connecting => "Opening the local node WebSocket.",
            Self::Connected => "Connected to the local Freenet node.",
            Self::Disconnected => "The local Freenet connection is closed.",
            Self::Failed(message) => message,
        }
    }

    pub fn css_class(&self) -> &'static str {
        match self {
            Self::Connecting => "connecting",
            Self::Connected => "connected",
            Self::Disconnected => "disconnected",
            Self::Failed(_) => "failed",
        }
    }

    pub fn can_reconnect(&self) -> bool {
        matches!(self, Self::Disconnected | Self::Failed(_))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContractProbeStatus {
    WaitingForConnection,
    Requesting,
    Updating,
    VerifyingUpdate,
    Retrieved { bytes: usize, action_count: usize },
    NotFound,
    Failed(String),
}

impl ContractProbeStatus {
    pub fn contract_label(&self) -> String {
        match self {
            Self::WaitingForConnection => "Waiting for Freenet".to_owned(),
            Self::Requesting => "Retrieving test ledger".to_owned(),
            Self::Updating => "Submitting first game action".to_owned(),
            Self::VerifyingUpdate => "Verifying updated ledger".to_owned(),
            Self::Retrieved { bytes, .. } => format!("Retrieved — {bytes} bytes"),
            Self::NotFound => "Test ledger not found".to_owned(),
            Self::Failed(_) => "Contract operation failed".to_owned(),
        }
    }

    pub fn state_label(&self) -> String {
        match self {
            Self::Retrieved {
                action_count: 0, ..
            } => "Empty ledger verified".to_owned(),
            Self::Retrieved {
                action_count: 1, ..
            } => "One network action verified".to_owned(),
            Self::Retrieved { action_count, .. } => {
                format!("{action_count} network actions verified")
            }
            Self::Updating => "Update submitted".to_owned(),
            Self::VerifyingUpdate => "Awaiting authoritative state".to_owned(),
            Self::Failed(message) => message.clone(),
            Self::NotFound => "No state returned".to_owned(),
            Self::WaitingForConnection | Self::Requesting => "Pending".to_owned(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SubscriptionStatus {
    Pending,
    Active,
    Inactive,
    Failed(String),
}

impl SubscriptionStatus {
    pub fn label(&self) -> &str {
        match self {
            Self::Pending => "Pending",
            Self::Active => "Active",
            Self::Inactive => "Inactive",
            Self::Failed(message) => message,
        }
    }
}

const EMPTY_LEDGER_CBOR: &[u8] = &[0xa1, 0x67, b'a', b'c', b't', b'i', b'o', b'n', b's', 0x80];

const FIRST_CREATE_DELTA_CBOR: &[u8] =
    include_bytes!("../fixtures/create-game-sequence-0.delta.cbor");

const EXPECTED_ONE_ACTION_STATE_CBOR: &[u8] =
    include_bytes!("../fixtures/expected-one-action-state.cbor");

fn retrieved_status(bytes: &[u8]) -> ContractProbeStatus {
    match crate::ledger_codec::decode_verified_ledger(bytes) {
        Ok(ledger) => ContractProbeStatus::Retrieved {
            bytes: bytes.len(),
            action_count: ledger.action_count(),
        },
        Err(error) => {
            ContractProbeStatus::Failed(format!("Retrieved ledger failed verification: {error}"))
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub struct ClassifiedResponse {
    pub contract_status: Option<ContractProbeStatus>,
    pub subscription_status: Option<SubscriptionStatus>,
    pub contract_key: Option<freenet_stdlib::prelude::ContractKey>,
    pub authoritative_state: Option<Vec<u8>>,
    pub should_submit_first_delta: bool,
}

#[cfg(target_arch = "wasm32")]
pub const DEFAULT_NODE_URL: &str =
    "ws://127.0.0.1:7509/v1/contract/command?encodingProtocol=native";

#[cfg(target_arch = "wasm32")]
pub const TEST_CONTRACT_ID: &str = "CE2FnE1vkobR1LssLzX3FBB1EgwXEvtPgUtQ4QdZsA9S";

#[cfg(target_arch = "wasm32")]
pub fn connect(
    status_handler: impl Fn(ConnectionStatus) + Clone + 'static,
    response_handler: impl Fn(freenet_stdlib::client_api::HostResponse) + 'static,
    open_handler: impl Fn() + 'static,
) -> Result<freenet_stdlib::client_api::WebApi, String> {
    use freenet_stdlib::client_api::WebApi;

    status_handler(ConnectionStatus::Connecting);

    let websocket = web_sys::WebSocket::new(DEFAULT_NODE_URL).map_err(|error| {
        format!("Could not create the Freenet WebSocket for {DEFAULT_NODE_URL}: {error:?}")
    })?;

    let open_status = status_handler.clone();
    let error_status = status_handler.clone();

    Ok(WebApi::start(
        websocket,
        move |result| match result {
            Ok(response) => response_handler(response),
            Err(error) => status_handler(ConnectionStatus::Failed(format!(
                "The Freenet node returned an error: {error:?}"
            ))),
        },
        move |error| {
            error_status(ConnectionStatus::Failed(format!(
                "Freenet WebSocket error: {error}"
            )));
        },
        move || {
            open_status(ConnectionStatus::Connected);
            open_handler();
        },
    ))
}

#[cfg(target_arch = "wasm32")]
pub async fn request_test_contract(
    api: &mut freenet_stdlib::client_api::WebApi,
) -> Result<(), String> {
    use freenet_stdlib::client_api::{ClientRequest, ContractRequest};
    use freenet_stdlib::prelude::ContractInstanceId;

    let key = ContractInstanceId::try_from(TEST_CONTRACT_ID.to_owned())
        .map_err(|error| format!("Invalid test contract ID: {error}"))?;

    if key.encode() != TEST_CONTRACT_ID {
        return Err("Test contract ID is not canonically encoded.".to_owned());
    }

    api.send(ClientRequest::ContractOp(ContractRequest::Get {
        key,
        return_contract_code: false,
        subscribe: true,
        blocking_subscribe: true,
    }))
    .await
    .map_err(|error| format!("Could not request the test ledger: {error:?}"))
}

#[cfg(target_arch = "wasm32")]
pub async fn submit_action_delta(
    api: &mut freenet_stdlib::client_api::WebApi,
    key: freenet_stdlib::prelude::ContractKey,
    delta: Vec<u8>,
) -> Result<(), String> {
    use freenet_stdlib::client_api::{ClientRequest, ContractRequest};
    use freenet_stdlib::prelude::UpdateData;

    if key.id().encode() != TEST_CONTRACT_ID {
        return Err("Refusing to update an unexpected contract key.".to_owned());
    }

    if delta.is_empty() {
        return Err("Refusing to submit an empty ledger delta.".to_owned());
    }

    api.send(ClientRequest::ContractOp(ContractRequest::Update {
        key,
        data: UpdateData::Delta(delta.into()),
    }))
    .await
    .map_err(|error| format!("Could not submit the ledger action: {error:?}"))
}

#[cfg(target_arch = "wasm32")]
pub async fn submit_first_create_delta(
    api: &mut freenet_stdlib::client_api::WebApi,
    key: freenet_stdlib::prelude::ContractKey,
) -> Result<(), String> {
    if FIRST_CREATE_DELTA_CBOR != EXPECTED_ONE_ACTION_STATE_CBOR {
        return Err("The pinned delta and expected state fixtures differ.".to_owned());
    }

    submit_action_delta(api, key, FIRST_CREATE_DELTA_CBOR.to_vec()).await
}

#[cfg(target_arch = "wasm32")]
pub fn classify_response(
    response: freenet_stdlib::client_api::HostResponse,
) -> Option<ClassifiedResponse> {
    use freenet_stdlib::client_api::{ContractResponse, HostResponse};
    use freenet_stdlib::prelude::UpdateData;

    let HostResponse::ContractResponse(response) = response else {
        return None;
    };

    match response {
        ContractResponse::GetResponse { key, state, .. } => {
            if key.id().encode() != TEST_CONTRACT_ID {
                return None;
            }

            let bytes = state.as_ref();
            let should_submit_first_delta = bytes == EMPTY_LEDGER_CBOR;

            Some(ClassifiedResponse {
                contract_status: Some(retrieved_status(bytes)),
                subscription_status: Some(SubscriptionStatus::Active),
                contract_key: Some(key),
                authoritative_state: Some(bytes.to_vec()),
                should_submit_first_delta,
            })
        }

        ContractResponse::UpdateNotification { key, update } => {
            if key.id().encode() != TEST_CONTRACT_ID {
                return None;
            }

            let (status, authoritative_state) = match update {
                UpdateData::State(state) => {
                    let bytes = state.as_ref();

                    (retrieved_status(bytes), Some(bytes.to_vec()))
                }

                UpdateData::Delta(delta) => {
                    let status = if delta.as_ref().is_empty() {
                        ContractProbeStatus::Failed("Received an empty ledger delta.".to_owned())
                    } else {
                        ContractProbeStatus::VerifyingUpdate
                    };

                    (status, None)
                }

                _ => (
                    ContractProbeStatus::Failed(
                        "Received an unsupported contract update type.".to_owned(),
                    ),
                    None,
                ),
            };

            /*
             * A full state notification is authoritative input, not merely a
             * display-status update. Preserve its key and bytes so the normal
             * verification and action-planning path can process it.
             */
            let contract_key = authoritative_state.as_ref().map(|_| key);

            Some(ClassifiedResponse {
                contract_status: Some(status),
                subscription_status: Some(SubscriptionStatus::Active),
                contract_key,
                authoritative_state,
                should_submit_first_delta: false,
            })
        }

        ContractResponse::SubscribeResponse { key, subscribed } => {
            if key.id().encode() != TEST_CONTRACT_ID {
                return None;
            }

            Some(ClassifiedResponse {
                contract_status: None,
                subscription_status: Some(if subscribed {
                    SubscriptionStatus::Active
                } else {
                    SubscriptionStatus::Inactive
                }),
                contract_key: None,
                authoritative_state: None,
                should_submit_first_delta: false,
            })
        }

        ContractResponse::NotFound { instance_id } => {
            if instance_id.encode() != TEST_CONTRACT_ID {
                return None;
            }

            Some(ClassifiedResponse {
                contract_status: Some(ContractProbeStatus::NotFound),
                subscription_status: Some(SubscriptionStatus::Inactive),
                contract_key: None,
                authoritative_state: None,
                should_submit_first_delta: false,
            })
        }

        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ConnectionStatus, ContractProbeStatus, SubscriptionStatus, EMPTY_LEDGER_CBOR,
        EXPECTED_ONE_ACTION_STATE_CBOR, FIRST_CREATE_DELTA_CBOR,
    };

    #[test]
    fn only_closed_or_failed_connections_can_reconnect() {
        assert!(!ConnectionStatus::Connecting.can_reconnect());
        assert!(!ConnectionStatus::Connected.can_reconnect());
        assert!(ConnectionStatus::Disconnected.can_reconnect());
        assert!(ConnectionStatus::Failed("failed".to_owned()).can_reconnect());
    }

    #[test]
    fn statuses_have_stable_user_facing_labels() {
        assert_eq!(
            ConnectionStatus::Connecting.label(),
            "Connecting to Freenet"
        );
        assert_eq!(ConnectionStatus::Connected.label(), "Freenet connected");
        assert_eq!(
            ConnectionStatus::Disconnected.label(),
            "Freenet disconnected"
        );
    }

    #[test]
    fn contract_probe_labels_report_verified_action_counts() {
        let empty = ContractProbeStatus::Retrieved {
            bytes: 10,
            action_count: 0,
        };

        let one = ContractProbeStatus::Retrieved {
            bytes: 516,
            action_count: 1,
        };

        let several = ContractProbeStatus::Retrieved {
            bytes: 900,
            action_count: 3,
        };

        assert_eq!(empty.contract_label(), "Retrieved — 10 bytes");
        assert_eq!(empty.state_label(), "Empty ledger verified");
        assert_eq!(one.contract_label(), "Retrieved — 516 bytes");
        assert_eq!(one.state_label(), "One network action verified");
        assert_eq!(several.state_label(), "3 network actions verified");
    }

    #[test]
    fn subscription_labels_are_stable() {
        assert_eq!(SubscriptionStatus::Pending.label(), "Pending");
        assert_eq!(SubscriptionStatus::Active.label(), "Active");
        assert_eq!(SubscriptionStatus::Inactive.label(), "Inactive");
    }

    #[test]
    fn pinned_first_action_fixtures_match_expected_wire_bytes() {
        assert_eq!(EMPTY_LEDGER_CBOR.len(), 10);
        assert_eq!(FIRST_CREATE_DELTA_CBOR.len(), 514);
        assert_eq!(EXPECTED_ONE_ACTION_STATE_CBOR.len(), 514);
        assert_eq!(FIRST_CREATE_DELTA_CBOR, EXPECTED_ONE_ACTION_STATE_CBOR);
    }
}
