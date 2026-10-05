use backgammon_protocol::{
    decode_presence_revision_response, encode_presence_revision_request, PresenceRevisionRequest,
    PresenceRevisionResponse,
};
use freenet_stdlib::client_api::{ClientRequest, DelegateRequest, HostResponse, WebApi};
use freenet_stdlib::prelude::{
    ApplicationMessage, DelegateKey, InboundDelegateMsg, OutboundDelegateMsg,
};

use crate::local_state_transport::LocalStateDelegateHandle;

pub const PRESENCE_REVISION_DELEGATE_WASM_URL: &str =
    "./backgammon-presence-revision-delegate.wasm";

pub fn classify_presence_revision_response(
    response: &HostResponse,
    expected_key: &DelegateKey,
) -> Result<Option<Option<PresenceRevisionResponse>>, String> {
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
            return Err("presence revision delegate returned multiple responses".to_owned());
        }
        application_response = Some(decode_presence_revision_response(&message.payload)?);
    }
    application_response
        .map(|value| Some(Some(value)))
        .ok_or_else(|| {
            "presence revision delegate response contained no application message".to_owned()
        })
}

#[cfg(target_arch = "wasm32")]
pub async fn fetch_presence_revision_delegate_wasm() -> Result<Vec<u8>, String> {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;

    let window = web_sys::window().ok_or_else(|| "browser window is unavailable".to_owned())?;
    let response_value = JsFuture::from(window.fetch_with_str(PRESENCE_REVISION_DELEGATE_WASM_URL))
        .await
        .map_err(|error| format!("failed to fetch presence revision WASM: {error:?}"))?;
    let response: web_sys::Response = response_value
        .dyn_into()
        .map_err(|_| "presence revision fetch did not return an HTTP Response".to_owned())?;
    if !response.ok() {
        return Err(format!(
            "presence revision WASM fetch returned HTTP {}",
            response.status()
        ));
    }
    let array_buffer = response
        .array_buffer()
        .map_err(|error| format!("failed to read presence revision WASM: {error:?}"))?;
    let array_buffer = JsFuture::from(array_buffer)
        .await
        .map_err(|error| format!("failed to resolve presence revision WASM: {error:?}"))?;
    Ok(js_sys::Uint8Array::new(&array_buffer).to_vec())
}

#[cfg(target_arch = "wasm32")]
pub async fn send_presence_revision_request(
    api: &mut WebApi,
    handle: &LocalStateDelegateHandle,
    request: PresenceRevisionRequest,
) -> Result<(), String> {
    let payload = encode_presence_revision_request(&request)?;
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
    .map_err(|error| format!("failed to send presence revision request: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local_state_transport::delegate_handle_from_wasm;

    #[test]
    fn another_delegate_response_cannot_satisfy_revision_registration() {
        let (_, revision) = delegate_handle_from_wasm(vec![3]).unwrap();
        let (_, profile) = delegate_handle_from_wasm(vec![4]).unwrap();
        let response = HostResponse::DelegateResponse {
            key: profile.key,
            values: Vec::new(),
        };
        assert!(
            classify_presence_revision_response(&response, &revision.key)
                .unwrap()
                .is_none()
        );
    }
}
