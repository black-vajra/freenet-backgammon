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
    Retrieved {
        bytes: usize,
        empty_ledger_verified: bool,
    },
    NotFound,
    Failed(String),
}

impl ContractProbeStatus {
    pub fn contract_label(&self) -> String {
        match self {
            Self::WaitingForConnection => "Waiting for Freenet".to_owned(),
            Self::Requesting => "Retrieving test ledger".to_owned(),
            Self::Retrieved { bytes, .. } => format!("Retrieved — {bytes} bytes"),
            Self::NotFound => "Test ledger not found".to_owned(),
            Self::Failed(_) => "Contract retrieval failed".to_owned(),
        }
    }

    pub fn state_label(&self) -> &str {
        match self {
            Self::Retrieved {
                empty_ledger_verified: true,
                ..
            } => "Empty ledger verified",
            Self::Retrieved {
                empty_ledger_verified: false,
                ..
            } => "Unexpected ledger state",
            Self::Failed(message) => message,
            Self::NotFound => "No state returned",
            Self::WaitingForConnection | Self::Requesting => "Pending",
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

#[cfg(target_arch = "wasm32")]
pub const DEFAULT_NODE_URL: &str =
    "ws://127.0.0.1:7509/v1/contract/command?encodingProtocol=native";

#[cfg(target_arch = "wasm32")]
pub const TEST_CONTRACT_ID: &str = "HA2DEihDKpRuFDAszokohNxWXZvmxyhnvbidDFJnHBCK";

#[cfg(target_arch = "wasm32")]
const EMPTY_LEDGER_CBOR: &[u8] = &[0xa1, 0x67, b'a', b'c', b't', b'i', b'o', b'n', b's', 0x80];

#[cfg(target_arch = "wasm32")]
pub fn connect(
    status_handler: impl Fn(ConnectionStatus) + Clone + 'static,
    response_handler: impl Fn(freenet_stdlib::client_api::HostResponse) + 'static,
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
pub fn classify_response(
    response: freenet_stdlib::client_api::HostResponse,
) -> Option<(Option<ContractProbeStatus>, Option<SubscriptionStatus>)> {
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

            Some((
                Some(ContractProbeStatus::Retrieved {
                    bytes: bytes.len(),
                    empty_ledger_verified: bytes == EMPTY_LEDGER_CBOR,
                }),
                Some(SubscriptionStatus::Active),
            ))
        }

        ContractResponse::UpdateNotification { key, update } => {
            if key.id().encode() != TEST_CONTRACT_ID {
                return None;
            }

            let status = match update {
                UpdateData::State(state) => {
                    let bytes = state.as_ref();

                    ContractProbeStatus::Retrieved {
                        bytes: bytes.len(),
                        empty_ledger_verified: bytes == EMPTY_LEDGER_CBOR,
                    }
                }
                UpdateData::Delta(_) => ContractProbeStatus::Failed(
                    "Received a delta before an initial ledger state.".to_owned(),
                ),
                _ => ContractProbeStatus::Failed(
                    "Received an unsupported contract update type.".to_owned(),
                ),
            };

            Some((Some(status), Some(SubscriptionStatus::Active)))
        }

        ContractResponse::SubscribeResponse { key, subscribed } => {
            if key.id().encode() != TEST_CONTRACT_ID {
                return None;
            }

            Some((
                None,
                Some(if subscribed {
                    SubscriptionStatus::Active
                } else {
                    SubscriptionStatus::Inactive
                }),
            ))
        }

        ContractResponse::NotFound { instance_id } => {
            if instance_id.encode() != TEST_CONTRACT_ID {
                return None;
            }

            Some((
                Some(ContractProbeStatus::NotFound),
                Some(SubscriptionStatus::Inactive),
            ))
        }

        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{ConnectionStatus, ContractProbeStatus, SubscriptionStatus};

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
    fn contract_probe_labels_distinguish_retrieval_and_validation() {
        let valid = ContractProbeStatus::Retrieved {
            bytes: 10,
            empty_ledger_verified: true,
        };

        assert_eq!(valid.contract_label(), "Retrieved — 10 bytes");
        assert_eq!(valid.state_label(), "Empty ledger verified");
    }

    #[test]
    fn subscription_labels_are_stable() {
        assert_eq!(SubscriptionStatus::Pending.label(), "Pending");
        assert_eq!(SubscriptionStatus::Active.label(), "Active");
        assert_eq!(SubscriptionStatus::Inactive.label(), "Inactive");
    }
}
