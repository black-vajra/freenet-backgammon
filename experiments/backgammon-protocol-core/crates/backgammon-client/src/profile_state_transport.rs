use backgammon_protocol::{
    decode_profile_state_response, encode_profile_state_request, ProfileStateRequest,
    ProfileStateResponse,
};
use freenet_stdlib::client_api::{ClientRequest, DelegateRequest, HostResponse, WebApi};
use freenet_stdlib::prelude::{
    ApplicationMessage, DelegateKey, InboundDelegateMsg, OutboundDelegateMsg,
};

use crate::local_state_transport::LocalStateDelegateHandle;

pub const PROFILE_DELEGATE_WASM_URL: &str = "./backgammon-profile-delegate.wasm";

pub fn classify_profile_delegate_response(
    response: &HostResponse,
    expected_key: &DelegateKey,
) -> Result<Option<Option<ProfileStateResponse>>, String> {
    let HostResponse::DelegateResponse { key, values } = response else {
        return Ok(None);
    };
    if key != expected_key {
        return Ok(None);
    }
    if values.is_empty() {
        return Ok(Some(None));
    }
    let mut application_response = None;
    for value in values {
        let OutboundDelegateMsg::ApplicationMessage(message) = value else {
            continue;
        };
        if application_response.is_some() {
            return Err("profile delegate returned multiple application responses".to_owned());
        }
        application_response = Some(decode_profile_state_response(&message.payload)?);
    }
    application_response
        .map(|value| Some(Some(value)))
        .ok_or_else(|| "profile delegate response contained no application message".to_owned())
}

#[cfg(target_arch = "wasm32")]
pub async fn fetch_profile_delegate_wasm() -> Result<Vec<u8>, String> {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;

    let window = web_sys::window().ok_or_else(|| "browser window is unavailable".to_owned())?;
    let response_value = JsFuture::from(window.fetch_with_str(PROFILE_DELEGATE_WASM_URL))
        .await
        .map_err(|error| format!("failed to fetch profile delegate WASM: {error:?}"))?;
    let response: web_sys::Response = response_value
        .dyn_into()
        .map_err(|_| "profile delegate fetch did not return an HTTP Response".to_owned())?;
    if !response.ok() {
        return Err(format!(
            "profile delegate WASM fetch returned HTTP {}",
            response.status()
        ));
    }
    let array_buffer = response
        .array_buffer()
        .map_err(|error| format!("failed to read profile delegate WASM: {error:?}"))?;
    let array_buffer = JsFuture::from(array_buffer)
        .await
        .map_err(|error| format!("failed to resolve profile delegate WASM: {error:?}"))?;
    Ok(js_sys::Uint8Array::new(&array_buffer).to_vec())
}

#[cfg(target_arch = "wasm32")]
pub async fn send_profile_request(
    api: &mut WebApi,
    handle: &LocalStateDelegateHandle,
    request: ProfileStateRequest,
) -> Result<(), String> {
    let payload = encode_profile_state_request(&request)?;
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
    .map_err(|error| format!("failed to send profile delegate request: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local_state_transport::delegate_handle_from_wasm;

    #[test]
    fn profile_response_routes_only_to_its_delegate_key() {
        let (_, identity) = delegate_handle_from_wasm(vec![1]).unwrap();
        let (_, profile) = delegate_handle_from_wasm(vec![2]).unwrap();
        let response = HostResponse::DelegateResponse {
            key: profile.key.clone(),
            values: Vec::new(),
        };
        assert!(classify_profile_delegate_response(&response, &identity.key)
            .unwrap()
            .is_none());
        assert!(matches!(
            classify_profile_delegate_response(&response, &profile.key).unwrap(),
            Some(None)
        ));
    }
}
