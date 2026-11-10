use backgammon_core::Player;
use backgammon_protocol::{ActionId, GameActionPayload};
use ed25519_dalek::SigningKey;

use crate::ledger_codec::{build_encoded_signed_action_delta, decode_verified_ledger};
use crate::pending_action::{PendingAction, PendingActionResolution};

pub fn prepare_resignation(
    contract_id: &str,
    state: &[u8],
    player: Player,
    action_id: ActionId,
    signing_key: &SigningKey,
) -> Result<PendingAction, String> {
    let (record, delta) = build_encoded_signed_action_delta(
        state,
        action_id,
        GameActionPayload::Resign { player },
        signing_key,
    )?;
    PendingAction::new(contract_id, &record, delta)
}

#[derive(Debug, PartialEq, Eq)]
pub enum ResignationRecovery {
    Accepted,
    Retry,
    Superseded,
}

pub fn recover_resignation(
    contract_id: &str,
    state: &[u8],
    player: Player,
    signing_key: &SigningKey,
    pending: &PendingAction,
) -> Result<ResignationRecovery, String> {
    let record = pending.verify()?;
    if pending.contract_id != contract_id
        || record.payload != (GameActionPayload::Resign { player })
    {
        return Err("Stored resignation belongs to another contract or player.".to_owned());
    }

    let ledger = decode_verified_ledger(state)?;
    if ledger.typed_actions().iter().any(|action| {
        action.game_id == record.game_id
            && action.sequence == record.sequence
            && action.action_id != record.action_id
    }) {
        return Ok(ResignationRecovery::Superseded);
    }

    match pending.reconcile(state)? {
        PendingActionResolution::Accepted => Ok(ResignationRecovery::Accepted),
        PendingActionResolution::Pending => {
            let expected =
                prepare_resignation(contract_id, state, player, record.action_id, signing_key)?;
            if &expected != pending {
                return Err(
                    "Stored resignation differs from the canonical signed action.".to_owned(),
                );
            }
            Ok(ResignationRecovery::Retry)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use backgammon_contract::{LedgerState, LedgerStateDelta};
    use backgammon_protocol::ReplayStatus;
    use ciborium::{de::from_reader, ser::into_writer};

    use crate::ledger_codec::decode_verified_replay;
    use crate::test_support::{black_signing_key, one_action_state, white_signing_key};

    const CONTRACT: &str = "test-contract";

    fn append_delta(state: &[u8], delta: &[u8]) -> Vec<u8> {
        let mut ledger: LedgerState = from_reader(state).unwrap();
        let delta: LedgerStateDelta = from_reader(delta).unwrap();
        ledger.actions.0.extend(delta.actions.unwrap());
        let mut encoded = Vec::new();
        into_writer(&ledger, &mut encoded).unwrap();
        encoded
    }

    #[test]
    fn off_turn_player_can_resign_and_recover_the_verified_result() {
        let state = one_action_state();
        assert_eq!(
            decode_verified_replay(state).unwrap().state.active_player,
            Player::White
        );
        let pending =
            prepare_resignation(CONTRACT, state, Player::Black, [31; 32], black_signing_key())
                .unwrap();
        let recover = |state| {
            recover_resignation(
                CONTRACT,
                state,
                Player::Black,
                black_signing_key(),
                &pending,
            )
        };
        assert_eq!(recover(state).unwrap(), ResignationRecovery::Retry);

        let accepted = append_delta(state, &pending.delta);
        assert_eq!(recover(&accepted).unwrap(), ResignationRecovery::Accepted);
        assert_eq!(
            decode_verified_replay(&accepted).unwrap().status,
            ReplayStatus::Resigned {
                resigned: Player::Black,
                winner: Player::White
            },
        );
        assert!(
            prepare_resignation(
                CONTRACT,
                &accepted,
                Player::Black,
                [32; 32],
                black_signing_key()
            )
            .is_err()
        );
        assert!(
            prepare_resignation(CONTRACT, state, Player::Black, [32; 32], white_signing_key())
                .is_err()
        );
    }

    #[test]
    fn competing_roll_supersedes_pending_resignation() {
        let state = one_action_state();
        let pending =
            prepare_resignation(CONTRACT, state, Player::Black, [31; 32], black_signing_key())
                .unwrap();
        let (_, roll) = build_encoded_signed_action_delta(
            state,
            [32; 32],
            GameActionPayload::RequestRoll {
                turn: 0,
                player: Player::White,
            },
            white_signing_key(),
        )
        .unwrap();
        let advanced = append_delta(state, &roll);

        assert_eq!(
            recover_resignation(
                CONTRACT,
                &advanced,
                Player::Black,
                black_signing_key(),
                &pending
            )
            .unwrap(),
            ResignationRecovery::Superseded,
        );
    }
}
