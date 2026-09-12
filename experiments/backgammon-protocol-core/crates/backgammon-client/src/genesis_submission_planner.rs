use backgammon_contract::LedgerStateDelta;
use backgammon_protocol::{verify_typed_action_history, Action, GameActionPayload};
use ciborium::{de::from_reader, ser::into_writer};

use crate::game_contract_publication::calculate_expected_game_contract;
use crate::ledger_codec::decode_verified_ledger;
use crate::pending_action::{PendingAction, PendingActionResolution};

/// Pure disposition of one exact authenticated sequence-zero action.
///
/// The caller performs persistence, transport submission, and pending-record
/// cleanup only after interpreting this result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GenesisSubmissionPlan {
    Accepted {
        remove_pending: bool,
    },

    Submit {
        pending: PendingAction,
        recovered_pending: bool,
    },
}

pub struct GenesisSubmissionPlannerInput<'a> {
    pub contract_id: &'a str,
    pub authenticated_genesis: &'a Action,
    pub authoritative_state: &'a [u8],
    pub pending: Option<&'a PendingAction>,
}

/// Plans durable, idempotent submission of an already-authenticated genesis.
///
/// This function never signs, persists, submits, removes, or performs network
/// I/O. A fresh plan canonically encodes the exact supplied Action. Recovery
/// requires the stored delta to contain that same Action byte-for-byte at the
/// typed level, including both genesis signatures.
pub fn plan_authenticated_genesis_submission(
    input: GenesisSubmissionPlannerInput<'_>,
) -> Result<GenesisSubmissionPlan, String> {
    verify_typed_action_history(std::slice::from_ref(input.authenticated_genesis))
        .map_err(|error| format!("Authenticated genesis failed verification: {error}"))?;

    let genesis_record = input
        .authenticated_genesis
        .to_game_action_record()
        .map_err(|error| format!("Authenticated genesis failed typed decoding: {error}"))?;

    if !matches!(&genesis_record.payload, GameActionPayload::CreateGame(_)) {
        return Err("Genesis submission requires a CreateGame action.".to_owned());
    }

    let expected_contract = calculate_expected_game_contract(genesis_record.game_id)?;

    if expected_contract.contract_id != input.contract_id {
        return Err(
            "Authenticated genesis game ID does not derive the active contract ID.".to_owned(),
        );
    }

    let ledger = decode_verified_ledger(input.authoritative_state)?;

    if let Some(pending) = input.pending {
        if pending.contract_id != input.contract_id {
            return Err("Stored pending action belongs to another contract.".to_owned());
        }

        let pending_record = pending.verify()?;

        /*
         * Once genesis is authoritative, the shared pending slot may correctly
         * contain a later player action. Verify genesis without consuming or
         * removing that unrelated durable retry unit.
         */
        if !matches!(&pending_record.payload, GameActionPayload::CreateGame(_)) {
            if ledger.action_count() == 0 {
                return Err(
                    "An empty authoritative ledger cannot coexist with a post-genesis pending action."
                        .to_owned(),
                );
            }

            verify_authoritative_exact_genesis(&ledger, input.authenticated_genesis)?;

            return Ok(GenesisSubmissionPlan::Accepted {
                remove_pending: false,
            });
        }

        verify_exact_pending_genesis(
            input.contract_id,
            pending,
            input.authenticated_genesis,
            &genesis_record,
        )?;

        return match pending.reconcile(input.authoritative_state)? {
            PendingActionResolution::Pending => {
                if ledger.action_count() != 0 {
                    return Err(
                        "Pending genesis can only extend an empty authoritative ledger.".to_owned(),
                    );
                }

                Ok(GenesisSubmissionPlan::Submit {
                    pending: pending.clone(),
                    recovered_pending: true,
                })
            }

            PendingActionResolution::Accepted => {
                verify_authoritative_exact_genesis(&ledger, input.authenticated_genesis)?;

                Ok(GenesisSubmissionPlan::Accepted {
                    remove_pending: true,
                })
            }
        };
    }

    if ledger.action_count() == 0 {
        let pending = build_pending_genesis(
            input.contract_id,
            input.authenticated_genesis,
            &genesis_record,
        )?;

        return Ok(GenesisSubmissionPlan::Submit {
            pending,
            recovered_pending: false,
        });
    }

    verify_authoritative_exact_genesis(&ledger, input.authenticated_genesis)?;

    Ok(GenesisSubmissionPlan::Accepted {
        remove_pending: false,
    })
}

fn build_pending_genesis(
    contract_id: &str,
    authenticated_genesis: &Action,
    genesis_record: &backgammon_protocol::GameActionRecord,
) -> Result<PendingAction, String> {
    let delta = LedgerStateDelta {
        actions: Some(vec![authenticated_genesis.clone()]),
    };

    let mut encoded = Vec::new();

    into_writer(&delta, &mut encoded)
        .map_err(|error| format!("Could not encode authenticated genesis delta: {error}"))?;

    let decoded: LedgerStateDelta = from_reader(encoded.as_slice())
        .map_err(|error| format!("Encoded authenticated genesis delta did not decode: {error}"))?;

    if decoded != delta {
        return Err("Authenticated genesis delta failed exact typed round-trip.".to_owned());
    }

    PendingAction::new(contract_id, genesis_record, encoded)
}

