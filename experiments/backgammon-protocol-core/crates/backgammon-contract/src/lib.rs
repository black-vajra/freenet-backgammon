use backgammon_protocol::{verify_typed_action_history, Action, LedgerParameters};
use ciborium::{de::from_reader, ser::into_writer};
use freenet_scaffold_macro::composable;
use freenet_stdlib::prelude::*;
use serde::{Deserialize, Serialize};

const MAX_ACTIONS: usize = 256;
const MAX_PAYLOAD_BYTES: usize = 1024;

#[derive(Serialize, Deserialize, Clone, Default, PartialEq, Eq, Debug)]
pub struct Actions(pub Vec<Action>);

impl Actions {
    fn canonicalize(&mut self) {
        self.0.sort_by(|a, b| a.id.cmp(&b.id));
        self.0.dedup();
    }

    fn verify_inner(&self) -> Result<(), String> {
        if self.0.len() > MAX_ACTIONS {
            return Err("action limit exceeded".into());
        }
        for pair in self.0.windows(2) {
            if pair[0].id >= pair[1].id {
                return Err("actions are not in canonical unique-ID order".into());
            }
        }
        if self.0.iter().any(|a| a.payload.len() > MAX_PAYLOAD_BYTES) {
            return Err("action payload limit exceeded".into());
        }
        Ok(())
    }
}

impl ComposableState for Actions {
    type ParentState = LedgerState;
    type Summary = Vec<[u8; 32]>;
    type Delta = Vec<Action>;
    type Parameters = LedgerParameters;

    fn verify(
        &self,
        _parent: &Self::ParentState,
        parameters: &Self::Parameters,
    ) -> Result<(), String> {
        parameters.verify()?;
        self.verify_inner()?;
        verify_typed_action_history(&self.0)
    }

    fn summarize(
        &self,
        _parent: &Self::ParentState,
        _parameters: &Self::Parameters,
    ) -> Self::Summary {
        self.0.iter().map(|a| a.id).collect()
    }

    fn delta(
        &self,
        _parent: &Self::ParentState,
        _parameters: &Self::Parameters,
        old: &Self::Summary,
    ) -> Option<Self::Delta> {
        let missing: Vec<_> = self
            .0
            .iter()
            .filter(|a| old.binary_search(&a.id).is_err())
            .cloned()
            .collect();
        (!missing.is_empty()).then_some(missing)
    }

    fn apply_delta(
        &mut self,
        _parent: &Self::ParentState,
        _parameters: &Self::Parameters,
        delta: &Option<Self::Delta>,
    ) -> Result<(), String> {
        if let Some(incoming) = delta {
            for action in incoming {
                if let Some(existing) = self.0.iter().find(|a| a.id == action.id) {
                    if existing != action {
                        return Err("conflicting actions share an ID".into());
                    }
                } else {
                    self.0.push(action.clone());
                }
            }
            self.canonicalize();
            self.verify_inner()?;
        }
        Ok(())
    }
}

#[composable]
#[derive(Serialize, Deserialize, Clone, Default, PartialEq, Eq, Debug)]
pub struct LedgerState {
    pub actions: Actions,
}

struct Contract;

