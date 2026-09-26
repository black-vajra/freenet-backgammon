use backgammon_protocol::{
    decode_local_state_response, encode_local_state_request, LocalStateRequest, LocalStateResponse,
};
use freenet_stdlib::client_api::{ClientRequest, DelegateRequest, HostResponse, WebApi};
use freenet_stdlib::prelude::{
    ApplicationMessage, Delegate, DelegateCode, DelegateContainer, DelegateKey,
    DelegateWasmAPIVersion, InboundDelegateMsg, OutboundDelegateMsg, Parameters,
};

pub const LOCAL_STATE_DELEGATE_WASM_URL: &str = "./backgammon-local-state-delegate.wasm";

#[derive(Clone, Debug)]
pub struct LocalStateDelegateHandle {
    pub key: DelegateKey,
    pub parameters: Parameters<'static>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LocalStateDelegateResponse {
    Registered,
    Identity(Option<[u8; 32]>),
    IdentityStored,
    IdentityCleared,
    Error(String),
}

impl From<LocalStateResponse> for LocalStateDelegateResponse {
    fn from(response: LocalStateResponse) -> Self {
        match response {
            LocalStateResponse::Identity(identity) => Self::Identity(identity),
            LocalStateResponse::IdentityStored => Self::IdentityStored,
            LocalStateResponse::IdentityCleared => Self::IdentityCleared,
            LocalStateResponse::Error(error) => Self::Error(error),
        }
    }
}

pub fn delegate_handle_from_wasm(
    wasm_bytes: Vec<u8>,
) -> Result<(DelegateContainer, LocalStateDelegateHandle), String> {
    if wasm_bytes.is_empty() {
        return Err("local-state delegate WASM is empty".to_owned());
    }

    let code = DelegateCode::from(wasm_bytes);
    let parameters = Parameters::from(Vec::<u8>::new()).into_owned();
    let delegate = Delegate::from((&code, &parameters));

    let container = DelegateContainer::Wasm(DelegateWasmAPIVersion::V1(delegate));

    let handle = LocalStateDelegateHandle {
        key: container.key().clone(),
        parameters,
    };

    Ok((container, handle))
}

#[cfg(target_arch = "wasm32")]
pub async fn fetch_local_state_delegate_wasm() -> Result<Vec<u8>, String> {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;

    let window = web_sys::window().ok_or_else(|| "browser window is unavailable".to_owned())?;

    let response_value = JsFuture::from(window.fetch_with_str(LOCAL_STATE_DELEGATE_WASM_URL))
        .await
        .map_err(|error| format!("failed to fetch local-state delegate WASM: {error:?}"))?;

    let response: web_sys::Response = response_value
        .dyn_into()
        .map_err(|_| "delegate WASM fetch did not return an HTTP Response".to_owned())?;

    if !response.ok() {
        return Err(format!(
            "local-state delegate WASM fetch returned HTTP {}",
            response.status()
        ));
    }

    let array_buffer = response
        .array_buffer()
        .map_err(|error| format!("failed to read delegate WASM response: {error:?}"))?;

    let array_buffer = JsFuture::from(array_buffer)
        .await
        .map_err(|error| format!("failed to resolve delegate WASM bytes: {error:?}"))?;

    Ok(js_sys::Uint8Array::new(&array_buffer).to_vec())
}

#[cfg(target_arch = "wasm32")]
pub async fn register_local_state_delegate(
    api: &mut WebApi,
    container: DelegateContainer,
) -> Result<(), String> {
    api.send(ClientRequest::DelegateOp(
        DelegateRequest::RegisterDelegate {
            delegate: container,
            /*
             * These legacy fields are structurally required by freenet-stdlib.
             * Current freenet-core ignores them for delegate-secret sealing.
             */
            cipher: [0_u8; 32],
            nonce: [0_u8; 24],
        },
    ))
    .await
    .map_err(|error| format!("failed to register local-state delegate: {error}"))
}

