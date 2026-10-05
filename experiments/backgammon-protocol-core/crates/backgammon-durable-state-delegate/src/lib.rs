use backgammon_protocol::{
    decode_durable_request, encode_durable_response, DurableAction, DurableResponse,
    DurableResult, DurableSlot, MAX_DURABLE_VALUE_BYTES, DURABLE_STATE_VERSION,
};
use freenet_stdlib::prelude::*;

const PREFIX: &[u8] = b"backgammon:durable-state:v1:";

struct Delegate;

fn key(scope: &[u8; 32], slot: DurableSlot) -> Vec<u8> {
    // Explicit byte mapping is stable even if enum variants are reordered.
    let discriminator = match slot {
        DurableSlot::OutboundChallenge => 1,
        DurableSlot::IncomingAcceptance => 2,
        DurableSlot::GenesisHandshake => 3,
        DurableSlot::DiceSecret => 4,
        DurableSlot::PendingAction => 5,
        DurableSlot::LocalRole => 6,
    };
    let mut key = Vec::with_capacity(PREFIX.len() + 33);
    key.extend_from_slice(PREFIX);
    key.push(discriminator);
    key.extend_from_slice(scope);
    key
}

fn response(
    response: DurableResponse,
    context: DelegateContext,
) -> Result<Vec<OutboundDelegateMsg>, DelegateError> {
    let payload = encode_durable_response(&response).map_err(DelegateError::Other)?;
    Ok(vec![OutboundDelegateMsg::ApplicationMessage(
        ApplicationMessage::new(payload)
            .processed(true)
            .with_context(context),
    )])
}

fn read(ctx: &mut DelegateCtx, key: &[u8]) -> Result<Option<Vec<u8>>, String> {
    let value = ctx.get_secret(key);
    if value
        .as_ref()
        .is_some_and(|v| v.is_empty() || v.len() > MAX_DURABLE_VALUE_BYTES)
    {
        return Err("stored durable record has invalid length".to_owned());
    }
    Ok(value)
}

fn apply(ctx: &mut DelegateCtx, key: &[u8], action: DurableAction) -> DurableResult {
    let old = match read(ctx, key) {
        Ok(old) => old,
        Err(error) => return DurableResult::Error(error),
    };
    match action {
        DurableAction::Get => DurableResult::Value(old),
        DurableAction::Create(value) if old.is_none() => {
            if !ctx.set_secret(key, &value) {
                DurableResult::Error("Freenet secret store rejected durable record".to_owned())
            } else if ctx.get_secret(key).as_deref() != Some(value.as_slice()) {
                DurableResult::Error("durable record failed exact readback".to_owned())
            } else {
                DurableResult::Stored
            }
        }
        DurableAction::Create(_) => DurableResult::Conflict,
        DurableAction::Replace { expected, replacement }
            if old.as_deref() == Some(expected.as_slice()) =>
        {
            if !ctx.set_secret(key, &replacement) {
                DurableResult::Error("Freenet secret store rejected durable update".to_owned())
            } else if ctx.get_secret(key).as_deref() != Some(replacement.as_slice()) {
                DurableResult::Error("durable update failed exact readback".to_owned())
            } else {
                DurableResult::Stored
            }
        }
        DurableAction::Replace { .. } => DurableResult::Conflict,
        DurableAction::Delete { expected } if old.as_deref() == Some(expected.as_slice()) => {
            if !ctx.remove_secret(key) {
                DurableResult::Error("Freenet secret store rejected durable removal".to_owned())
            } else if ctx.has_secret(key) {
                DurableResult::Error("durable record remained after removal".to_owned())
            } else {
                DurableResult::Deleted
            }
        }
        DurableAction::Delete { .. } => DurableResult::Conflict,
    }
}

#[delegate]
impl DelegateInterface for Delegate {
    fn process(
        ctx: &mut DelegateCtx,
        _parameters: Parameters<'static>,
        _origin: Option<MessageOrigin>,
        message: InboundDelegateMsg,
    ) -> Result<Vec<OutboundDelegateMsg>, DelegateError> {
        let InboundDelegateMsg::ApplicationMessage(incoming) = message else {
            return Err(DelegateError::Other(
                "unsupported durable delegate message type".to_owned(),
            ));
        };
        let request = decode_durable_request(&incoming.payload).map_err(DelegateError::Other)?;
        let result = apply(ctx, &key(&request.scope, request.slot), request.action);
        response(
            DurableResponse {
                version: DURABLE_STATE_VERSION,
                request_id: request.request_id,
                scope: request.scope,
                slot: request.slot,
                result,
            },
            incoming.context,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_and_scopes_have_distinct_keys() {
        assert_ne!(key(&[1; 32], DurableSlot::OutboundChallenge), key(&[1; 32], DurableSlot::IncomingAcceptance));
        assert_ne!(key(&[1; 32], DurableSlot::OutboundChallenge), key(&[2; 32], DurableSlot::OutboundChallenge));
    }
}