#[derive(Clone, Debug, PartialEq)]
enum DecodedUpdate {
    Delta(LedgerStateDelta),
    State(LedgerState),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ApplyUpdatesError {
    InvalidUpdate,
    InvalidState,
}

fn apply_decoded_updates(
    parameters: &LedgerParameters,
    mut current: LedgerState,
    updates: impl IntoIterator<Item = DecodedUpdate>,
) -> Result<LedgerState, ApplyUpdatesError> {
    for update in updates {
        match update {
            DecodedUpdate::Delta(delta) => {
                let parent = current.clone();

                current
                    .apply_delta(&parent, parameters, &Some(delta))
                    .map_err(|_| ApplyUpdatesError::InvalidUpdate)?;
            }

            DecodedUpdate::State(incoming) => {
                let parent = current.clone();

                current
                    .merge(&parent, parameters, &incoming)
                    .map_err(|_| ApplyUpdatesError::InvalidUpdate)?;
            }
        }
    }

    current
        .verify(&current, parameters)
        .map_err(|_| ApplyUpdatesError::InvalidState)?;

    Ok(current)
}

fn decode<T: for<'de> Deserialize<'de>>(bytes: &[u8]) -> Result<T, ContractError> {
    from_reader(bytes).map_err(|e| ContractError::Deser(e.to_string()))
}

fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, ContractError> {
    let mut output = Vec::new();
    into_writer(value, &mut output).map_err(|e| ContractError::Deser(e.to_string()))?;
    Ok(output)
}

#[contract]
impl ContractInterface for Contract {
    fn validate_state(
        parameters: Parameters<'static>,
        state: State<'static>,
        _related: RelatedContracts<'static>,
    ) -> Result<ValidateResult, ContractError> {
        if state.as_ref().is_empty() {
            return Ok(ValidateResult::Valid);
        }
        let parameters: LedgerParameters = decode(parameters.as_ref())?;
        let state: LedgerState = decode(state.as_ref())?;
        state
            .verify(&state, &parameters)
            .map(|_| ValidateResult::Valid)
            .map_err(|_| ContractError::InvalidState)
    }

    fn update_state(
        parameters: Parameters<'static>,
        state: State<'static>,
        data: Vec<UpdateData<'static>>,
    ) -> Result<UpdateModification<'static>, ContractError> {
        let parameters: LedgerParameters = decode(parameters.as_ref())?;
        let current: LedgerState = decode(state.as_ref())?;
        let mut updates = Vec::with_capacity(data.len());

        for update in data {
            match update {
                UpdateData::Delta(bytes) => {
                    updates.push(DecodedUpdate::Delta(decode(bytes.as_ref())?));
                }

                UpdateData::State(bytes) => {
                    updates.push(DecodedUpdate::State(decode(bytes.as_ref())?));
                }

                _ => return Err(ContractError::InvalidUpdate),
            }
        }

        let current =
            apply_decoded_updates(&parameters, current, updates).map_err(|error| match error {
                ApplyUpdatesError::InvalidUpdate => ContractError::InvalidUpdate,
                ApplyUpdatesError::InvalidState => ContractError::InvalidState,
            })?;

        Ok(UpdateModification::valid(encode(&current)?.into()))
    }