#[cfg(target_arch = "wasm32")]
async fn send_request(
    api: &mut WebApi,
    handle: &LocalStateDelegateHandle,
    request: LocalStateRequest,
) -> Result<(), String> {
    let payload = encode_local_state_request(&request)?;

    api.send(ClientRequest::DelegateOp(
        DelegateRequest::ApplicationMessages {
            key: handle.key.clone(),
            params: handle.parameters.clone(),
            inbound: vec![InboundDelegateMsg::ApplicationMessage(
                ApplicationMessage::new(payload),
            )],
        },
    ))
    .await
    .map_err(|error| format!("failed to send local-state delegate request: {error}"))
}

#[cfg(target_arch = "wasm32")]
pub async fn request_identity(
    api: &mut WebApi,
    handle: &LocalStateDelegateHandle,
) -> Result<(), String> {
    send_request(api, handle, LocalStateRequest::GetIdentity).await
}

#[cfg(target_arch = "wasm32")]
pub async fn store_identity(
    api: &mut WebApi,
    handle: &LocalStateDelegateHandle,
    seed: [u8; 32],
) -> Result<(), String> {
    send_request(api, handle, LocalStateRequest::StoreIdentity(seed)).await
}

#[cfg(target_arch = "wasm32")]
pub async fn clear_identity(
    api: &mut WebApi,
    handle: &LocalStateDelegateHandle,
) -> Result<(), String> {
    send_request(api, handle, LocalStateRequest::ClearIdentity).await
}

pub fn classify_local_state_delegate_response(
    response: &HostResponse,
    expected_key: &DelegateKey,
) -> Result<Option<LocalStateDelegateResponse>, String> {
    let HostResponse::DelegateResponse { key, values } = response else {
        return Ok(None);
    };

    if key != expected_key {
        return Ok(None);
    }

    if values.is_empty() {
        return Ok(Some(LocalStateDelegateResponse::Registered));
    }

    let mut application_response = None;

    for value in values {
        let OutboundDelegateMsg::ApplicationMessage(message) = value else {
            continue;
        };

        if application_response.is_some() {
            return Err("local-state delegate returned multiple application responses".to_owned());
        }

        let decoded = decode_local_state_response(&message.payload)?;
        application_response = Some(decoded.into());
    }

    application_response
        .map(Some)
        .ok_or_else(|| "local-state delegate response contained no application message".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delegate_handle_is_stable_for_same_code_and_parameters() {
        let (_, first) = delegate_handle_from_wasm(vec![0, 97, 115, 109]).unwrap();
        let (_, second) = delegate_handle_from_wasm(vec![0, 97, 115, 109]).unwrap();

        assert_eq!(first.key, second.key);
        assert_eq!(first.parameters, second.parameters);
    }

    #[test]
    fn different_delegate_code_changes_key() {
        let (_, first) = delegate_handle_from_wasm(vec![0, 97, 115, 109]).unwrap();
        let (_, second) = delegate_handle_from_wasm(vec![0, 97, 115, 110]).unwrap();

        assert_ne!(first.key, second.key);
    }

    #[test]
    fn empty_delegate_code_is_rejected() {
        assert!(delegate_handle_from_wasm(Vec::new()).is_err());
    }

    #[test]
    fn empty_matching_delegate_response_is_registration_acknowledgement() {
        let (_, handle) = delegate_handle_from_wasm(vec![0, 97, 115, 109]).unwrap();

        let response = HostResponse::DelegateResponse {
            key: handle.key.clone(),
            values: Vec::new(),
        };

        assert_eq!(
            classify_local_state_delegate_response(&response, &handle.key).unwrap(),
            Some(LocalStateDelegateResponse::Registered)
        );
    }

    #[test]
    fn response_for_different_delegate_is_ignored() {
        let (_, expected) = delegate_handle_from_wasm(vec![0, 97, 115, 109]).unwrap();
        let (_, other) = delegate_handle_from_wasm(vec![0, 97, 115, 110]).unwrap();

        let response = HostResponse::DelegateResponse {
            key: other.key,
            values: Vec::new(),
        };

        assert_eq!(
            classify_local_state_delegate_response(&response, &expected.key).unwrap(),
            None
        );
    }
}
