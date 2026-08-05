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

#[cfg(target_arch = "wasm32")]
pub const DEFAULT_NODE_URL: &str =
    "ws://127.0.0.1:7509/v1/contract/command?encodingProtocol=native";

#[cfg(target_arch = "wasm32")]
pub fn connect(
    status_handler: impl Fn(ConnectionStatus) + Clone + 'static,
) -> Result<freenet_stdlib::client_api::WebApi, String> {
    use freenet_stdlib::client_api::WebApi;

    status_handler(ConnectionStatus::Connecting);

    let websocket = web_sys::WebSocket::new(DEFAULT_NODE_URL).map_err(|error| {
        format!(
            "Could not create the Freenet WebSocket for {DEFAULT_NODE_URL}: {error:?}"
        )
    })?;

    let open_status = status_handler.clone();
    let error_status = status_handler.clone();

    Ok(WebApi::start(
        websocket,
        move |result| {
            if let Err(error) = result {
                status_handler(ConnectionStatus::Failed(format!(
                    "The Freenet node returned an error: {error:?}"
                )));
            }
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

#[cfg(test)]
mod tests {
    use super::ConnectionStatus;

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
}
