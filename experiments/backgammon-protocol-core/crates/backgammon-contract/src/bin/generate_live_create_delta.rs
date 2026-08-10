use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use backgammon_contract::{Actions, LedgerState, LedgerStateDelta};
use backgammon_core::GameState;
use backgammon_protocol::{
    verify_typed_action_history, Action, CanonicalReplayState, DiceRoundState, GameActionPayload,
    GameActionRecord, GameConfiguration, LedgerParameters, PlayerDescriptor, ReplayStatus,
    GENESIS_STATE_HASH, PROTOCOL_VERSION,
};
use ciborium::{de::from_reader, ser::into_writer};

fn decode_hex_32(label: &str, encoded: &str) -> Result<[u8; 32], String> {
    if encoded.len() != 64 {
        return Err(format!(
            "{label} must contain 64 hexadecimal characters; found {}",
            encoded.len()
        ));
    }

    let bytes = encoded.as_bytes();
    let mut decoded = [0_u8; 32];

    for (index, pair) in bytes.chunks_exact(2).enumerate() {
        let high = decode_nibble(pair[0])
            .ok_or_else(|| format!("{label} is not canonical lowercase hexadecimal"))?;
        let low = decode_nibble(pair[1])
            .ok_or_else(|| format!("{label} is not canonical lowercase hexadecimal"))?;

        decoded[index] = (high << 4) | low;
    }

    Ok(decoded)
}

fn decode_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        _ => None,
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join("")
}

fn encode_to_file<T: serde::Serialize>(
    value: &T,
    path: &Path,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut encoded = Vec::new();
    into_writer(value, &mut encoded)?;
    fs::write(path, &encoded)?;
    Ok(encoded)
}

