use backgammon_protocol::{
    decode_presence_revision_request, encode_presence_revision_response, next_reserved_revision,
    PresenceRevisionAction, PresenceRevisionResponse, PresenceRevisionResult,
    PRESENCE_REVISION_PROTOCOL_VERSION,
};
use freenet_stdlib::prelude::*;

const REVISION_SECRET_PREFIX: &[u8] = b"backgammon:presence-revision:v1:";

struct Delegate;

fn revision_key(player_id: &[u8; 32]) -> Vec<u8> {
    let mut key = Vec::with_capacity(REVISION_SECRET_PREFIX.len() + player_id.len());
    key.extend_from_slice(REVISION_SECRET_PREFIX);
    key.extend_from_slice(player_id);
    key
}

fn read_revision(ctx: &mut DelegateCtx, key: &[u8]) -> Result<Option<u64>, String> {
    match ctx.get_secret(key) {
        None => Ok(None),
        Some(bytes) => {
            let raw: [u8; 8] = bytes
                .try_into()
                .map_err(|_| "stored presence revision has invalid length".to_owned())?;
            let revision = u64::from_be_bytes(raw);
            if revision == 0 {
                return Err("stored presence revision zero is invalid".to_owned());
            }
            Ok(Some(revision))
        }
    }
}

fn respond(
    response: PresenceRevisionResponse,
    context: DelegateContext,
) -> Result<Vec<OutboundDelegateMsg>, DelegateError> {
    let payload = encode_presence_revision_response(&response).map_err(DelegateError::Other)?;
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
                "unsupported presence revision delegate message type".to_owned(),
            ));
        };
        let request =
            decode_presence_revision_request(&incoming.payload).map_err(DelegateError::Other)?;
        let key = revision_key(&request.player_id);
        let result = match request.action {
            PresenceRevisionAction::GetReserved => match read_revision(ctx, &key) {
                Ok(value) => PresenceRevisionResult::LastReserved(value),
                Err(error) => PresenceRevisionResult::Error(error),
            },
            PresenceRevisionAction::ReserveNext { observed_revision } => {
                match read_revision(ctx, &key)
                    .and_then(|last| next_reserved_revision(last, observed_revision))
                {
                    Err(error) => PresenceRevisionResult::Error(error),
                    Ok(next) => {
                        let bytes = next.to_be_bytes();
                        if !ctx.set_secret(&key, &bytes) {
                            PresenceRevisionResult::Error(
                                "Freenet secret store rejected presence revision reservation"
                                    .to_owned(),
                            )
                        } else if ctx.get_secret(&key).as_deref() != Some(bytes.as_slice()) {
                            PresenceRevisionResult::Error(
                                "reserved presence revision failed exact readback".to_owned(),
                            )
                        } else {
                            PresenceRevisionResult::Reserved(next)
                        }
                    }
                }
            }
        };
        respond(
            PresenceRevisionResponse {
                version: PRESENCE_REVISION_PROTOCOL_VERSION,
                request_id: request.request_id,
                player_id: request.player_id,
                result,
            },
            incoming.context,
        )
    }
}
