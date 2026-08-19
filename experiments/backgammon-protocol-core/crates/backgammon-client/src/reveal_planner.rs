use backgammon_core::{Player, TurnPhase};
use backgammon_protocol::{replay_game, ActionId, DiceSecret, GameActionPayload};
use ed25519_dalek::SigningKey;

use crate::ledger_codec::{build_encoded_signed_action_delta, decode_verified_ledger};
use crate::pending_action::{PendingAction, PendingActionResolution};
use crate::secret_store::verify_dice_secret_commitment;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RevealPlan {
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

pub struct RevealPlannerInput<'a> {
    pub contract_id: &'a str,
    pub local_player: Player,
    pub signing_key: &'a SigningKey,
    pub authoritative_state: &'a [u8],
    pub pending: Option<&'a PendingAction>,
    pub stored_secret: Option<DiceSecret>,
    pub new_action_id: Option<ActionId>,
}

pub fn plan_reveal(input: RevealPlannerInput<'_>) -> Result<RevealPlan, String> {
    let ledger = decode_verified_ledger(input.authoritative_state)?;

    let replay = replay_game(ledger.typed_actions())
        .map_err(|error| format!("Could not replay verified reveal state: {error:?}"))?;

    if let Some(pending) = input.pending {
        if pending.contract_id != input.contract_id {
            return Err("Stored pending action belongs to another contract.".to_owned());
        }

        let record = pending.verify()?;

        let GameActionPayload::RevealDice {
            turn,
            player,
            secret,
        } = &record.payload
        else {
            return Err("Stored pending action is not a dice reveal.".to_owned());
        };

        if *player != input.local_player {
            return Err("Stored pending reveal belongs to another player.".to_owned());
        }

        let stored_secret = input
            .stored_secret
            .ok_or_else(|| "Stored pending reveal has no matching local secret.".to_owned())?;

        if stored_secret != *secret {
            return Err("Stored pending reveal differs from the local dice secret.".to_owned());
        }

        verify_local_commitment(
            ledger.typed_actions(),
            &pending.game_id,
            *turn,
            *player,
            &stored_secret,
        )?;

        return match pending.reconcile(input.authoritative_state)? {
            PendingActionResolution::Accepted => Ok(RevealPlan::Accepted {
                secret: stored_secret,
            }),

            PendingActionResolution::Pending => {
                if *turn != replay.next_turn {
                    return Err(format!(
                        "Stored pending reveal is for turn {turn}, \
                         but authoritative turn is {}.",
                        replay.next_turn,
                    ));
                }

                Ok(RevealPlan::Submit {
                    secret: stored_secret,
                    pending: pending.clone(),
                    recovered_pending: true,
                })
            }
        };
    }

    let accepted_reveal = ledger.typed_actions().iter().find_map(|record| {
        let GameActionPayload::RevealDice {
            turn,
            player,
            secret,
        } = &record.payload
        else {
            return None;
        };

        (*turn == replay.next_turn && *player == input.local_player).then_some((
            record.game_id,
            *turn,
            *player,
            *secret,
        ))
    });

    if let Some((game_id, turn, player, accepted_secret)) = accepted_reveal {
        let stored_secret = input
            .stored_secret
            .ok_or_else(|| "Accepted local reveal has no stored secret.".to_owned())?;

        if stored_secret != accepted_secret {
            return Err("Accepted local reveal differs from the stored secret.".to_owned());
        }

        verify_local_commitment(
            ledger.typed_actions(),
            &game_id,
            turn,
            player,
            &stored_secret,
        )?;

        return Ok(RevealPlan::Accepted {
            secret: stored_secret,
        });
    }

    if replay.state.turn_phase != TurnPhase::AwaitingRoll || replay.state.dice.is_some() {
        return Ok(RevealPlan::NoAction);
    }

    let white_commitment = replay.dice_round.white_commitment.is_some();

    let black_commitment = replay.dice_round.black_commitment.is_some();

    if !white_commitment || !black_commitment {
        return Ok(RevealPlan::NoAction);
    }

    let white_reveal = replay.dice_round.white_reveal.is_some();
    let black_reveal = replay.dice_round.black_reveal.is_some();

    let may_create = match input.local_player {
        /*
         * Deterministic reveal order prevents both peers from creating
         * conflicting actions for the same next sequence.
         */
        Player::White => !white_reveal && !black_reveal,
        Player::Black => white_reveal && !black_reveal,
    };

    if !may_create {
        return Ok(RevealPlan::NoAction);
    }

    let secret = input
        .stored_secret
        .ok_or_else(|| "Reveal creation requires the committed local secret.".to_owned())?;

    verify_local_commitment(
        ledger.typed_actions(),
        &replay.game_id,
        replay.next_turn,
        input.local_player,
        &secret,
    )?;

    let action_id = input
        .new_action_id
        .ok_or_else(|| "Reveal creation requires a fresh random action ID.".to_owned())?;

    let (record, delta) = build_encoded_signed_action_delta(
        input.authoritative_state,
        action_id,
        GameActionPayload::RevealDice {
            turn: replay.next_turn,
            player: input.local_player,
            secret,
        },
        input.signing_key,
    )?;

    if record.sequence != replay.next_sequence {
        return Err(format!(
            "Built reveal sequence {} differs from replay next \
             sequence {}.",
            record.sequence, replay.next_sequence,
        ));
    }

    let pending = PendingAction::new(input.contract_id, &record, delta)?;

    Ok(RevealPlan::Submit {
        secret,
        pending,
        recovered_pending: false,
    })
}