fn usage(program: &str) -> String {
    format!(
        "usage: {program} OUTPUT_DIRECTORY INSTANCE_NONCE_HEX GAME_ID_HEX \
ACTION_ID_HEX WHITE_PLAYER_ID_HEX WHITE_DISPLAY_NAME \
BLACK_PLAYER_ID_HEX BLACK_DISPLAY_NAME MATCH_LENGTH"
    )
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();

    if args.len() != 10 {
        return Err(usage(&args[0]).into());
    }

    let output_dir = PathBuf::from(&args[1]);
    let instance_nonce = decode_hex_32("INSTANCE_NONCE_HEX", &args[2])?;
    let game_id = decode_hex_32("GAME_ID_HEX", &args[3])?;
    let action_id = decode_hex_32("ACTION_ID_HEX", &args[4])?;
    let white_player_id = decode_hex_32("WHITE_PLAYER_ID_HEX", &args[5])?;
    let white_display_name = args[6].clone();
    let black_player_id = decode_hex_32("BLACK_PLAYER_ID_HEX", &args[7])?;
    let black_display_name = args[8].clone();

    let match_length = args[9]
        .parse::<u16>()
        .map_err(|error| format!("MATCH_LENGTH must be an unsigned integer: {error}"))?;

    fs::create_dir_all(&output_dir)?;

    let parameters = LedgerParameters::for_instance(instance_nonce);
    parameters
        .verify()
        .map_err(|error| format!("parameters failed verification: {error}"))?;

    let empty_state = LedgerState::default();

    let configuration = GameConfiguration {
        white: PlayerDescriptor {
            id: white_player_id,
            display_name: white_display_name,
        },
        black: PlayerDescriptor {
            id: black_player_id,
            display_name: black_display_name,
        },
        match_length,
    };

    configuration
        .verify()
        .map_err(|error| format!("configuration failed verification: {error:?}"))?;

    let canonical_state = CanonicalReplayState::new(
        game_id,
        configuration.clone(),
        GameState::standard_start(),
        0,
        DiceRoundState::default(),
        ReplayStatus::InProgress,
    );

    let resulting_state_hash = canonical_state
        .hash()
        .map_err(|error| format!("canonical state hashing failed: {error:?}"))?;

    let record = GameActionRecord {
        protocol_version: PROTOCOL_VERSION,
        game_id,
        action_id,
        sequence: 0,
        previous_state_hash: GENESIS_STATE_HASH,
        resulting_state_hash,
        payload: GameActionPayload::CreateGame(configuration),
    };

    record
        .verify()
        .map_err(|error| format!("typed action record failed verification: {error:?}"))?;

    let action = Action::from_game_action_record(&record)?;

    verify_typed_action_history(std::slice::from_ref(&action))?;

    let delta = LedgerStateDelta {
        actions: Some(vec![action.clone()]),
    };

    let expected_state = LedgerState {
        actions: Actions(vec![action]),
    };

    verify_typed_action_history(&expected_state.actions.0)?;

    let parameters_path = output_dir.join("ledger-parameters-v3.cbor");
    let empty_state_path = output_dir.join("empty-ledger-state-v3.cbor");
    let delta_path = output_dir.join("create-game-sequence-0.delta.cbor");
    let expected_state_path = output_dir.join("expected-one-action-state.cbor");
    let canonical_state_path = output_dir.join("canonical-replay-state-v3.cbor");

    let parameter_bytes = encode_to_file(&parameters, &parameters_path)?;
    let empty_state_bytes = encode_to_file(&empty_state, &empty_state_path)?;
    let delta_bytes = encode_to_file(&delta, &delta_path)?;
    let expected_state_bytes = encode_to_file(&expected_state, &expected_state_path)?;
    let canonical_state_bytes = encode_to_file(&canonical_state, &canonical_state_path)?;

    let decoded_parameters: LedgerParameters = from_reader(parameter_bytes.as_slice())?;
    let decoded_empty_state: LedgerState = from_reader(empty_state_bytes.as_slice())?;
    let decoded_delta: LedgerStateDelta = from_reader(delta_bytes.as_slice())?;
    let decoded_state: LedgerState = from_reader(expected_state_bytes.as_slice())?;
    let decoded_canonical: CanonicalReplayState = from_reader(canonical_state_bytes.as_slice())?;

    if decoded_parameters != parameters {
        return Err("parameter CBOR failed exact typed round-trip".into());
    }

    if decoded_parameters.instance_nonce != instance_nonce {
        return Err("decoded instance nonce differs from requested nonce".into());
    }

    if decoded_empty_state != empty_state {
        return Err("empty ledger CBOR failed exact typed round-trip".into());
    }

    if decoded_delta != delta {
        return Err("CreateGame delta CBOR failed exact typed round-trip".into());
    }

    if decoded_state != expected_state {
        return Err("expected state CBOR failed exact typed round-trip".into());
    }

    if decoded_canonical != canonical_state {
        return Err("canonical replay state CBOR failed exact typed round-trip".into());
    }

    decoded_parameters
        .verify()
        .map_err(|error| format!("decoded parameters failed verification: {error}"))?;

    verify_typed_action_history(&decoded_state.actions.0)?;

    println!("Generated and locally verified live CreateGame bundle.");
    println!("protocol_version={PROTOCOL_VERSION}");
    println!("instance_nonce={}", hex(&instance_nonce));
    println!("game_id={}", hex(&game_id));
    println!("action_id={}", hex(&action_id));
    println!("white_player_id={}", hex(&white_player_id));
    println!("white_display_name={}", args[6]);
    println!("black_player_id={}", hex(&black_player_id));
    println!("black_display_name={}", args[8]);
    println!("match_length={match_length}");
    println!("sequence=0");
    println!("previous_state_hash={}", hex(&GENESIS_STATE_HASH));
    println!("resulting_state_hash={}", hex(&resulting_state_hash));
    println!("parameter_bytes={}", parameter_bytes.len());
    println!("empty_state_bytes={}", empty_state_bytes.len());
    println!("delta_bytes={}", delta_bytes.len());
    println!("expected_state_bytes={}", expected_state_bytes.len());
    println!("canonical_state_bytes={}", canonical_state_bytes.len());
    println!("parameters={}", parameters_path.display());
    println!("empty_state={}", empty_state_path.display());
    println!("delta={}", delta_path.display());
    println!("expected_state={}", expected_state_path.display());
    println!("canonical_state={}", canonical_state_path.display());

    Ok(())
}
