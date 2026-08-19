#![forbid(unsafe_code)]

mod authentication;
mod fair_dice;
mod game_action;
mod replay;
mod state_hash;

pub use authentication::*;
pub use fair_dice::*;
pub use game_action::*;
pub use replay::*;
pub use state_hash::*;

use ciborium::{de::from_reader, ser::into_writer};
use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u16 = 4;
pub const GENESIS_STATE_HASH: StateHash = [0_u8; 32];

pub type GameId = [u8; 32];
pub type ActionId = [u8; 32];
pub type InstanceNonce = [u8; 32];
pub type StateHash = [u8; 32];

#[derive(Serialize, Deserialize, Clone, Default, PartialEq, Eq, Debug)]
pub struct LedgerParameters {
    pub protocol_version: u16,

    /*
     * The exact parameter bytes form part of the Freenet contract identity.
     * A unique nonce permits multiple independent game-ledger instances to
     * use the same verified contract code.
     *
     * The default preserves decoding compatibility with the original
     * protocol-v2 parameter encoding, which contained only protocol_version.
     */
    #[serde(default)]
    pub instance_nonce: InstanceNonce,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct Action {
    pub game_id: GameId,
    pub id: ActionId,
    pub sequence: u32,
    pub previous_state_hash: StateHash,
    pub resulting_state_hash: StateHash,
    pub payload: Vec<u8>,

    /*
     * Protocol-v4 actions require authentication.
     *
     * The default exists only so historical unsigned wire data can still be
     * decoded far enough to fail closed with an explicit missing-
     * authentication error. Verified v4 history never accepts None.
     */
    #[serde(default)]
    pub authentication: Option<ActionAuthentication>,
}

pub fn encode_game_action_payload(payload: &GameActionPayload) -> Result<Vec<u8>, String> {
    payload
        .verify()
        .map_err(|error| format!("invalid typed game-action payload: {error:?}"))?;

    let mut encoded = Vec::new();

    into_writer(payload, &mut encoded)
        .map_err(|error| format!("failed to encode typed game-action payload: {error}"))?;

    Ok(encoded)
}

pub fn decode_game_action_payload(bytes: &[u8]) -> Result<GameActionPayload, String> {
    let payload: GameActionPayload = from_reader(bytes)
        .map_err(|error| format!("failed to decode typed game-action payload: {error}"))?;

    payload
        .verify()
        .map_err(|error| format!("invalid typed game-action payload: {error:?}"))?;

    /*
     * Reject alternate CBOR representations. The exact encoded payload
     * bytes form part of the authenticated and replicated action record.
     */
    let canonical = encode_game_action_payload(&payload)?;

    if canonical != bytes {
        return Err("typed game-action payload is not canonical".into());
    }

    Ok(payload)
}

impl Action {
    pub fn from_game_action_record(record: &GameActionRecord) -> Result<Self, String> {
        record
            .verify()
            .map_err(|error| format!("invalid typed game-action record: {error:?}"))?;

        let sequence = u32::try_from(record.sequence)
            .map_err(|_| "typed action sequence exceeds ledger range")?;

        Ok(Self {
            game_id: record.game_id,
            id: record.action_id,
            sequence,
            previous_state_hash: record.previous_state_hash,
            resulting_state_hash: record.resulting_state_hash,
            payload: encode_game_action_payload(&record.payload)?,
            authentication: None,
        })
    }

    /// Converts a finalized game-action body into a replicated v4 action with
    /// explicit authentication attached.
    pub fn from_authenticated_game_action_record(
        record: &GameActionRecord,
        authentication: ActionAuthentication,
    ) -> Result<Self, String> {
        let mut action = Self::from_game_action_record(record)?;
        authentication.verify_structure()?;
        action.authentication = Some(authentication);
        Ok(action)
    }