fn verify_exact_pending_genesis(
    contract_id: &str,
    pending: &PendingAction,
    authenticated_genesis: &Action,
    genesis_record: &backgammon_protocol::GameActionRecord,
) -> Result<(), String> {
    if pending.contract_id != contract_id {
        return Err("Stored pending genesis belongs to another contract.".to_owned());
    }

    let pending_record = pending.verify()?;

    if pending_record != *genesis_record {
        return Err(
            "Stored pending genesis does not match the authenticated genesis record.".to_owned(),
        );
    }

    let decoded: LedgerStateDelta = from_reader(pending.delta.as_slice())
        .map_err(|error| format!("Stored pending genesis delta did not decode: {error}"))?;

    let actions = decoded
        .actions
        .as_ref()
        .ok_or_else(|| "Stored pending genesis delta has no action component.".to_owned())?;

    if actions.as_slice() != std::slice::from_ref(authenticated_genesis) {
        return Err(
            "Stored pending genesis delta does not contain the exact authenticated action."
                .to_owned(),
        );
    }

    let mut canonical = Vec::new();

    into_writer(&decoded, &mut canonical)
        .map_err(|error| format!("Could not re-encode stored pending genesis delta: {error}"))?;

    if canonical != pending.delta {
        return Err("Stored pending genesis delta is not canonically encoded.".to_owned());
    }

    Ok(())
}