fn verify_local_commitment(
    actions: &[backgammon_protocol::GameActionRecord],
    game_id: &[u8; 32],
    turn: u32,
    player: Player,
    secret: &DiceSecret,
) -> Result<(), String> {
    let commitment = actions.iter().find_map(|record| {
        let GameActionPayload::CommitDice {
            turn: action_turn,
            player: action_player,
            commitment,
        } = &record.payload
        else {
            return None;
        };

        (*action_turn == turn && *action_player == player).then_some(*commitment)
    });

    let commitment =
        commitment.ok_or_else(|| "Local reveal has no accepted matching commitment.".to_owned())?;

    verify_dice_secret_commitment(game_id, turn, player, &commitment, secret)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::build_encoded_action_delta;

    use backgammon_contract::{LedgerState, LedgerStateDelta};
    use backgammon_protocol::DiceCommit;
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

    fn committed_state() -> Vec<u8> {
        let ledger = decode_verified_ledger(one_action_state()).unwrap();

        let game_id = ledger.typed_actions()[0].game_id;

        let white = DiceCommit::new(&game_id, 0, Player::White, &[11; 32]);

        let requested = append_action(
            one_action_state(),
            [20; 32],
            GameActionPayload::RequestRoll {
                turn: 0,
                player: Player::White,
            },
        );

        let with_white = append_action(
            &requested,
            [21; 32],
            GameActionPayload::CommitDice {
                turn: 0,
                player: Player::White,
                commitment: white.commitment,
            },
        );

        let black = DiceCommit::new(&game_id, 0, Player::Black, &[22; 32]);

        append_action(
            &with_white,
            [22; 32],
            GameActionPayload::CommitDice {
                turn: 0,
                player: Player::Black,
                commitment: black.commitment,
            },
        )
    }

    fn newly_created(
        player: Player,
        state: &[u8],
        secret: DiceSecret,
        action_id: ActionId,
    ) -> RevealPlan {
        plan_reveal(RevealPlannerInput {
            contract_id: CONTRACT_ID,
            local_player: player,
            signing_key: crate::test_support::signing_key_for_player(player),
            authoritative_state: state,
            pending: None,
            stored_secret: Some(secret),
            new_action_id: Some(action_id),
        })
        .unwrap()
    }

    fn pending_from(plan: RevealPlan) -> PendingAction {
        match plan {
            RevealPlan::Submit {
                pending,
                recovered_pending: false,
                ..
            } => pending,

            other => {
                panic!("expected newly created reveal, got {other:?}")
            }
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
    fn reveal_waits_for_both_commitments() {
        let plan = plan_reveal(RevealPlannerInput {
            contract_id: CONTRACT_ID,
            local_player: Player::White,
            signing_key: crate::test_support::signing_key_for_player(Player::White),
            authoritative_state: one_action_state(),
            pending: None,
            stored_secret: Some([11; 32]),
            new_action_id: Some([31; 32]),
        })
        .unwrap();

        assert_eq!(plan, RevealPlan::NoAction);
    }

    #[test]
    fn white_creates_first_reveal() {
        let state = committed_state();

        let pending = pending_from(newly_created(Player::White, &state, [11; 32], [31; 32]));

        let record = pending.verify().unwrap();

        assert_eq!(record.sequence, 4);

        assert_eq!(
            record.payload,
            GameActionPayload::RevealDice {
                turn: 0,
                player: Player::White,
                secret: [11; 32],
            },
        );
    }

    #[test]
    fn black_waits_for_authoritative_white_reveal() {
        let state = committed_state();

        let plan = newly_created(Player::Black, &state, [22; 32], [32; 32]);

        assert_eq!(plan, RevealPlan::NoAction);
    }

    #[test]
    fn black_creates_second_reveal() {
        let state = committed_state();

        let white = pending_from(newly_created(Player::White, &state, [11; 32], [31; 32]));

        let four_action_state = state_with_pending(&state, &white);

        let black = pending_from(newly_created(
            Player::Black,
            &four_action_state,
            [22; 32],
            [32; 32],
        ));

        let record = black.verify().unwrap();

        assert_eq!(record.sequence, 5);

        assert_eq!(
            record.payload,
            GameActionPayload::RevealDice {
                turn: 0,
                player: Player::Black,
                secret: [22; 32],
            },
        );
    }

    #[test]
    fn exact_pending_reveal_is_retried() {
        let state = committed_state();

        let pending = pending_from(newly_created(Player::White, &state, [11; 32], [31; 32]));

        let plan = plan_reveal(RevealPlannerInput {
            contract_id: CONTRACT_ID,
            local_player: Player::White,
            signing_key: crate::test_support::signing_key_for_player(Player::White),
            authoritative_state: &state,
            pending: Some(&pending),
            stored_secret: Some([11; 32]),
            new_action_id: Some([99; 32]),
        })
        .unwrap();

        assert_eq!(
            plan,
            RevealPlan::Submit {
                secret: [11; 32],
                pending,
                recovered_pending: true,
            },
        );
    }

    #[test]
    fn accepted_pending_reveal_is_reconciled() {
        let state = committed_state();

        let pending = pending_from(newly_created(Player::White, &state, [11; 32], [31; 32]));

        let accepted = state_with_pending(&state, &pending);

        let plan = plan_reveal(RevealPlannerInput {
            contract_id: CONTRACT_ID,
            local_player: Player::White,
            signing_key: crate::test_support::signing_key_for_player(Player::White),
            authoritative_state: &accepted,
            pending: Some(&pending),
            stored_secret: Some([11; 32]),
            new_action_id: None,
        })
        .unwrap();

        assert_eq!(plan, RevealPlan::Accepted { secret: [11; 32] },);
    }

    #[test]
    fn accepted_reveal_without_pending_is_recovered() {
        let state = committed_state();

        let pending = pending_from(newly_created(Player::White, &state, [11; 32], [31; 32]));

        let accepted = state_with_pending(&state, &pending);

        let plan = plan_reveal(RevealPlannerInput {
            contract_id: CONTRACT_ID,
            local_player: Player::White,
            signing_key: crate::test_support::signing_key_for_player(Player::White),
            authoritative_state: &accepted,
            pending: None,
            stored_secret: Some([11; 32]),
            new_action_id: None,
        })
        .unwrap();

        assert_eq!(plan, RevealPlan::Accepted { secret: [11; 32] },);
    }

    #[test]
    fn wrong_secret_is_rejected() {
        let state = committed_state();

        let error = plan_reveal(RevealPlannerInput {
            contract_id: CONTRACT_ID,
            local_player: Player::White,
            signing_key: crate::test_support::signing_key_for_player(Player::White),
            authoritative_state: &state,
            pending: None,
            stored_secret: Some([99; 32]),
            new_action_id: Some([31; 32]),
        })
        .unwrap_err();

        assert!(error.contains("does not match the accepted network commitment"));
    }

    #[test]
    fn wrong_player_pending_reveal_is_rejected() {
        let state = committed_state();

        let pending = pending_from(newly_created(Player::White, &state, [11; 32], [31; 32]));

        assert!(plan_reveal(RevealPlannerInput {
            contract_id: CONTRACT_ID,
            local_player: Player::Black,
            signing_key: crate::test_support::signing_key_for_player(Player::Black),
            authoritative_state: &state,
            pending: Some(&pending),
            stored_secret: Some([22; 32]),
            new_action_id: None,
        })
        .is_err());
    }

    #[test]
    fn second_reveal_derives_dice_in_replay() {
        let state = committed_state();

        let white = pending_from(newly_created(Player::White, &state, [11; 32], [31; 32]));

        let with_white = state_with_pending(&state, &white);

        let black = pending_from(newly_created(
            Player::Black,
            &with_white,
            [22; 32],
            [32; 32],
        ));

        let complete = state_with_pending(&with_white, &black);

        let ledger = decode_verified_ledger(&complete).unwrap();

        let replay = replay_game(ledger.typed_actions()).unwrap();

        assert_eq!(replay.next_sequence, 6);
        assert_eq!(replay.next_turn, 0);
        assert!(replay.state.dice.is_some());
        assert_eq!(replay.state.turn_phase, TurnPhase::Moving,);
    }

    #[test]
    fn missing_action_id_is_rejected_when_reveal_is_due() {
        let state = committed_state();

        let error = plan_reveal(RevealPlannerInput {
            contract_id: CONTRACT_ID,
            local_player: Player::White,
            signing_key: crate::test_support::signing_key_for_player(Player::White),
            authoritative_state: &state,
            pending: None,
            stored_secret: Some([11; 32]),
            new_action_id: None,
        })
        .unwrap_err();

        assert!(error.contains("fresh random action ID"));
    }
}
