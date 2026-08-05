use backgammon_core::{Player, TurnPhase};
use backgammon_protocol::{replay_game, ActionId, DiceCommit, DiceSecret, GameActionPayload};

use crate::ledger_codec::{build_encoded_action_delta, decode_verified_ledger};
use crate::pending_action::{PendingAction, PendingActionResolution};
use crate::secret_store::verify_dice_secret_commitment;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommitmentPlan {
    NoAction,

    Accepted {
        secret: DiceSecret,
    },

    Submit {
        secret: DiceSecret,
        pending: PendingAction,
        recovered_pending: bool,
    },
}

pub struct CommitmentPlannerInput<'a> {
    pub contract_id: &'a str,
    pub local_player: Player,
    pub authoritative_state: &'a [u8],
    pub pending: Option<&'a PendingAction>,
    pub stored_secret: Option<DiceSecret>,
    pub new_secret: Option<DiceSecret>,
    pub new_action_id: Option<ActionId>,
}

pub fn plan_commitment(input: CommitmentPlannerInput<'_>) -> Result<CommitmentPlan, String> {
    let ledger = decode_verified_ledger(input.authoritative_state)?;
    let replay = replay_game(ledger.typed_actions())
        .map_err(|error| format!("Could not replay verified commitment state: {error:?}"))?;

    if let Some(pending) = input.pending {
        if pending.contract_id != input.contract_id {
            return Err("Stored pending action belongs to another contract.".to_owned());
        }

        let record = pending.verify()?;

        let GameActionPayload::CommitDice {
            turn,
            player,
            commitment,
        } = &record.payload
        else {
            return Err("Stored pending action is not a dice commitment.".to_owned());
        };

        if *player != input.local_player {
            return Err("Stored pending commitment belongs to another player.".to_owned());
        }

        let secret = input
            .stored_secret
            .ok_or_else(|| "Stored pending commitment has no matching local secret.".to_owned())?;

        verify_dice_secret_commitment(&pending.game_id, *turn, *player, commitment, &secret)?;

        return match pending.reconcile(input.authoritative_state)? {
            PendingActionResolution::Accepted => Ok(CommitmentPlan::Accepted { secret }),

            PendingActionResolution::Pending => {
                if *turn != replay.next_turn {
                    return Err(format!(
                        "Stored pending commitment is for turn {turn}, but authoritative turn is {}.",
                        replay.next_turn
                    ));
                }

                Ok(CommitmentPlan::Submit {
                    secret,
                    pending: pending.clone(),
                    recovered_pending: true,
                })
            }
        };
    }

    let accepted_commitment = ledger.typed_actions().iter().find_map(|record| {
        let GameActionPayload::CommitDice {
            turn,
            player,
            commitment,
        } = &record.payload
        else {
            return None;
        };

        (*turn == replay.next_turn && *player == input.local_player).then_some((
            record.game_id,
            *turn,
            *player,
            *commitment,
        ))
    });

    if let Some((game_id, turn, player, commitment)) = accepted_commitment {
        let secret = input
            .stored_secret
            .ok_or_else(|| "Accepted local commitment has no stored secret.".to_owned())?;

        verify_dice_secret_commitment(&game_id, turn, player, &commitment, &secret)?;

        return Ok(CommitmentPlan::Accepted { secret });
    }

    if replay.state.turn_phase != TurnPhase::AwaitingRoll || replay.state.dice.is_some() {
        return Ok(CommitmentPlan::NoAction);
    }

    let white_present = replay.dice_round.white_commitment.is_some();
    let black_present = replay.dice_round.black_commitment.is_some();

    let may_create = match input.local_player {
        Player::White => !white_present && !black_present,

        /*
         * Deterministic ordering prevents both peers from independently
         * constructing different actions for the same next sequence.
         */
        Player::Black => white_present && !black_present,
    };

    if !may_create {
        return Ok(CommitmentPlan::NoAction);
    }

    let secret = input
        .new_secret
        .ok_or_else(|| "Commitment creation requires fresh random secret material.".to_owned())?;

    let action_id = input
        .new_action_id
        .ok_or_else(|| "Commitment creation requires a fresh random action ID.".to_owned())?;

    let commitment = DiceCommit::new(
        &replay.game_id,
        replay.next_turn,
        input.local_player,
        &secret,
    );

    let (record, delta) = build_encoded_action_delta(
        input.authoritative_state,
        action_id,
        GameActionPayload::CommitDice {
            turn: commitment.turn,
            player: commitment.player,
            commitment: commitment.commitment,
        },
    )?;

    if record.sequence != replay.next_sequence {
        return Err(format!(
            "Built commitment sequence {} differs from replay next sequence {}.",
            record.sequence, replay.next_sequence
        ));
    }

    let pending = PendingAction::new(input.contract_id, &record, delta)?;

    Ok(CommitmentPlan::Submit {
        secret,
        pending,
        recovered_pending: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use backgammon_contract::{LedgerState, LedgerStateDelta};
    use ciborium::{de::from_reader, ser::into_writer};

    const CONTRACT_ID: &str = "test-contract";
    const ONE_ACTION_STATE: &[u8] = include_bytes!("../fixtures/expected-one-action-state.cbor");

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

    fn created_plan(
        player: Player,
        state: &[u8],
        secret: DiceSecret,
        action_id: ActionId,
    ) -> CommitmentPlan {
        plan_commitment(CommitmentPlannerInput {
            contract_id: CONTRACT_ID,
            local_player: player,
            authoritative_state: state,
            pending: None,
            stored_secret: None,
            new_secret: Some(secret),
            new_action_id: Some(action_id),
        })
        .unwrap()
    }

    fn pending_from(plan: CommitmentPlan) -> PendingAction {
        match plan {
            CommitmentPlan::Submit {
                pending,
                recovered_pending: false,
                ..
            } => pending,
            other => panic!("expected newly created pending plan, got {other:?}"),
        }
    }

    fn state_with_pending(state: &[u8], pending: &PendingAction) -> Vec<u8> {
        let mut ledger: LedgerState = from_reader(state).unwrap();
        let delta: LedgerStateDelta = from_reader(pending.delta.as_slice()).unwrap();

        ledger
            .actions
            .0
            .extend(delta.actions.expect("pending delta must contain actions"));

        let mut encoded = Vec::new();
        into_writer(&ledger, &mut encoded).unwrap();
        encoded
    }

    #[test]
    fn white_creates_first_commitment_from_verified_replay_state() {
        let plan = created_plan(Player::White, ONE_ACTION_STATE, [11; 32], [21; 32]);

        let pending = pending_from(plan);
        let record = pending.verify().unwrap();

        assert_eq!(record.sequence, 1);

        assert!(matches!(
            record.payload,
            GameActionPayload::CommitDice {
                turn: 0,
                player: Player::White,
                ..
            }
        ));
    }

    #[test]
    fn black_waits_until_white_commitment_is_authoritative() {
        let plan = plan_commitment(CommitmentPlannerInput {
            contract_id: CONTRACT_ID,
            local_player: Player::Black,
            authoritative_state: ONE_ACTION_STATE,
            pending: None,
            stored_secret: None,
            new_secret: Some([22; 32]),
            new_action_id: Some([32; 32]),
        })
        .unwrap();

        assert_eq!(plan, CommitmentPlan::NoAction);
    }

    #[test]
    fn black_creates_next_commitment_after_white_is_accepted() {
        let white_pending = pending_from(created_plan(
            Player::White,
            ONE_ACTION_STATE,
            [11; 32],
            [21; 32],
        ));

        let two_action_state = state_with_pending(ONE_ACTION_STATE, &white_pending);

        let black_pending = pending_from(created_plan(
            Player::Black,
            &two_action_state,
            [22; 32],
            [32; 32],
        ));

        let record = black_pending.verify().unwrap();

        assert_eq!(record.sequence, 2);

        assert!(matches!(
            record.payload,
            GameActionPayload::CommitDice {
                turn: 0,
                player: Player::Black,
                ..
            }
        ));
    }

    #[test]
    fn exact_pending_action_is_retried_without_regeneration() {
        let pending = pending_from(created_plan(
            Player::White,
            ONE_ACTION_STATE,
            [11; 32],
            [21; 32],
        ));

        let plan = plan_commitment(CommitmentPlannerInput {
            contract_id: CONTRACT_ID,
            local_player: Player::White,
            authoritative_state: ONE_ACTION_STATE,
            pending: Some(&pending),
            stored_secret: Some([11; 32]),
            new_secret: Some([99; 32]),
            new_action_id: Some([99; 32]),
        })
        .unwrap();

        assert_eq!(
            plan,
            CommitmentPlan::Submit {
                secret: [11; 32],
                pending,
                recovered_pending: true,
            }
        );
    }

    #[test]
    fn accepted_commitment_recovers_and_verifies_secret() {
        let pending = pending_from(created_plan(
            Player::White,
            ONE_ACTION_STATE,
            [11; 32],
            [21; 32],
        ));

        let accepted_state = state_with_pending(ONE_ACTION_STATE, &pending);

        let plan = plan_commitment(CommitmentPlannerInput {
            contract_id: CONTRACT_ID,
            local_player: Player::White,
            authoritative_state: &accepted_state,
            pending: Some(&pending),
            stored_secret: Some([11; 32]),
            new_secret: None,
            new_action_id: None,
        })
        .unwrap();

        assert_eq!(plan, CommitmentPlan::Accepted { secret: [11; 32] });
    }

    #[test]
    fn accepted_pending_commitment_recovers_after_game_advances() {
        let white_secret = [11; 32];
        let black_secret = [22; 32];

        let white_pending = pending_from(created_plan(
            Player::White,
            ONE_ACTION_STATE,
            white_secret,
            [21; 32],
        ));

        let mut state = state_with_pending(ONE_ACTION_STATE, &white_pending);
        let ledger = decode_verified_ledger(&state).unwrap();
        let game_id = ledger.typed_actions()[0].game_id;

        let black_commit = DiceCommit::new(&game_id, 0, Player::Black, &black_secret);

        state = append_action(
            &state,
            [32; 32],
            GameActionPayload::CommitDice {
                turn: 0,
                player: Player::Black,
                commitment: black_commit.commitment,
            },
        );

        state = append_action(
            &state,
            [33; 32],
            GameActionPayload::RevealDice {
                turn: 0,
                player: Player::White,
                secret: white_secret,
            },
        );

        state = append_action(
            &state,
            [34; 32],
            GameActionPayload::RevealDice {
                turn: 0,
                player: Player::Black,
                secret: black_secret,
            },
        );

        let rolled = decode_verified_ledger(&state).unwrap();
        let replay = replay_game(rolled.typed_actions()).unwrap();
        let sequence = replay.state.legal_turn_sequences().unwrap()[0].clone();

        state = append_action(
            &state,
            [35; 32],
            GameActionPayload::PlayTurn {
                turn: 0,
                player: Player::White,
                sequence,
            },
        );

        let advanced = decode_verified_ledger(&state).unwrap();
        let replay = replay_game(advanced.typed_actions()).unwrap();
        assert_eq!(replay.next_turn, 1);

        let plan = plan_commitment(CommitmentPlannerInput {
            contract_id: CONTRACT_ID,
            local_player: Player::White,
            authoritative_state: &state,
            pending: Some(&white_pending),
            stored_secret: Some(white_secret),
            new_secret: None,
            new_action_id: None,
        })
        .unwrap();

        assert_eq!(
            plan,
            CommitmentPlan::Accepted {
                secret: white_secret,
            }
        );
    }

    #[test]
    fn accepted_commitment_without_pending_record_is_recovered() {
        let pending = pending_from(created_plan(
            Player::White,
            ONE_ACTION_STATE,
            [11; 32],
            [21; 32],
        ));

        let accepted_state = state_with_pending(ONE_ACTION_STATE, &pending);

        let plan = plan_commitment(CommitmentPlannerInput {
            contract_id: CONTRACT_ID,
            local_player: Player::White,
            authoritative_state: &accepted_state,
            pending: None,
            stored_secret: Some([11; 32]),
            new_secret: None,
            new_action_id: None,
        })
        .unwrap();

        assert_eq!(plan, CommitmentPlan::Accepted { secret: [11; 32] });
    }

    #[test]
    fn wrong_player_pending_action_is_rejected() {
        let pending = pending_from(created_plan(
            Player::White,
            ONE_ACTION_STATE,
            [11; 32],
            [21; 32],
        ));

        let error = plan_commitment(CommitmentPlannerInput {
            contract_id: CONTRACT_ID,
            local_player: Player::Black,
            authoritative_state: ONE_ACTION_STATE,
            pending: Some(&pending),
            stored_secret: Some([11; 32]),
            new_secret: None,
            new_action_id: None,
        })
        .unwrap_err();

        assert!(error.contains("another player"));
    }

    #[test]
    fn pending_commitment_without_secret_is_rejected() {
        let pending = pending_from(created_plan(
            Player::White,
            ONE_ACTION_STATE,
            [11; 32],
            [21; 32],
        ));

        let error = plan_commitment(CommitmentPlannerInput {
            contract_id: CONTRACT_ID,
            local_player: Player::White,
            authoritative_state: ONE_ACTION_STATE,
            pending: Some(&pending),
            stored_secret: None,
            new_secret: None,
            new_action_id: None,
        })
        .unwrap_err();

        assert!(error.contains("no matching local secret"));
    }

    #[test]
    fn stale_pending_sequence_fails_closed() {
        let pending = pending_from(created_plan(
            Player::White,
            ONE_ACTION_STATE,
            [11; 32],
            [21; 32],
        ));

        let conflicting_state = append_action(
            ONE_ACTION_STATE,
            [44; 32],
            GameActionPayload::Resign {
                player: Player::White,
            },
        );

        assert!(plan_commitment(CommitmentPlannerInput {
            contract_id: CONTRACT_ID,
            local_player: Player::White,
            authoritative_state: &conflicting_state,
            pending: Some(&pending),
            stored_secret: Some([11; 32]),
            new_secret: None,
            new_action_id: None,
        })
        .is_err());
    }

    #[test]
    fn missing_creation_entropy_is_rejected() {
        let error = plan_commitment(CommitmentPlannerInput {
            contract_id: CONTRACT_ID,
            local_player: Player::White,
            authoritative_state: ONE_ACTION_STATE,
            pending: None,
            stored_secret: None,
            new_secret: None,
            new_action_id: None,
        })
        .unwrap_err();

        assert!(error.contains("fresh random secret"));
    }
}
