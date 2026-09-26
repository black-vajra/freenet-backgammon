use backgammon_protocol::{
    decode_profile_state_request, encode_profile_state_response, validate_display_name,
    ProfileStateAction, ProfileStateResponse, ProfileStateResult, PROFILE_STATE_PROTOCOL_VERSION,
};
use freenet_stdlib::prelude::*;

const PROFILE_SECRET_PREFIX: &[u8] = b"backgammon:profile:v1:";

struct Delegate;

fn profile_key(player_id: &[u8; 32]) -> Vec<u8> {
    let mut key = Vec::with_capacity(PROFILE_SECRET_PREFIX.len() + player_id.len());
    key.extend_from_slice(PROFILE_SECRET_PREFIX);
    key.extend_from_slice(player_id);
    key
}

fn respond(
    response: ProfileStateResponse,
    context: DelegateContext,
) -> Result<Vec<OutboundDelegateMsg>, DelegateError> {
    let payload = encode_profile_state_response(&response).map_err(DelegateError::Other)?;
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
                "unsupported profile delegate message type".to_owned(),
            ));
        };
        let request =
            decode_profile_state_request(&incoming.payload).map_err(DelegateError::Other)?;
        let key = profile_key(&request.player_id);
        let result = match request.action {
            ProfileStateAction::Get => match ctx.get_secret(&key) {
                None => ProfileStateResult::DisplayName(None),
                Some(bytes) => match String::from_utf8(bytes) {
                    Ok(name) => match validate_display_name(&name) {
                        Ok(()) => ProfileStateResult::DisplayName(Some(name)),
                        Err(_) => {
                            ProfileStateResult::Error("stored profile name is invalid".into())
                        }
                    },
                    Err(_) => ProfileStateResult::Error("stored profile name is not UTF-8".into()),
                },
            },
            ProfileStateAction::Store(name) => {
                if !ctx.set_secret(&key, name.as_bytes()) {
                    ProfileStateResult::Error(
                        "Freenet secret store rejected the profile name".into(),
                    )
                } else if ctx.get_secret(&key).as_deref() != Some(name.as_bytes()) {
                    ProfileStateResult::Error("persisted profile name failed exact readback".into())
                } else {
                    ProfileStateResult::Stored
                }
            }
        };
        respond(
            ProfileStateResponse {
                version: PROFILE_STATE_PROTOCOL_VERSION,
                request_id: request.request_id,
                player_id: request.player_id,
                result,
            },
            incoming.context,
        )
    }
}
