use backgammon_protocol::{
    decode_local_state_request, encode_local_state_response, LocalStateRequest, LocalStateResponse,
};
use freenet_stdlib::prelude::*;

const IDENTITY_SECRET_KEY: &[u8] = b"backgammon:identity:v1";

struct Delegate;

fn response_message(
    response: LocalStateResponse,
    context: DelegateContext,
) -> Result<Vec<OutboundDelegateMsg>, DelegateError> {
    let payload = encode_local_state_response(&response).map_err(DelegateError::Other)?;

    Ok(vec![OutboundDelegateMsg::ApplicationMessage(
        ApplicationMessage::new(payload)
            .processed(true)
            .with_context(context),
    )])
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
                "unsupported local-state delegate message type".to_owned(),
            ));
        };

        let request =
            decode_local_state_request(&incoming.payload).map_err(DelegateError::Other)?;

        match request {
            LocalStateRequest::GetIdentity => {
                let identity = match ctx.get_secret(IDENTITY_SECRET_KEY) {
                    None => None,
                    Some(bytes) => {
                        let seed: [u8; 32] = bytes.try_into().map_err(|bytes: Vec<u8>| {
                            DelegateError::Other(format!(
                                "stored identity has invalid length {}; expected 32",
                                bytes.len()
                            ))
                        })?;

                        Some(seed)
                    }
                };

                response_message(
                    LocalStateResponse::Identity(identity),
                    incoming.context,
                )
            }

            LocalStateRequest::StoreIdentity(seed) => {
                if !ctx.set_secret(IDENTITY_SECRET_KEY, &seed) {
                    return response_message(
                        LocalStateResponse::Error(
                            "Freenet secret store rejected the identity seed".to_owned(),
                        ),
                        incoming.context,
                    );
                }

                /*
                 * Fail closed: immediately read the value back and require an exact
                 * byte-for-byte match before reporting successful persistence.
                 */
                match ctx.get_secret(IDENTITY_SECRET_KEY) {
                    Some(persisted) if persisted.as_slice() == seed => response_message(
                        LocalStateResponse::IdentityStored,
                        incoming.context,
                    ),
                    _ => response_message(
                        LocalStateResponse::Error(
                            "persisted identity failed exact round-trip verification".to_owned(),
                        ),
                        incoming.context,
                    ),
                }
            }

            LocalStateRequest::ClearIdentity => {
                if ctx.has_secret(IDENTITY_SECRET_KEY)
                    && !ctx.remove_secret(IDENTITY_SECRET_KEY)
                {
                    return response_message(
                        LocalStateResponse::Error(
                            "Freenet secret store rejected identity removal".to_owned(),
                        ),
                        incoming.context,
                    );
                }

                if ctx.has_secret(IDENTITY_SECRET_KEY) {
                    return response_message(
                        LocalStateResponse::Error(
                            "identity remained present after removal".to_owned(),
                        ),
                        incoming.context,
                    );
                }

                response_message(
                    LocalStateResponse::IdentityCleared,
                    incoming.context,
                )
            }
        }
    }
}
