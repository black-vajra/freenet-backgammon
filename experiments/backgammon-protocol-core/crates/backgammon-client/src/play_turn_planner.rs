use backgammon_core::{Player, TurnPhase, TurnSequence};
use backgammon_protocol::{replay_game, ActionId, GameActionPayload};
use ed25519_dalek::SigningKey;

use crate::ledger_codec::{build_encoded_signed_action_delta, decode_verified_ledger};
use crate::pending_action::{PendingAction, PendingActionResolution};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlayTurnPlan {
    NoAction,

    Accepted,

    Submit {
        pending: PendingAction,
        recovered_pending: bool,
    },
}

pub struct PlayTurnPlannerInput<'a> {
    pub contract_id: &'a str,
    pub local_player: Player,
    pub signing_key: &'a SigningKey,
    pub authoritative_state: &'a [u8],
    pub pending: Option<&'a PendingAction>,

    /// A completed locally selected sequence. `None` means that the player
    /// has not yet completed a turn in the interface.
    pub sequence: Option<&'a TurnSequence>,

    /// Fresh entropy supplied only when a new action is permitted.
    pub new_action_id: Option<ActionId>,
}

pub fn plan_play_turn(input: PlayTurnPlannerInput<'_>) -> Result<PlayTurnPlan, String> {
    let ledger = decode_verified_ledger(input.authoritative_state)?;

    let replay = replay_game(ledger.typed_actions())
        .map_err(|error| format!("Could not replay verified turn state: {error:?}"))?;

    /*
     * A durable pending action always takes precedence over transient UI
     * state. Retry must use the same action ID and exact encoded delta.
     */
    if let Some(pending) = input.pending {
        if pending.contract_id != input.contract_id {
            return Err("Stored pending turn belongs to another contract.".to_owned());
        }

        let record = pending.verify()?;

        let (turn, player) = match &record.payload {
            GameActionPayload::PlayTurn { turn, player, .. } => (*turn, *player),

            _ => {
                return Err("Stored pending action is not a completed game turn.".to_owned());
            }
        };

        if player != input.local_player {
            return Err("Stored pending turn belongs to another player.".to_owned());
        }

        return match pending.reconcile(input.authoritative_state)? {
            PendingActionResolution::Accepted => Ok(PlayTurnPlan::Accepted),

            PendingActionResolution::Pending => {
                if turn != replay.next_turn {
                    return Err(format!(
                        "Stored pending turn is for turn {turn}, but authoritative turn is {}.",
                        replay.next_turn,
                    ));
                }

                if player != replay.state.active_player {
                    return Err(format!(
                        "Stored pending turn belongs to {player:?}, but authoritative player is {:?}.",
                        replay.state.active_player,
                    ));
                }

                if replay.state.turn_phase != TurnPhase::Moving || replay.state.dice.is_none() {
                    return Err(
                        "Stored pending turn does not extend an authoritative rolled state."
                            .to_owned(),
                    );
                }

                /*
                 * Local storage is untrusted. Rebuild the canonical action from
                 * the verified parent and require byte-for-byte equality before
                 * permitting an exact retry.
                 */
                let (expected_record, expected_delta) = build_encoded_signed_action_delta(
                    input.authoritative_state,
                    record.action_id,
                    record.payload.clone(),
                    input.signing_key,
                )?;

                if expected_record != record || expected_delta != pending.delta {
                    return Err(
                        "Stored pending turn differs from the canonical action derived from the authoritative state."
                            .to_owned(),
                    );
                }

                Ok(PlayTurnPlan::Submit {
                    pending: pending.clone(),
                    recovered_pending: true,
                })
            }
        };
    }

    let Some(sequence) = input.sequence else {
        return Ok(PlayTurnPlan::NoAction);
    };

    if replay.state.turn_phase != TurnPhase::Moving || replay.state.dice.is_none() {
        return Err(
            "A completed local turn cannot extend a state without verified rolled dice.".to_owned(),
        );
    }

    if input.local_player != replay.state.active_player {
        return Err(format!(
            "Local player {:?} cannot play the authoritative {:?} turn.",
            input.local_player, replay.state.active_player,
        ));
    }

    let action_id = input
        .new_action_id
        .ok_or_else(|| "Turn submission requires a fresh random action ID.".to_owned())?;

    let (record, delta) = build_encoded_signed_action_delta(
        input.authoritative_state,
        action_id,
        GameActionPayload::PlayTurn {
            turn: replay.next_turn,
            player: input.local_player,
            sequence: sequence.clone(),
        },
        input.signing_key,
    )?;

    if record.sequence != replay.next_sequence {
        return Err(format!(
            "Built turn sequence {} differs from replay next sequence {}.",
            record.sequence, replay.next_sequence,
        ));
    }

    let pending = PendingAction::new(input.contract_id, &record, delta)?;

    Ok(PlayTurnPlan::Submit {
        pending,
        recovered_pending: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::build_encoded_action_delta;

    use backgammon_contract::{LedgerState, LedgerStateDelta};
    use backgammon_protocol::{DiceCommit, DiceSecret};
    use ciborium::{de::from_reader, ser::into_writer};

    const CONTRACT_ID: &str = "test-contract";
    fn one_action_state() -> &'static [u8] {
        crate::test_support::one_action_state()
    }

    fn append_action(
        state_bytes: &[u8],
        action_id: ActionId,
        payload: GameActionPayload,
    ) -> Vec<u8> {
        let (_, delta_bytes) = build_encoded_action_delta(state_bytes, action_id, payload).unwrap();

        let mut state: LedgerState = from_reader(state_bytes).unwrap();
        let delta: LedgerStateDelta = from_reader(delta_bytes.as_slice()).unwrap();

        state
            .actions
            .0
            .extend(delta.actions.expect("delta must contain actions"));

        let mut encoded = Vec::new();
        into_writer(&state, &mut encoded).unwrap();
        encoded
    }

    fn rolled_state() -> Vec<u8> {
        let ledger = decode_verified_ledger(one_action_state()).unwrap();
        let game_id = ledger.typed_actions()[0].game_id;

        let white_secret: DiceSecret = [11; 32];
        let black_secret: DiceSecret = [22; 32];

        let white = DiceCommit::new(&game_id, 0, Player::White, &white_secret);

        let black = DiceCommit::new(&game_id, 0, Player::Black, &black_secret);

        let state = append_action(
            one_action_state(),
            [20; 32],
            GameActionPayload::RequestRoll {
                turn: 0,
                player: Player::White,
            },
        );

        let state = append_action(
            &state,
            [21; 32],
            GameActionPayload::CommitDice {
                turn: 0,
                player: Player::White,
                commitment: white.commitment,
            },
        );

        let state = append_action(
            &state,
            [22; 32],
            GameActionPayload::CommitDice {
                turn: 0,
                player: Player::Black,
                commitment: black.commitment,
            },
        );

        let state = append_action(
            &state,
            [31; 32],
            GameActionPayload::RevealDice {
                turn: 0,
                player: Player::White,
                secret: white_secret,
            },
        );

        append_action(
            &state,
            [32; 32],
            GameActionPayload::RevealDice {
                turn: 0,
                player: Player::Black,
                secret: black_secret,
            },
        )
    }

    fn legal_sequence(state: &[u8]) -> TurnSequence {
        let ledger = decode_verified_ledger(state).unwrap();
        let replay = replay_game(ledger.typed_actions()).unwrap();

        replay.state.legal_turn_sequences().unwrap()[0].clone()
    }

    fn new_pending(state: &[u8]) -> PendingAction {
        let sequence = legal_sequence(state);

        match plan_play_turn(PlayTurnPlannerInput {
            contract_id: CONTRACT_ID,
            local_player: Player::White,
            signing_key: crate::test_support::signing_key_for_player(Player::White),
            authoritative_state: state,
            pending: None,
            sequence: Some(&sequence),
            new_action_id: Some([41; 32]),
        })
        .unwrap()
        {
            PlayTurnPlan::Submit {
                pending,
                recovered_pending: false,
            } => pending,

            other => panic!("expected new pending turn, got {other:?}"),
        }
    }

    fn state_with_pending(state_bytes: &[u8], pending: &PendingAction) -> Vec<u8> {
        let mut state: LedgerState = from_reader(state_bytes).unwrap();

        let delta: LedgerStateDelta = from_reader(pending.delta.as_slice()).unwrap();

        state
            .actions
            .0
            .extend(delta.actions.expect("pending delta must contain actions"));

        let mut encoded = Vec::new();
        into_writer(&state, &mut encoded).unwrap();
        encoded
    }

    #[test]
    fn active_player_builds_canonical_completed_turn() {
        let state = rolled_state();
        let pending = new_pending(&state);
        let record = pending.verify().unwrap();

        assert_eq!(record.sequence, 6);

        match record.payload {
            GameActionPayload::PlayTurn {
                turn,
                player,
                sequence,
            } => {
                assert_eq!(turn, 0);
                assert_eq!(player, Player::White);
                assert_eq!(sequence, legal_sequence(&state));
            }

            other => panic!("expected PlayTurn, got {other:?}"),
        }
    }

    #[test]
    fn no_completed_sequence_produces_no_action() {
        let state = rolled_state();

        let plan = plan_play_turn(PlayTurnPlannerInput {
            contract_id: CONTRACT_ID,
            local_player: Player::White,
            signing_key: crate::test_support::signing_key_for_player(Player::White),
            authoritative_state: &state,
            pending: None,
            sequence: None,
            new_action_id: Some([41; 32]),
        })
        .unwrap();

        assert_eq!(plan, PlayTurnPlan::NoAction);
    }

    #[test]
    fn inactive_player_without_sequence_waits() {
        let state = rolled_state();

        let plan = plan_play_turn(PlayTurnPlannerInput {
            contract_id: CONTRACT_ID,
            local_player: Player::Black,
            signing_key: crate::test_support::signing_key_for_player(Player::Black),
            authoritative_state: &state,
            pending: None,
            sequence: None,
            new_action_id: Some([42; 32]),
        })
        .unwrap();

        assert_eq!(plan, PlayTurnPlan::NoAction);
    }

    #[test]
    fn inactive_player_cannot_submit_completed_sequence() {
        let state = rolled_state();
        let sequence = legal_sequence(&state);

        let error = plan_play_turn(PlayTurnPlannerInput {
            contract_id: CONTRACT_ID,
            local_player: Player::Black,
            signing_key: crate::test_support::signing_key_for_player(Player::Black),
            authoritative_state: &state,
            pending: None,
            sequence: Some(&sequence),
            new_action_id: Some([42; 32]),
        })
        .unwrap_err();

        assert!(error.contains("cannot play"));
    }

    #[test]
    fn unrolled_state_cannot_accept_completed_sequence() {
        let sequence = TurnSequence::default();

        let error = plan_play_turn(PlayTurnPlannerInput {
            contract_id: CONTRACT_ID,
            local_player: Player::White,
            signing_key: crate::test_support::signing_key_for_player(Player::White),
            authoritative_state: one_action_state(),
            pending: None,
            sequence: Some(&sequence),
            new_action_id: Some([41; 32]),
        })
        .unwrap_err();

        assert!(error.contains("verified rolled dice"));
    }

    #[test]
    fn illegal_sequence_is_rejected_by_protocol_replay() {
        let state = rolled_state();
        let illegal = TurnSequence::default();

        assert!(plan_play_turn(PlayTurnPlannerInput {
            contract_id: CONTRACT_ID,
            local_player: Player::White,
            signing_key: crate::test_support::signing_key_for_player(Player::White),
            authoritative_state: &state,
            pending: None,
            sequence: Some(&illegal),
            new_action_id: Some([41; 32]),
        })
        .is_err());
    }

    #[test]
    fn fresh_turn_requires_fresh_action_id() {
        let state = rolled_state();
        let sequence = legal_sequence(&state);

        let error = plan_play_turn(PlayTurnPlannerInput {
            contract_id: CONTRACT_ID,
            local_player: Player::White,
            signing_key: crate::test_support::signing_key_for_player(Player::White),
            authoritative_state: &state,
            pending: None,
            sequence: Some(&sequence),
            new_action_id: None,
        })
        .unwrap_err();

        assert!(error.contains("fresh random action ID"));
    }

    #[test]
    fn exact_pending_turn_is_retried_without_regeneration() {
        let state = rolled_state();
        let pending = new_pending(&state);

        let plan = plan_play_turn(PlayTurnPlannerInput {
            contract_id: CONTRACT_ID,
            local_player: Player::White,
            signing_key: crate::test_support::signing_key_for_player(Player::White),
            authoritative_state: &state,
            pending: Some(&pending),
            sequence: None,
            new_action_id: Some([99; 32]),
        })
        .unwrap();

        assert_eq!(
            plan,
            PlayTurnPlan::Submit {
                pending,
                recovered_pending: true,
            }
        );
    }

    #[test]
    fn accepted_pending_turn_is_reconciled() {
        let state = rolled_state();
        let pending = new_pending(&state);
        let accepted = state_with_pending(&state, &pending);

        let plan = plan_play_turn(PlayTurnPlannerInput {
            contract_id: CONTRACT_ID,
            local_player: Player::White,
            signing_key: crate::test_support::signing_key_for_player(Player::White),
            authoritative_state: &accepted,
            pending: Some(&pending),
            sequence: None,
            new_action_id: None,
        })
        .unwrap();

        assert_eq!(plan, PlayTurnPlan::Accepted);
    }

    #[test]
    fn pending_turn_owned_by_other_player_is_rejected() {
        let state = rolled_state();
        let pending = new_pending(&state);

        let error = plan_play_turn(PlayTurnPlannerInput {
            contract_id: CONTRACT_ID,
            local_player: Player::Black,
            signing_key: crate::test_support::signing_key_for_player(Player::Black),
            authoritative_state: &state,
            pending: Some(&pending),
            sequence: None,
            new_action_id: None,
        })
        .unwrap_err();

        assert!(error.contains("another player"));
    }
}