    pub fn to_game_action_record(&self) -> Result<GameActionRecord, String> {
        let record = GameActionRecord {
            protocol_version: PROTOCOL_VERSION,
            game_id: self.game_id,
            action_id: self.id,
            sequence: u64::from(self.sequence),
            previous_state_hash: self.previous_state_hash,
            resulting_state_hash: self.resulting_state_hash,
            payload: decode_game_action_payload(&self.payload)?,
        };

        record
            .verify()
            .map_err(|error| format!("invalid typed game-action record: {error:?}"))?;

        Ok(record)
    }
}

pub fn verify_typed_action_history(actions: &[Action]) -> Result<(), String> {
    verify_action_history(actions)?;

    if actions.is_empty() {
        return Ok(());
    }

    let mut ordered: Vec<&Action> = actions.iter().collect();
    ordered.sort_unstable_by_key(|action| action.sequence);

    let records: Vec<GameActionRecord> = ordered
        .iter()
        .map(|action| action.to_game_action_record())
        .collect::<Result<_, _>>()?;

    let configuration = match &records[0].payload {
        GameActionPayload::CreateGame(configuration) => configuration.clone(),
        _ => return Err("first authenticated action must create the game".into()),
    };

    for (action, record) in ordered.iter().zip(records.iter()) {
        let authentication = action.authentication.as_ref().ok_or_else(|| {
            format!(
                "protocol-v4 action {} is missing authentication",
                action.sequence
            )
        })?;

        let signing_body = ActionSigningBody {
            protocol_version: record.protocol_version,
            game_id: record.game_id,
            action_id: record.action_id,
            sequence: record.sequence,
            previous_state_hash: record.previous_state_hash,
            resulting_state_hash: record.resulting_state_hash,
            payload: record.payload.clone(),
        };

        verify_action_authentication_v4(&signing_body, authentication, &configuration).map_err(
            |error| {
                format!(
                    "protocol-v4 action {} failed authentication: {error}",
                    action.sequence
                )
            },
        )?;
    }

    replay_game(&records)
        .map(|_| ())
        .map_err(|error| format!("typed game replay failed: {error:?}"))
}

impl LedgerParameters {
    pub const fn current() -> Self {
        Self::for_instance([0_u8; 32])
    }

    pub const fn for_instance(instance_nonce: InstanceNonce) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            instance_nonce,
        }
    }

    pub fn verify(&self) -> Result<(), String> {
        if self.protocol_version != PROTOCOL_VERSION {
            return Err("unsupported protocol version".into());
        }

        Ok(())
    }
}

pub fn verify_action_sequences(actions: &[Action]) -> Result<(), String> {
    let mut sequences: Vec<u32> = actions.iter().map(|action| action.sequence).collect();
    sequences.sort_unstable();

    for (expected, actual) in sequences.into_iter().enumerate() {
        let expected =
            u32::try_from(expected).map_err(|_| "action sequence exceeds supported range")?;

        if actual < expected {
            return Err("duplicate action sequence".into());
        }

        if actual > expected {
            return Err("action sequence gap".into());
        }
    }

    Ok(())
}

