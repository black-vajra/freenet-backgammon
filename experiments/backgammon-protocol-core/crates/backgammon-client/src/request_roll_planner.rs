use backgammon_core::{Player, TurnPhase};
use backgammon_protocol::{replay_game, ActionId, GameActionPayload};
use ed25519_dalek::SigningKey;

use crate::ledger_codec::{build_encoded_signed_action_delta, decode_verified_ledger};
use crate::pending_action::{PendingAction, PendingActionResolution};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RequestRollPlan {
    NoAction,
    Accepted,

    Submit {
        pending: PendingAction,
        recovered_pending: bool,
    },
}

pub struct RequestRollPlannerInput<'a> {
    pub contract_id: &'a str,
    pub local_player: Player,
    pub signing_key: &'a SigningKey,
    pub authoritative_state: &'a [u8],
    pub pending: Option<&'a PendingAction>,

    /*
     * True only when the human explicitly pressed Roll.
     * Recovery of an existing durable pending request does not require
     * another button press.
     */
    pub requested: bool,

    pub new_action_id: Option<ActionId>,
}

pub fn plan_request_roll(input: RequestRollPlannerInput<'_>) -> Result<RequestRollPlan, String> {
    let ledger = decode_verified_ledger(input.authoritative_state)?;

    let replay = replay_game(ledger.typed_actions())
        .map_err(|error| format!("Could not replay verified roll-request state: {error:?}"))?;

    /*
     * Durable recovery always takes precedence over creation.
     */
    if let Some(pending) = input.pending {
        if pending.contract_id != input.contract_id {
            return Err("Stored pending roll request belongs to another contract.".to_owned());
        }

        let record = pending.verify()?;

        let GameActionPayload::RequestRoll { turn, player } = &record.payload else {
            return Err("Stored pending action is not a roll request.".to_owned());
        };

        if *player != input.local_player {
            return Err("Stored pending roll request belongs to another player.".to_owned());
        }

        return match pending.reconcile(input.authoritative_state)? {
            PendingActionResolution::Accepted => Ok(RequestRollPlan::Accepted),

            PendingActionResolution::Pending => {
                if *turn != replay.next_turn {
                    return Err(format!(
                        "Stored pending roll request is for turn {turn}, \
                         but authoritative turn is {}.",
                        replay.next_turn
                    ));
                }

                if replay.state.turn_phase != TurnPhase::AwaitingRoll || replay.state.dice.is_some()
                {
                    return Err("Stored pending roll request can no longer extend \
                         the authoritative board state."
                        .to_owned());
                }

                if input.local_player != replay.state.active_player {
                    return Err(format!(
                        "Stored pending roll request belongs to {:?}, \
                         but {:?} is now active.",
                        input.local_player, replay.state.active_player,
                    ));
                }

                if replay.roll_requested_by.is_some() || !replay.dice_round.is_empty() {
                    return Err("Authoritative dice processing already started \
                         without accepting the stored pending request."
                        .to_owned());
                }

                Ok(RequestRollPlan::Submit {
                    pending: pending.clone(),
                    recovered_pending: true,
                })
            }
        };
    }

    /*
     * Recover an already accepted request after refresh/restart.
     */
    let accepted_request = ledger.typed_actions().iter().any(|record| {
        matches!(
            &record.payload,
            GameActionPayload::RequestRoll {
                turn,
                player,
            } if *turn == replay.next_turn
                && *player == input.local_player
        )
    });

    if accepted_request {
        return Ok(RequestRollPlan::Accepted);
    }

    /*
     * Background network processing must never invent a roll.
     */
    if !input.requested {
        return Ok(RequestRollPlan::NoAction);
    }

    if replay.state.turn_phase != TurnPhase::AwaitingRoll || replay.state.dice.is_some() {
        return Err("Roll cannot be requested while the board is not awaiting a roll.".to_owned());
    }

    if input.local_player != replay.state.active_player {
        return Err(format!(
            "Local player {:?} cannot request the authoritative {:?} roll.",
            input.local_player, replay.state.active_player,
        ));
    }

    if replay.roll_requested_by.is_some() || !replay.dice_round.is_empty() {
        return Err("The authoritative dice round has already been requested.".to_owned());
    }

    let action_id = input
        .new_action_id
        .ok_or_else(|| "Roll request requires a fresh random action ID.".to_owned())?;

    let (record, delta) = build_encoded_signed_action_delta(
        input.authoritative_state,
        action_id,
        GameActionPayload::RequestRoll {
            turn: replay.next_turn,
            player: input.local_player,
        },
        input.signing_key,
    )?;

    if record.sequence != replay.next_sequence {
        return Err(format!(
            "Built roll-request sequence {} differs from replay next sequence {}.",
            record.sequence, replay.next_sequence,
        ));
    }

    let pending = PendingAction::new(input.contract_id, &record, delta)?;

    Ok(RequestRollPlan::Submit {
        pending,
        recovered_pending: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::build_encoded_action_delta;

    use backgammon_contract::{LedgerState, LedgerStateDelta};
    use ciborium::{de::from_reader, ser::into_writer};

    const CONTRACT_ID: &str = "test-contract";

    fn one_action_state() -> &'static [u8] {
        crate::test_support::one_action_state()
    }

    fn append_pending(state_bytes: &[u8], pending: &PendingAction) -> Vec<u8> {
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

    fn fresh_plan(player: Player, requested: bool, action_id: Option<ActionId>) -> RequestRollPlan {
        plan_request_roll(RequestRollPlannerInput {
            contract_id: CONTRACT_ID,
            local_player: player,
            signing_key: crate::test_support::signing_key_for_player(player),
            authoritative_state: one_action_state(),
            pending: None,
            requested,
            new_action_id: action_id,
        })
        .unwrap()
    }

    fn new_pending() -> PendingAction {
        match fresh_plan(Player::White, true, Some([21; 32])) {
            RequestRollPlan::Submit {
                pending,
                recovered_pending: false,
            } => pending,

            other => panic!("expected fresh roll-request submission, got {other:?}"),
        }
    }

    #[test]
    fn background_processing_does_not_request_roll() {
        assert_eq!(
            fresh_plan(Player::White, false, Some([21; 32]),),
            RequestRollPlan::NoAction,
        );
    }

    #[test]
    fn active_human_builds_canonical_roll_request() {
        let pending = new_pending();
        let record = pending.verify().unwrap();

        assert_eq!(record.sequence, 1);

        assert_eq!(
            record.payload,
            GameActionPayload::RequestRoll {
                turn: 0,
                player: Player::White,
            },
        );
    }

    #[test]
    fn inactive_player_cannot_request_roll() {
        let error = plan_request_roll(RequestRollPlannerInput {
            contract_id: CONTRACT_ID,
            local_player: Player::Black,
            signing_key: crate::test_support::signing_key_for_player(Player::Black),
            authoritative_state: one_action_state(),
            pending: None,
            requested: true,
            new_action_id: Some([22; 32]),
        })
        .unwrap_err();

        assert!(error.contains("cannot request"));
    }

    #[test]
    fn explicit_request_requires_fresh_action_id() {
        let error = plan_request_roll(RequestRollPlannerInput {
            contract_id: CONTRACT_ID,
            local_player: Player::White,
            signing_key: crate::test_support::signing_key_for_player(Player::White),
            authoritative_state: one_action_state(),
            pending: None,
            requested: true,
            new_action_id: None,
        })
        .unwrap_err();

        assert!(error.contains("fresh random action ID"));
    }

    #[test]
    fn exact_pending_request_is_retried() {
        let pending = new_pending();

        let plan = plan_request_roll(RequestRollPlannerInput {
            contract_id: CONTRACT_ID,
            local_player: Player::White,
            signing_key: crate::test_support::signing_key_for_player(Player::White),
            authoritative_state: one_action_state(),
            pending: Some(&pending),
            requested: false,
            new_action_id: Some([99; 32]),
        })
        .unwrap();

        assert_eq!(
            plan,
            RequestRollPlan::Submit {
                pending,
                recovered_pending: true,
            },
        );
    }

    #[test]
    fn accepted_pending_request_is_reconciled() {
        let pending = new_pending();
        let accepted = append_pending(one_action_state(), &pending);

        let plan = plan_request_roll(RequestRollPlannerInput {
            contract_id: CONTRACT_ID,
            local_player: Player::White,
            signing_key: crate::test_support::signing_key_for_player(Player::White),
            authoritative_state: &accepted,
            pending: Some(&pending),
            requested: false,
            new_action_id: None,
        })
        .unwrap();

        assert_eq!(plan, RequestRollPlan::Accepted);
    }

    #[test]
    fn accepted_request_without_pending_is_recovered() {
        let pending = new_pending();
        let accepted = append_pending(one_action_state(), &pending);

        let plan = plan_request_roll(RequestRollPlannerInput {
            contract_id: CONTRACT_ID,
            local_player: Player::White,
            signing_key: crate::test_support::signing_key_for_player(Player::White),
            authoritative_state: &accepted,
            pending: None,
            requested: false,
            new_action_id: None,
        })
        .unwrap();

        assert_eq!(plan, RequestRollPlan::Accepted);
    }

    #[test]
    fn pending_request_owned_by_other_player_is_rejected() {
        let pending = new_pending();

        let error = plan_request_roll(RequestRollPlannerInput {
            contract_id: CONTRACT_ID,
            local_player: Player::Black,
            signing_key: crate::test_support::signing_key_for_player(Player::Black),
            authoritative_state: one_action_state(),
            pending: Some(&pending),
            requested: false,
            new_action_id: None,
        })
        .unwrap_err();

        assert!(error.contains("another player"));
    }

    #[test]
    fn conflicting_action_at_pending_sequence_fails_closed() {
        let pending = new_pending();

        let (_, competing_delta) = build_encoded_action_delta(
            one_action_state(),
            [88; 32],
            GameActionPayload::Resign {
                player: Player::White,
            },
        )
        .unwrap();

        let mut state: LedgerState = from_reader(one_action_state()).unwrap();

        let delta: LedgerStateDelta = from_reader(competing_delta.as_slice()).unwrap();

        state
            .actions
            .0
            .extend(delta.actions.expect("competing delta must contain actions"));

        let mut encoded = Vec::new();
        into_writer(&state, &mut encoded).unwrap();

        assert!(plan_request_roll(RequestRollPlannerInput {
            contract_id: CONTRACT_ID,
            local_player: Player::White,
            signing_key: crate::test_support::signing_key_for_player(Player::White),
            authoritative_state: &encoded,
            pending: Some(&pending),
            requested: false,
            new_action_id: None,
        })
        .is_err());
    }
}