fn verify_authoritative_exact_genesis(
    ledger: &crate::ledger_codec::VerifiedLedger,
    authenticated_genesis: &Action,
) -> Result<(), String> {
    let authoritative_genesis = ledger
        .storage_actions()
        .iter()
        .find(|action| action.sequence == 0)
        .ok_or_else(|| {
            "Nonempty authoritative ledger does not contain sequence-zero genesis.".to_owned()
        })?;

    if authoritative_genesis != authenticated_genesis {
        return Err(
            "Authoritative ledger contains a different authenticated genesis action.".to_owned(),
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use backgammon_contract::LedgerState;
    use backgammon_protocol::{
        assemble_authenticated_genesis, sign_genesis_proposal, GameConfiguration, GenesisProposal,
        PlayerDescriptor,
    };
    use ed25519_dalek::SigningKey;

    use crate::game_contract_publication::prepare_game_contract_publication;

    fn fixture(game_id: [u8; 32], action_id: [u8; 32]) -> (GenesisProposal, Action, String) {
        let white = SigningKey::from_bytes(&[41; 32]);
        let black = SigningKey::from_bytes(&[42; 32]);

        let proposal = GenesisProposal::new(
            game_id,
            action_id,
            GameConfiguration {
                white: PlayerDescriptor {
                    id: white.verifying_key().to_bytes(),
                    display_name: "White".to_owned(),
                },
                black: PlayerDescriptor {
                    id: black.verifying_key().to_bytes(),
                    display_name: "Black".to_owned(),
                },
                match_length: 1,
            },
        );

        let white_share = sign_genesis_proposal(&proposal, &white).unwrap();
        let black_share = sign_genesis_proposal(&proposal, &black).unwrap();

        let action =
            assemble_authenticated_genesis(&proposal, &[white_share, black_share]).unwrap();

        let contract_id = calculate_expected_game_contract(game_id)
            .unwrap()
            .contract_id;

        (proposal, action, contract_id)
    }

    fn empty_state(game_id: [u8; 32]) -> Vec<u8> {
        prepare_game_contract_publication(game_id)
            .unwrap()
            .state_bytes
    }

    fn accepted_state(game_id: [u8; 32], action: &Action) -> Vec<u8> {
        let mut state: LedgerState = from_reader(empty_state(game_id).as_slice()).unwrap();
        state.actions.0.push(action.clone());

        let mut encoded = Vec::new();
        into_writer(&state, &mut encoded).unwrap();

        decode_verified_ledger(&encoded).unwrap();
        encoded
    }

    fn fresh_pending(game_id: [u8; 32], action: &Action, contract_id: &str) -> PendingAction {
        let state = empty_state(game_id);

        match plan_authenticated_genesis_submission(GenesisSubmissionPlannerInput {
            contract_id,
            authenticated_genesis: action,
            authoritative_state: &state,
            pending: None,
        })
        .unwrap()
        {
            GenesisSubmissionPlan::Submit {
                pending,
                recovered_pending: false,
            } => pending,

            other => panic!("expected fresh genesis submission, got {other:?}"),
        }
    }

    #[test]
    fn empty_ledger_builds_exact_authenticated_genesis_delta() {
        let game_id = [7; 32];
        let (_, action, contract_id) = fixture(game_id, [8; 32]);
        let pending = fresh_pending(game_id, &action, &contract_id);
        let decoded: LedgerStateDelta = from_reader(pending.delta.as_slice()).unwrap();

        assert_eq!(decoded.actions, Some(vec![action]));
        assert_eq!(pending.sequence, 0);
    }

    #[test]
    fn exact_pending_genesis_is_recovered_without_regeneration() {
        let game_id = [9; 32];
        let (_, action, contract_id) = fixture(game_id, [10; 32]);
        let pending = fresh_pending(game_id, &action, &contract_id);
        let state = empty_state(game_id);

        assert_eq!(
            plan_authenticated_genesis_submission(GenesisSubmissionPlannerInput {
                contract_id: &contract_id,
                authenticated_genesis: &action,
                authoritative_state: &state,
                pending: Some(&pending),
            }),
            Ok(GenesisSubmissionPlan::Submit {
                pending,
                recovered_pending: true,
            }),
        );
    }

    #[test]
    fn authoritative_exact_genesis_is_accepted_with_or_without_pending() {
        let game_id = [11; 32];
        let (_, action, contract_id) = fixture(game_id, [12; 32]);
        let pending = fresh_pending(game_id, &action, &contract_id);
        let state = accepted_state(game_id, &action);

        assert_eq!(
            plan_authenticated_genesis_submission(GenesisSubmissionPlannerInput {
                contract_id: &contract_id,
                authenticated_genesis: &action,
                authoritative_state: &state,
                pending: Some(&pending),
            }),
            Ok(GenesisSubmissionPlan::Accepted {
                remove_pending: true,
            }),
        );

        assert_eq!(
            plan_authenticated_genesis_submission(GenesisSubmissionPlannerInput {
                contract_id: &contract_id,
                authenticated_genesis: &action,
                authoritative_state: &state,
                pending: None,
            }),
            Ok(GenesisSubmissionPlan::Accepted {
                remove_pending: false,
            }),
        );
    }

    #[test]
    fn wrong_contract_and_different_pending_genesis_fail_closed() {
        let game_id = [13; 32];
        let (_, action, contract_id) = fixture(game_id, [14; 32]);
        let (_, different, _) = fixture(game_id, [15; 32]);
        let different_pending = fresh_pending(game_id, &different, &contract_id);
        let state = empty_state(game_id);

        assert!(
            plan_authenticated_genesis_submission(GenesisSubmissionPlannerInput {
                contract_id: "wrong-contract",
                authenticated_genesis: &action,
                authoritative_state: &state,
                pending: None,
            })
            .is_err()
        );

        assert!(
            plan_authenticated_genesis_submission(GenesisSubmissionPlannerInput {
                contract_id: &contract_id,
                authenticated_genesis: &action,
                authoritative_state: &state,
                pending: Some(&different_pending),
            })
            .is_err()
        );
    }

    #[test]
    fn conflicting_authoritative_genesis_fails_closed() {
        let game_id = [16; 32];
        let (_, expected, contract_id) = fixture(game_id, [17; 32]);
        let (_, competing, _) = fixture(game_id, [18; 32]);
        let state = accepted_state(game_id, &competing);

        assert!(
            plan_authenticated_genesis_submission(GenesisSubmissionPlannerInput {
                contract_id: &contract_id,
                authenticated_genesis: &expected,
                authoritative_state: &state,
                pending: None,
            })
            .unwrap_err()
            .contains("different authenticated genesis")
        );
    }

    #[test]
    fn authoritative_genesis_preserves_later_pending_action() {
        let game_id = [21; 32];
        let (_, action, contract_id) = fixture(game_id, [22; 32]);
        let state = accepted_state(game_id, &action);

        let (record, delta) = crate::test_support::build_encoded_action_delta(
            &state,
            [23; 32],
            GameActionPayload::RequestRoll {
                turn: 0,
                player: backgammon_core::Player::White,
            },
        )
        .unwrap();

        let later_pending = PendingAction::new(&contract_id, &record, delta).unwrap();

        assert_eq!(
            plan_authenticated_genesis_submission(GenesisSubmissionPlannerInput {
                contract_id: &contract_id,
                authenticated_genesis: &action,
                authoritative_state: &state,
                pending: Some(&later_pending),
            }),
            Ok(GenesisSubmissionPlan::Accepted {
                remove_pending: false,
            }),
        );

        let empty = empty_state(game_id);

        assert!(
            plan_authenticated_genesis_submission(GenesisSubmissionPlannerInput {
                contract_id: &contract_id,
                authenticated_genesis: &action,
                authoritative_state: &empty,
                pending: Some(&later_pending),
            })
            .unwrap_err()
            .contains("cannot coexist")
        );
    }

    #[test]
    fn unsigned_genesis_is_rejected() {
        let game_id = [19; 32];
        let (proposal, _, contract_id) = fixture(game_id, [20; 32]);
        let unsigned = Action::from_game_action_record(&proposal.build_record().unwrap()).unwrap();
        let state = empty_state(game_id);

        assert!(
            plan_authenticated_genesis_submission(GenesisSubmissionPlannerInput {
                contract_id: &contract_id,
                authenticated_genesis: &unsigned,
                authoritative_state: &state,
                pending: None,
            })
            .is_err()
        );
    }
}
