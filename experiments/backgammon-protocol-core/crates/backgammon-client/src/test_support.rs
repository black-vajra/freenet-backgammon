use std::sync::OnceLock;

use backgammon_contract::{Actions, LedgerState};
use backgammon_core::{GameState, Player};
use backgammon_protocol::{
    encode_action_signing_message_v4, Action, ActionAuthentication, ActionId, ActionSignature,
    ActionSigningBody, CanonicalReplayState, DiceRoundState, GameActionPayload, GameActionRecord,
    GameConfiguration, PlayerDescriptor, ReplayStatus, GENESIS_STATE_HASH, PROTOCOL_VERSION,
};
use ciborium::ser::into_writer;
use ed25519_dalek::{Signer, SigningKey};

use crate::ledger_codec::build_encoded_signed_action_delta;

pub fn white_signing_key() -> &'static SigningKey {
    static KEY: OnceLock<SigningKey> = OnceLock::new();
    KEY.get_or_init(|| SigningKey::from_bytes(&[41; 32]))
}

pub fn black_signing_key() -> &'static SigningKey {
    static KEY: OnceLock<SigningKey> = OnceLock::new();
    KEY.get_or_init(|| SigningKey::from_bytes(&[42; 32]))
}

pub fn signing_key_for_player(player: Player) -> &'static SigningKey {
    match player {
        Player::White => white_signing_key(),
        Player::Black => black_signing_key(),
    }
}

pub fn configuration() -> GameConfiguration {
    GameConfiguration {
        white: PlayerDescriptor {
            id: white_signing_key().verifying_key().to_bytes(),
            display_name: "White".to_owned(),
        },
        black: PlayerDescriptor {
            id: black_signing_key().verifying_key().to_bytes(),
            display_name: "Black".to_owned(),
        },
        match_length: 1,
    }
}

fn signed_genesis_action() -> Action {
    let game_id = [7; 32];
    let configuration = configuration();

    let resulting_state_hash = CanonicalReplayState::new(
        game_id,
        configuration.clone(),
        GameState::standard_start(),
        0,
        DiceRoundState::default(),
        ReplayStatus::InProgress,
    )
    .hash()
    .expect("test genesis replay state must hash");

    let record = GameActionRecord {
        protocol_version: PROTOCOL_VERSION,
        game_id,
        action_id: [1; 32],
        sequence: 0,
        previous_state_hash: GENESIS_STATE_HASH,
        resulting_state_hash,
        payload: GameActionPayload::CreateGame(configuration),
    };

    let body = ActionSigningBody {
        protocol_version: record.protocol_version,
        game_id: record.game_id,
        action_id: record.action_id,
        sequence: record.sequence,
        previous_state_hash: record.previous_state_hash,
        resulting_state_hash: record.resulting_state_hash,
        payload: record.payload.clone(),
    };

    let message =
        encode_action_signing_message_v4(&body).expect("test genesis signing message must encode");

    let authentication = ActionAuthentication::Genesis {
        white_signature: ActionSignature::from_bytes(white_signing_key().sign(&message).to_bytes()),
        black_signature: ActionSignature::from_bytes(black_signing_key().sign(&message).to_bytes()),
    };

    Action::from_authenticated_game_action_record(&record, authentication)
        .expect("test genesis action must be structurally valid")
}

pub fn one_action_state() -> &'static [u8] {
    static STATE: OnceLock<Vec<u8>> = OnceLock::new();

    STATE
        .get_or_init(|| {
            let state = LedgerState {
                actions: Actions(vec![signed_genesis_action()]),
            };

            let mut encoded = Vec::new();

            into_writer(&state, &mut encoded).expect("test genesis ledger state must encode");

            encoded
        })
        .as_slice()
}

pub fn build_encoded_action_delta(
    state_bytes: &[u8],
    action_id: ActionId,
    payload: GameActionPayload,
) -> Result<(GameActionRecord, Vec<u8>), String> {
    let player = match &payload {
        GameActionPayload::RequestRoll { player, .. }
        | GameActionPayload::CommitDice { player, .. }
        | GameActionPayload::RevealDice { player, .. }
        | GameActionPayload::PlayTurn { player, .. }
        | GameActionPayload::Resign { player } => *player,

        _ => {
            return Err(
                "Test signed-action helper only supports post-genesis player actions.".to_owned(),
            );
        }
    };

    build_encoded_signed_action_delta(
        state_bytes,
        action_id,
        payload,
        signing_key_for_player(player),
    )
}
