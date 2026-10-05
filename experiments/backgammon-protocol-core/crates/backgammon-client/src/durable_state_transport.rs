use backgammon_protocol::{
    decode_durable_response, encode_durable_request, DurableRequest, DurableResponse, DurableResult,
};
use futures_channel::oneshot;
use std::collections::BTreeMap;
use freenet_stdlib::client_api::{ClientRequest, DelegateRequest, HostResponse, WebApi};
use freenet_stdlib::prelude::{
    ApplicationMessage, DelegateKey, InboundDelegateMsg, OutboundDelegateMsg,
};

use crate::local_state_transport::LocalStateDelegateHandle;

pub const DURABLE_STATE_DELEGATE_WASM_URL: &str = "./backgammon-durable-state-delegate.wasm";

struct PendingReply {
    scope: [u8; 32],
    slot: backgammon_protocol::DurableSlot,
    sender: oneshot::Sender<Result<DurableResult, String>>,
}

/// Owns the reply to each unique request. A reconnect drops pending senders;
/// the awaiting caller then fails closed without publishing anything.
#[derive(Default)]
pub struct DurableReplyRouter {
    pending: BTreeMap<[u8; 32], PendingReply>,
}

impl DurableReplyRouter {
    pub fn register(
        &mut self,
        request: &DurableRequest,
    ) -> Result<oneshot::Receiver<Result<DurableResult, String>>, String> {
        if self.pending.contains_key(&request.request_id) {
            return Err("duplicate durable delegate request ID".to_owned());
        }
        let (sender, receiver) = oneshot::channel();
        self.pending.insert(
            request.request_id,
            PendingReply {
                scope: request.scope,
                slot: request.slot,
                sender,
            },
        );
        Ok(receiver)
    }

    pub fn cancel(&mut self, request_id: &[u8; 32]) {
        self.pending.remove(request_id);
    }

    pub fn clear(&mut self) {
        self.pending.clear();
    }

    pub fn dispatch(&mut self, response: DurableResponse) -> Result<(), String> {
        let Some(pending) = self.pending.remove(&response.request_id) else {
            return Err("unsolicited durable delegate response".to_owned());
        };
        if pending.scope != response.scope || pending.slot != response.slot {
            let _ = pending.sender.send(Err(
                "durable delegate response does not match its request".to_owned(),
            ));
            return Err("durable delegate response does not match its request".to_owned());
        }
        let _ = pending.sender.send(Ok(response.result));
        Ok(())
    }
}

pub fn classify_durable_state_response(
    response: &HostResponse,
    expected_key: &DelegateKey,
) -> Result<Option<Option<DurableResponse>>, String> {
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
            return Err("durable delegate returned multiple application responses".to_owned());
        }
        application_response = Some(decode_durable_response(&message.payload)?);
    }
    application_response
        .map(|value| Some(Some(value)))
        .ok_or_else(|| "durable delegate response contained no application message".to_owned())
}

#[cfg(target_arch = "wasm32")]
pub async fn fetch_durable_state_delegate_wasm() -> Result<Vec<u8>, String> {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;

    let window = web_sys::window().ok_or_else(|| "browser window is unavailable".to_owned())?;
    let response_value = JsFuture::from(window.fetch_with_str(DURABLE_STATE_DELEGATE_WASM_URL))
        .await
        .map_err(|error| format!("failed to fetch durable delegate WASM: {error:?}"))?;
    let response: web_sys::Response = response_value
        .dyn_into()
        .map_err(|_| "durable delegate fetch did not return an HTTP Response".to_owned())?;
    if !response.ok() {
        return Err(format!(
            "durable delegate WASM fetch returned HTTP {}",
            response.status()
        ));
    }
    let buffer = response
        .array_buffer()
        .map_err(|error| format!("failed to read durable delegate WASM: {error:?}"))?;
    let buffer = JsFuture::from(buffer)
        .await
        .map_err(|error| format!("failed to resolve durable delegate WASM: {error:?}"))?;
    Ok(js_sys::Uint8Array::new(&buffer).to_vec())
}

#[cfg(target_arch = "wasm32")]
pub async fn send_durable_state_request(
    api: &mut WebApi,
    handle: &LocalStateDelegateHandle,
    request: &DurableRequest,
) -> Result<(), String> {
    let payload = encode_durable_request(request)?;
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
    .map_err(|error| format!("failed to send durable delegate request: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local_state_transport::delegate_handle_from_wasm;

    #[test]
    fn another_delegate_cannot_satisfy_registration() {
        let (_, durable) = delegate_handle_from_wasm(vec![3]).unwrap();
        let (_, identity) = delegate_handle_from_wasm(vec![4]).unwrap();
        let response = HostResponse::DelegateResponse {
            key: identity.key,
            values: Vec::new(),
        };
        assert!(classify_durable_state_response(&response, &durable.key)
            .unwrap()
            .is_none());
    }

    #[test]
    fn response_must_match_the_request_scope_and_slot() {
        use backgammon_protocol::{DurableAction, DurableSlot, DURABLE_STATE_VERSION};
        let request = DurableRequest {
            version: DURABLE_STATE_VERSION,
            request_id: [1; 32],
            scope: [2; 32],
            slot: DurableSlot::OutboundChallenge,
            action: DurableAction::Get,
        };
        let mut router = DurableReplyRouter::default();
        let _reply = router.register(&request).unwrap();
        assert!(router.register(&request).is_err());
        let wrong = DurableResponse {
            version: DURABLE_STATE_VERSION,
            request_id: request.request_id,
            scope: [3; 32],
            slot: request.slot,
            result: DurableResult::Stored,
        };
        assert!(router.dispatch(wrong).is_err());
    }
}