    fn summarize_state(
        parameters: Parameters<'static>,
        state: State<'static>,
    ) -> Result<StateSummary<'static>, ContractError> {
        if state.as_ref().is_empty() {
            return Ok(StateSummary::from(Vec::new()));
        }
        let parameters: LedgerParameters = decode(parameters.as_ref())?;
        let state: LedgerState = decode(state.as_ref())?;
        Ok(StateSummary::from(encode(
            &state.summarize(&state, &parameters),
        )?))
    }

    fn get_state_delta(
        parameters: Parameters<'static>,
        state: State<'static>,
        summary: StateSummary<'static>,
    ) -> Result<StateDelta<'static>, ContractError> {
        let parameters: LedgerParameters = decode(parameters.as_ref())?;
        let state: LedgerState = decode(state.as_ref())?;
        let summary: LedgerStateSummary = decode(summary.as_ref())?;
        Ok(StateDelta::from(encode(&state.delta(
            &state,
            &parameters,
            &summary,
        ))?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use backgammon_core::{GameState, Player, TurnPhase, TurnSequence};
    use backgammon_protocol::{
        derive_dice, replay_game, CanonicalReplayState, DiceCommit, GameActionPayload,
        GameActionRecord, GameConfiguration, PlayerDescriptor, ReplayStatus, StateHash,
        GENESIS_STATE_HASH, PROTOCOL_VERSION,
    };

    fn params() -> LedgerParameters {
        LedgerParameters {
            protocol_version: PROTOCOL_VERSION,
        }
    }

    fn state_hash(id: u8) -> StateHash {
        [id; 32]
    }

    fn configuration() -> GameConfiguration {
        GameConfiguration {
            white: PlayerDescriptor {
                id: [1; 32],
                display_name: "White".to_owned(),
            },
            black: PlayerDescriptor {
                id: [2; 32],
                display_name: "Black".to_owned(),
            },
            match_length: 1,
        }
    }

    fn create_hash() -> StateHash {
        CanonicalReplayState::new(
            [7; 32],
            configuration(),
            GameState::standard_start(),
            0,
            backgammon_protocol::DiceRoundState::default(),
            ReplayStatus::InProgress,
        )
        .hash()
        .unwrap()
    }

    fn resignation_hash() -> StateHash {
        CanonicalReplayState::new(
            [7; 32],
            configuration(),
            GameState::standard_start(),
            0,
            backgammon_protocol::DiceRoundState::default(),
            ReplayStatus::Resigned {
                resigned: Player::White,
                winner: Player::Black,
            },
        )
        .hash()
        .unwrap()
    }

    fn typed_action(
        id: u8,
        sequence: u32,
        previous_state_hash: StateHash,
        resulting_state_hash: StateHash,
        payload: GameActionPayload,
    ) -> Action {
        Action::from_game_action_record(&GameActionRecord {
            protocol_version: PROTOCOL_VERSION,
            game_id: [7; 32],
            action_id: [id; 32],
            sequence: u64::from(sequence),
            previous_state_hash,
            resulting_state_hash,
            payload,
        })
        .unwrap()
    }

    fn action(id: u8, sequence: u32) -> Action {
        match sequence {
            0 => typed_action(
                id,
                0,
                GENESIS_STATE_HASH,
                create_hash(),
                GameActionPayload::CreateGame(configuration()),
            ),
            1 => typed_action(
                id,
                1,
                create_hash(),
                resignation_hash(),
                GameActionPayload::Resign {
                    player: Player::White,
                },
            ),
            _ => Action {
                game_id: [7; 32],
                id: [id; 32],
                sequence,
                previous_state_hash: state_hash(sequence as u8),
                resulting_state_hash: state_hash((sequence + 1) as u8),
                payload: vec![id],
            },
        }
    }

    fn complete_opening_turn_actions() -> (Vec<Action>, GameState, StateHash) {
        let game_id = [7; 32];
        let white_secret = [11; 32];
        let black_secret = [22; 32];

        let white_commit = DiceCommit::new(&game_id, 0, Player::White, &white_secret);

        let black_commit = DiceCommit::new(&game_id, 0, Player::Black, &black_secret);

        let dice = derive_dice(&game_id, 0, &white_secret, &black_secret).unwrap();

        let initial_state = GameState::standard_start();

        let mut rolled_state = initial_state.clone();
        rolled_state.dice = Some(dice);
        rolled_state.turn_phase = TurnPhase::Moving;

        let sequence = rolled_state.legal_turn_sequences().unwrap()[0].clone();

        let state_hash =
            |state: &GameState, next_turn: u32, dice_round: backgammon_protocol::DiceRoundState| {
                CanonicalReplayState::new(
                    game_id,
                    configuration(),
                    state.clone(),
                    next_turn,
                    dice_round,
                    ReplayStatus::InProgress,
                )
                .hash()
                .unwrap()
            };

        let create = action(1, 0);

        let mut white_committed = backgammon_protocol::DiceRoundState::default();

        white_committed.white_commitment = Some(white_commit.commitment);

        let white_commit_hash = state_hash(&initial_state, 0, white_committed.clone());

        let white_commit_action = typed_action(
            2,
            1,
            create_hash(),
            white_commit_hash,
            GameActionPayload::CommitDice {
                turn: 0,
                player: Player::White,
                commitment: white_commit.commitment,
            },
        );

        let mut both_committed = white_committed;
        both_committed.black_commitment = Some(black_commit.commitment);

        let black_commit_hash = state_hash(&initial_state, 0, both_committed.clone());

        let black_commit_action = typed_action(
            3,
            2,
            white_commit_hash,
            black_commit_hash,
            GameActionPayload::CommitDice {
                turn: 0,
                player: Player::Black,
                commitment: black_commit.commitment,
            },
        );

        let mut white_revealed = both_committed;
        white_revealed.white_reveal = Some(white_secret);

        let white_reveal_hash = state_hash(&initial_state, 0, white_revealed.clone());

        let white_reveal_action = typed_action(
            4,
            3,
            black_commit_hash,
            white_reveal_hash,
            GameActionPayload::RevealDice {
                turn: 0,
                player: Player::White,
                secret: white_secret,
            },
        );

        let mut both_revealed = white_revealed;
        both_revealed.black_reveal = Some(black_secret);

        let roll_hash = state_hash(&rolled_state, 0, both_revealed);

        let black_reveal_action = typed_action(
            5,
            4,
            white_reveal_hash,
            roll_hash,
            GameActionPayload::RevealDice {
                turn: 0,
                player: Player::Black,
                secret: black_secret,
            },
        );

        let mut completed_state = rolled_state;
        completed_state.apply_turn_sequence(&sequence).unwrap();

        let completed_hash = state_hash(
            &completed_state,
            1,
            backgammon_protocol::DiceRoundState::default(),
        );

        let play = typed_action(
            6,
            5,
            roll_hash,
            completed_hash,
            GameActionPayload::PlayTurn {
                turn: 0,
                player: Player::White,
                sequence,
            },
        );

        (
            vec![
                create,
                white_commit_action,
                black_commit_action,
                white_reveal_action,
                black_reveal_action,
                play,
            ],
            completed_state,
            completed_hash,
        )
    }

    fn action_delta(actions: Vec<Action>) -> DecodedUpdate {
        DecodedUpdate::Delta(LedgerStateDelta {
            actions: Some(actions),
        })
    }

    fn encoded<T: Serialize>(value: &T) -> Vec<u8> {
        encode(value).unwrap()
    }

    #[test]
    fn real_actions_apply_as_separate_verified_updates() {
        let p = params();
        let (actions, expected_state, expected_hash) = complete_opening_turn_actions();

        let mut state = LedgerState::default();

        for action in actions {
            state = apply_decoded_updates(&p, state, [action_delta(vec![action])]).unwrap();
        }

        assert_eq!(state.verify(&state, &p), Ok(()));

        let mut records: Vec<_> = state
            .actions
            .0
            .iter()
            .map(Action::to_game_action_record)
            .collect::<Result<_, _>>()
            .unwrap();

        records.sort_by_key(|record| record.sequence);

        let replayed = replay_game(&records).unwrap();

        assert_eq!(replayed.state, expected_state);
        assert_eq!(replayed.latest_state_hash, expected_hash);
        assert_eq!(replayed.next_sequence, 6);
        assert_eq!(replayed.next_turn, 1);
    }

    #[test]
    fn combined_delta_matches_separate_verified_updates() {
        let p = params();
        let (actions, _, _) = complete_opening_turn_actions();

        let combined =
            apply_decoded_updates(&p, LedgerState::default(), [action_delta(actions.clone())])
                .unwrap();

        let mut separate = LedgerState::default();

        for action in actions {
            separate = apply_decoded_updates(&p, separate, [action_delta(vec![action])]).unwrap();
        }

        assert_eq!(combined, separate);
    }

    #[test]
    fn different_valid_delivery_groupings_converge() {
        let p = params();
        let (actions, expected_state, expected_hash) = complete_opening_turn_actions();

        assert_eq!(actions.len(), 6);

        let grouping_a = apply_decoded_updates(
            &p,
            LedgerState::default(),
            [
                action_delta(vec![actions[0].clone()]),
                action_delta(vec![actions[1].clone(), actions[2].clone()]),
                action_delta(vec![
                    actions[3].clone(),
                    actions[4].clone(),
                    actions[5].clone(),
                ]),
            ],
        )
        .unwrap();

        let grouping_b = apply_decoded_updates(
            &p,
            LedgerState::default(),
            [
                action_delta(vec![actions[0].clone(), actions[1].clone()]),
                action_delta(vec![actions[2].clone(), actions[3].clone()]),
                action_delta(vec![actions[4].clone(), actions[5].clone()]),
            ],
        )
        .unwrap();

        let combined =
            apply_decoded_updates(&p, LedgerState::default(), [action_delta(actions)]).unwrap();

        assert_eq!(grouping_a, combined);
        assert_eq!(grouping_b, combined);
        assert_eq!(combined.verify(&combined, &p), Ok(()));

        let mut records: Vec<_> = combined
            .actions
            .0
            .iter()
            .map(Action::to_game_action_record)
            .collect::<Result<_, _>>()
            .unwrap();

        records.sort_by_key(|record| record.sequence);

        let replayed = replay_game(&records).unwrap();

        assert_eq!(replayed.state, expected_state);
        assert_eq!(replayed.latest_state_hash, expected_hash);
        assert_eq!(replayed.next_sequence, 6);
        assert_eq!(replayed.next_turn, 1);
    }

    #[test]
    fn incomplete_history_can_merge_but_not_be_accepted() {
        let p = params();
        let (actions, _, _) = complete_opening_turn_actions();
        let mut partial = LedgerState::default();

        partial
            .apply_delta(
                &partial.clone(),
                &p,
                &Some(LedgerStateDelta {
                    actions: Some(vec![actions[1].clone()]),
                }),
            )
            .unwrap();

        assert_eq!(partial.actions.0.len(), 1);
        assert!(partial.verify(&partial, &p).is_err());

        assert_eq!(
            apply_decoded_updates(
                &p,
                LedgerState::default(),
                [action_delta(vec![actions[1].clone()])],
            ),
            Err(ApplyUpdatesError::InvalidState)
        );
    }

    #[test]
    fn forged_action_is_rejected_through_update_path() {
        let p = params();
        let (mut actions, _, _) = complete_opening_turn_actions();

        actions[2].resulting_state_hash = [99; 32];

        assert_eq!(
            apply_decoded_updates(&p, LedgerState::default(), [action_delta(actions)],),
            Err(ApplyUpdatesError::InvalidState)
        );
    }

    #[test]
    fn summary_delta_synchronizes_missing_game_actions() {
        let p = params();
        let (mut actions, expected_state, expected_hash) = complete_opening_turn_actions();

        actions.sort_by(|left, right| left.id.cmp(&right.id));

        let full = LedgerState {
            actions: Actions(actions.clone()),
        };

        let mut client = LedgerState {
            actions: Actions(vec![actions[0].clone()]),
        };

        let client_summary = client.summarize(&client, &p);
        let delta = full.delta(&full, &p, &client_summary);

        client.apply_delta(&client.clone(), &p, &delta).unwrap();

        assert_eq!(client, full);
        assert_eq!(client.verify(&client, &p), Ok(()));

        let mut records: Vec<_> = client
            .actions
            .0
            .iter()
            .map(Action::to_game_action_record)
            .collect::<Result<_, _>>()
            .unwrap();

        records.sort_by_key(|record| record.sequence);

        let replayed = replay_game(&records).unwrap();

        assert_eq!(replayed.state, expected_state);
        assert_eq!(replayed.latest_state_hash, expected_hash);
        assert_eq!(replayed.next_sequence, 6);
        assert_eq!(replayed.next_turn, 1);
    }

    #[test]
    fn state_delta_contains_only_missing_actions() {
        let p = params();
        let (mut actions, _, _) = complete_opening_turn_actions();

        actions.sort_by(|left, right| left.id.cmp(&right.id));

        let full = LedgerState {
            actions: Actions(actions.clone()),
        };

        let client = LedgerState {
            actions: Actions(vec![actions[0].clone()]),
        };

        let summary = client.summarize(&client, &p);
        let delta = full
            .delta(&full, &p, &summary)
            .expect("client is missing five actions");

        let missing = delta
            .actions
            .expect("actions component must contain a delta");

        assert_eq!(missing.len(), actions.len() - 1);
        assert_eq!(missing, actions[1..].to_vec());
    }

    #[test]
    fn current_summary_produces_no_state_delta() {
        let p = params();
        let (mut actions, _, _) = complete_opening_turn_actions();

        actions.sort_by(|left, right| left.id.cmp(&right.id));

        let full = LedgerState {
            actions: Actions(actions),
        };

        let summary = full.summarize(&full, &p);

        assert_eq!(full.delta(&full, &p, &summary), None);
    }

    #[test]
    fn opposite_update_orders_converge() {
        let p = params();
        let empty = LedgerState::default();
        let da = LedgerStateDelta {
            actions: Some(vec![action(1, 0)]),
        };
        let db = LedgerStateDelta {
            actions: Some(vec![action(2, 1)]),
        };
        let mut ab = empty.clone();
        ab.apply_delta(&ab.clone(), &p, &Some(da.clone())).unwrap();
        ab.apply_delta(&ab.clone(), &p, &Some(db.clone())).unwrap();
        let mut ba = empty;
        ba.apply_delta(&ba.clone(), &p, &Some(db)).unwrap();
        ba.apply_delta(&ba.clone(), &p, &Some(da)).unwrap();
        assert_eq!(ab, ba);
    }

    #[test]
    fn duplicate_is_idempotent() {
        let p = params();
        let d = LedgerStateDelta {
            actions: Some(vec![action(1, 0)]),
        };
        let mut state = LedgerState::default();
        state
            .apply_delta(&state.clone(), &p, &Some(d.clone()))
            .unwrap();
        let once = state.clone();
        state.apply_delta(&state.clone(), &p, &Some(d)).unwrap();
        assert_eq!(state, once);
    }

    #[test]
    fn same_id_with_different_content_is_rejected() {
        let p = params();
        let mut state = LedgerState::default();
        state
            .apply_delta(
                &state.clone(),
                &p,
                &Some(LedgerStateDelta {
                    actions: Some(vec![action(1, 0)]),
                }),
            )
            .unwrap();
        let mut conflicting = action(1, 99);
        conflicting.payload = vec![9];
        assert!(state
            .apply_delta(
                &state.clone(),
                &p,
                &Some(LedgerStateDelta {
                    actions: Some(vec![conflicting])
                }),
            )
            .is_err());
    }

    #[test]
    fn malformed_cbor_is_rejected() {
        let malformed = [0x9f, 0x01];
        assert!(decode::<LedgerState>(&malformed).is_err());
    }

    #[test]
    fn unsupported_protocol_version_is_rejected() {
        let state = LedgerState::default();
        let unsupported = LedgerParameters {
            protocol_version: PROTOCOL_VERSION + 1,
        };
        assert_eq!(
            state.verify(&state, &unsupported),
            Err("unsupported protocol version".into())
        );
    }

    #[test]
    fn final_state_rejects_sequence_starting_after_zero() {
        let state = LedgerState {
            actions: Actions(vec![action(1, 1)]),
        };

        assert_eq!(
            state.verify(&state, &params()),
            Err("action sequence gap".into())
        );
    }

    #[test]
    fn final_state_rejects_duplicate_sequences() {
        let state = LedgerState {
            actions: Actions(vec![action(1, 0), action(2, 0)]),
        };

        assert_eq!(
            state.verify(&state, &params()),
            Err("duplicate action sequence".into())
        );
    }

    #[test]
    fn final_state_rejects_sequence_gaps() {
        let state = LedgerState {
            actions: Actions(vec![action(1, 0), action(3, 2)]),
        };

        assert_eq!(
            state.verify(&state, &params()),
            Err("action sequence gap".into())
        );
    }

    #[test]
    fn final_state_accepts_contiguous_sequences_in_id_order() {
        let state = LedgerState {
            actions: Actions(vec![action(1, 1), action(2, 0)]),
        };

        assert_eq!(state.verify(&state, &params()), Ok(()));
    }

    #[test]
    fn oversized_payload_is_rejected() {
        let mut state = LedgerState::default();
        let mut oversized = action(1, 0);
        oversized.payload = vec![0; MAX_PAYLOAD_BYTES + 1];
        state.actions.0.push(oversized);
        assert_eq!(
            state.verify(&state, &params()),
            Err("action payload limit exceeded".into())
        );
    }

    #[test]
    fn oversized_ledger_is_rejected() {
        let mut state = LedgerState::default();
        for n in 0..=MAX_ACTIONS {
            let mut id = [0_u8; 32];
            id[28..].copy_from_slice(&(n as u32).to_be_bytes());
            state.actions.0.push(Action {
                game_id: [7; 32],
                id,
                sequence: n as u32,
                previous_state_hash: GENESIS_STATE_HASH,
                resulting_state_hash: [0; 32],
                payload: Vec::new(),
            });
        }
        assert_eq!(
            state.verify(&state, &params()),
            Err("action limit exceeded".into())
        );
    }

    #[test]
    fn noncanonical_state_is_rejected() {
        let state = LedgerState {
            actions: Actions(vec![action(2, 1), action(1, 0)]),
        };
        assert_eq!(
            state.verify(&state, &params()),
            Err("actions are not in canonical unique-ID order".into())
        );
    }

    #[test]
    fn full_state_merge_orders_converge() {
        let p = params();
        let left = LedgerState {
            actions: Actions(vec![action(1, 0)]),
        };
        let right = LedgerState {
            actions: Actions(vec![action(2, 1)]),
        };

        let mut left_then_right = left.clone();
        left_then_right
            .merge(&left_then_right.clone(), &p, &right)
            .unwrap();

        let mut right_then_left = right;
        right_then_left
            .merge(&right_then_left.clone(), &p, &left)
            .unwrap();

        assert_eq!(left_then_right, right_then_left);
        assert_eq!(left_then_right.actions.0.len(), 2);
    }

    #[test]
    fn complete_legal_turn_is_accepted_and_reconstructed() {
        let (mut actions, expected_state, expected_hash) = complete_opening_turn_actions();

        actions.sort_by(|left, right| left.id.cmp(&right.id));

        let state = LedgerState {
            actions: Actions(actions.clone()),
        };

        assert_eq!(state.verify(&state, &params()), Ok(()));

        let mut ordered = actions;
        ordered.sort_by_key(|action| action.sequence);

        let records: Vec<_> = ordered
            .iter()
            .map(Action::to_game_action_record)
            .collect::<Result<_, _>>()
            .unwrap();

        let replayed = replay_game(&records).unwrap();

        assert_eq!(replayed.state, expected_state);
        assert_eq!(replayed.latest_state_hash, expected_hash);
        assert_eq!(replayed.next_sequence, 6);
        assert_eq!(replayed.next_turn, 1);
        assert_eq!(replayed.state.active_player, Player::Black);
        assert_eq!(replayed.state.turn_phase, TurnPhase::AwaitingRoll);
        assert_eq!(replayed.state.dice, None);
    }

    #[test]
    fn malformed_typed_payload_is_rejected_by_contract_state() {
        let mut malformed = action(1, 0);
        malformed.payload = vec![0x9f, 0x01];

        let state = LedgerState {
            actions: Actions(vec![malformed]),
        };

        assert!(state.verify(&state, &params()).is_err());
    }

    #[test]
    fn forged_canonical_state_hash_is_rejected_by_contract_state() {
        let mut forged = action(1, 0);
        forged.resulting_state_hash = [99; 32];

        let state = LedgerState {
            actions: Actions(vec![forged]),
        };

        assert!(state
            .verify(&state, &params())
            .unwrap_err()
            .contains("ResultingStateHashMismatch"));
    }

    #[test]
    fn turn_without_roll_is_rejected_by_contract_state() {
        let create = action(1, 0);

        let illegal_turn = typed_action(
            2,
            1,
            create_hash(),
            [88; 32],
            GameActionPayload::PlayTurn {
                turn: 0,
                player: Player::White,
                sequence: TurnSequence::default(),
            },
        );

        let mut actions = vec![create, illegal_turn];
        actions.sort_by(|left, right| left.id.cmp(&right.id));

        let state = LedgerState {
            actions: Actions(actions),
        };

        assert!(state
            .verify(&state, &params())
            .unwrap_err()
            .contains("RollExpected"));
    }

    #[test]
    fn valid_typed_create_and_resignation_are_accepted() {
        let mut actions = vec![action(1, 0), action(2, 1)];
        actions.sort_by(|left, right| left.id.cmp(&right.id));

        let state = LedgerState {
            actions: Actions(actions),
        };

        assert_eq!(state.verify(&state, &params()), Ok(()));
    }

    #[test]
    fn cbor_round_trip_is_stable() {
        let state = LedgerState {
            actions: Actions(vec![action(1, 0), action(2, 1)]),
        };
        let first = encoded(&state);
        let decoded: LedgerState = decode(&first).unwrap();
        let second = encoded(&decoded);
        assert_eq!(decoded, state);
        assert_eq!(first, second);
    }
}