pub fn verify_action_history(actions: &[Action]) -> Result<(), String> {
    verify_action_sequences(actions)?;

    if actions.is_empty() {
        return Ok(());
    }

    let expected_game_id = actions[0].game_id;

    if actions
        .iter()
        .any(|action| action.game_id != expected_game_id)
    {
        return Err("actions belong to different games".into());
    }

    let mut ordered: Vec<&Action> = actions.iter().collect();
    ordered.sort_unstable_by_key(|action| action.sequence);

    if ordered[0].previous_state_hash != GENESIS_STATE_HASH {
        return Err("genesis action has invalid previous-state hash".into());
    }

    for pair in ordered.windows(2) {
        let previous = pair[0];
        let current = pair[1];

        if current.previous_state_hash != previous.resulting_state_hash {
            return Err("action state-hash chain is broken".into());
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    fn game(id: u8) -> GameId {
        [id; 32]
    }

    fn state_hash(id: u8) -> StateHash {
        [id; 32]
    }

    fn action(
        game_id: u8,
        id: u8,
        sequence: u32,
        previous_state_hash: StateHash,
        resulting_state_hash: StateHash,
    ) -> Action {
        Action {
            game_id: game(game_id),
            id: [id; 32],
            sequence,
            previous_state_hash,
            resulting_state_hash,
            payload: vec![id],
            authentication: None,
        }
    }

    fn player(id: u8, name: &str) -> PlayerDescriptor {
        PlayerDescriptor {
            id: [id; 32],
            display_name: name.to_owned(),
        }
    }

    fn configuration() -> GameConfiguration {
        GameConfiguration {
            white: player(1, "White"),
            black: player(2, "Black"),
            match_length: 1,
        }
    }

    fn typed_create_record_with_configuration(
        configuration: GameConfiguration,
    ) -> GameActionRecord {
        let snapshot = CanonicalReplayState::new(
            [7; 32],
            configuration.clone(),
            backgammon_core::GameState::standard_start(),
            0,
            DiceRoundState::default(),
            ReplayStatus::InProgress,
        );

        GameActionRecord {
            protocol_version: PROTOCOL_VERSION,
            game_id: [7; 32],
            action_id: [1; 32],
            sequence: 0,
            previous_state_hash: GENESIS_STATE_HASH,
            resulting_state_hash: snapshot.hash().unwrap(),
            payload: GameActionPayload::CreateGame(configuration),
        }
    }

    fn typed_create_record() -> GameActionRecord {
        typed_create_record_with_configuration(configuration())
    }

    fn deterministic_signing_key(seed_byte: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed_byte; 32])
    }

    fn signed_configuration() -> (GameConfiguration, SigningKey, SigningKey) {
        let white = deterministic_signing_key(41);
        let black = deterministic_signing_key(42);

        let configuration = GameConfiguration {
            white: PlayerDescriptor {
                id: white.verifying_key().to_bytes(),
                display_name: "White".to_owned(),
            },
            black: PlayerDescriptor {
                id: black.verifying_key().to_bytes(),
                display_name: "Black".to_owned(),
            },
            match_length: 1,
        };

        (configuration, white, black)
    }

    fn dual_signed_create_action(
        record: &GameActionRecord,
        white: &SigningKey,
        black: &SigningKey,
    ) -> Action {
        let body = ActionSigningBody {
            protocol_version: record.protocol_version,
            game_id: record.game_id,
            action_id: record.action_id,
            sequence: record.sequence,
            previous_state_hash: record.previous_state_hash,
            resulting_state_hash: record.resulting_state_hash,
            payload: record.payload.clone(),
        };

        let message = encode_action_signing_message_v4(&body).unwrap();

        let authentication = ActionAuthentication::Genesis {
            white_signature: ActionSignature::from_bytes(white.sign(&message).to_bytes()),
            black_signature: ActionSignature::from_bytes(black.sign(&message).to_bytes()),
        };

        Action::from_authenticated_game_action_record(record, authentication).unwrap()
    }

    #[test]
    fn typed_action_round_trip_is_stable() {
        let record = typed_create_record();
        let action = Action::from_game_action_record(&record).unwrap();
        let decoded = action.to_game_action_record().unwrap();

        assert_eq!(decoded, record);
        assert_eq!(
            encode_game_action_payload(&decoded.payload).unwrap(),
            action.payload
        );
    }

    #[test]
    fn malformed_typed_payload_is_rejected() {
        assert!(decode_game_action_payload(&[0x9f, 0x01]).is_err());
    }

    #[test]
    fn typed_create_history_replays_successfully() {
        let (configuration, white, black) = signed_configuration();
        let record = typed_create_record_with_configuration(configuration);
        let action = dual_signed_create_action(&record, &white, &black);

        assert_eq!(verify_typed_action_history(&[action]), Ok(()));
    }

    #[test]
    fn forged_typed_resulting_hash_is_rejected() {
        let (configuration, white, black) = signed_configuration();
        let mut record = typed_create_record_with_configuration(configuration);

        /*
         * Sign the forged finalized body itself. Authentication must therefore
         * succeed before deterministic replay rejects the false resulting
         * state hash.
         */
        record.resulting_state_hash = [99; 32];

        let action = dual_signed_create_action(&record, &white, &black);

        assert!(verify_typed_action_history(&[action])
            .unwrap_err()
            .contains("ResultingStateHashMismatch"));
    }

    #[test]
    fn current_parameters_use_supported_version() {
        let parameters = LedgerParameters::current();

        assert_eq!(parameters.protocol_version, PROTOCOL_VERSION);
        assert_eq!(parameters.instance_nonce, [0_u8; 32]);
        assert_eq!(parameters.verify(), Ok(()));
    }

    #[test]
    fn instance_parameters_preserve_nonce_and_change_encoding() {
        let first = LedgerParameters::for_instance([1_u8; 32]);
        let second = LedgerParameters::for_instance([2_u8; 32]);

        assert_eq!(first.instance_nonce, [1_u8; 32]);
        assert_eq!(second.instance_nonce, [2_u8; 32]);
        assert_eq!(first.verify(), Ok(()));
        assert_eq!(second.verify(), Ok(()));

        let mut first_encoded = Vec::new();
        let mut second_encoded = Vec::new();

        ciborium::ser::into_writer(&first, &mut first_encoded).unwrap();
        ciborium::ser::into_writer(&second, &mut second_encoded).unwrap();

        assert_ne!(first_encoded, second_encoded);
    }

    #[test]
    fn legacy_parameter_encoding_defaults_instance_nonce() {
        #[derive(serde::Serialize)]
        struct LegacyLedgerParameters {
            protocol_version: u16,
        }

        let legacy = LegacyLedgerParameters {
            protocol_version: PROTOCOL_VERSION,
        };

        let mut encoded = Vec::new();
        ciborium::ser::into_writer(&legacy, &mut encoded).unwrap();

        let decoded: LedgerParameters = ciborium::de::from_reader(encoded.as_slice()).unwrap();

        assert_eq!(decoded, LedgerParameters::current());
        assert_eq!(decoded.verify(), Ok(()));
    }

    #[test]
    fn unsupported_version_is_rejected() {
        let parameters = LedgerParameters {
            protocol_version: PROTOCOL_VERSION + 1,
            instance_nonce: [0_u8; 32],
        };

        assert_eq!(
            parameters.verify(),
            Err("unsupported protocol version".into())
        );
    }

    #[test]
    fn contiguous_sequences_are_valid_regardless_of_storage_order() {
        let actions = vec![
            action(7, 3, 2, state_hash(2), state_hash(3)),
            action(7, 1, 0, GENESIS_STATE_HASH, state_hash(1)),
            action(7, 2, 1, state_hash(1), state_hash(2)),
        ];

        assert_eq!(verify_action_sequences(&actions), Ok(()));
    }

    #[test]
    fn empty_history_is_valid() {
        assert_eq!(verify_action_history(&[]), Ok(()));
    }

    #[test]
    fn valid_hash_chain_is_accepted_regardless_of_storage_order() {
        let actions = vec![
            action(7, 3, 2, state_hash(2), state_hash(3)),
            action(7, 1, 0, GENESIS_STATE_HASH, state_hash(1)),
            action(7, 2, 1, state_hash(1), state_hash(2)),
        ];

        assert_eq!(verify_action_history(&actions), Ok(()));
    }

    #[test]
    fn sequence_must_start_at_zero() {
        let actions = vec![action(7, 1, 1, GENESIS_STATE_HASH, state_hash(1))];

        assert_eq!(
            verify_action_history(&actions),
            Err("action sequence gap".into())
        );
    }

    #[test]
    fn duplicate_sequence_is_rejected() {
        let actions = vec![
            action(7, 1, 0, GENESIS_STATE_HASH, state_hash(1)),
            action(7, 2, 0, GENESIS_STATE_HASH, state_hash(2)),
        ];

        assert_eq!(
            verify_action_history(&actions),
            Err("duplicate action sequence".into())
        );
    }

    #[test]
    fn sequence_gap_is_rejected() {
        let actions = vec![
            action(7, 1, 0, GENESIS_STATE_HASH, state_hash(1)),
            action(7, 3, 2, state_hash(1), state_hash(3)),
        ];

        assert_eq!(
            verify_action_history(&actions),
            Err("action sequence gap".into())
        );
    }

    #[test]
    fn mixed_game_ids_are_rejected() {
        let actions = vec![
            action(7, 1, 0, GENESIS_STATE_HASH, state_hash(1)),
            action(8, 2, 1, state_hash(1), state_hash(2)),
        ];

        assert_eq!(
            verify_action_history(&actions),
            Err("actions belong to different games".into())
        );
    }

    #[test]
    fn genesis_action_must_reference_genesis_hash() {
        let actions = vec![action(7, 1, 0, state_hash(9), state_hash(1))];

        assert_eq!(
            verify_action_history(&actions),
            Err("genesis action has invalid previous-state hash".into())
        );
    }

    #[test]
    fn broken_state_hash_chain_is_rejected() {
        let actions = vec![
            action(7, 1, 0, GENESIS_STATE_HASH, state_hash(1)),
            action(7, 2, 1, state_hash(9), state_hash(2)),
        ];

        assert_eq!(
            verify_action_history(&actions),
            Err("action state-hash chain is broken".into())
        );
    }
}
