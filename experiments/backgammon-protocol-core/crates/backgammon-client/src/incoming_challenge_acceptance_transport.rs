use freenet_stdlib::client_api::{ContractResponse, HostResponse};
use freenet_stdlib::prelude::ContractKey;

/// Direct result of an explicitly armed incoming-challenge contract read.
///
/// Subscription notifications are intentionally excluded. Only a direct
/// `GetResponse` supplies the response proof consumed by the later acceptance
/// finalizer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IncomingChallengeContractRead {
    Retrieved {
        contract_key: ContractKey,
        state: Vec<u8>,
    },
    NotFound,
}

/// Routes only a direct contract-read result for the expected canonical
/// instance ID.
///
/// This function performs no signing or persistence. Complete-key and canonical
/// empty-state verification remain the responsibility of the acceptance
/// finalizer, which also rechecks current authoritative challenge evidence.
pub fn classify_incoming_challenge_contract_response(
    response: &HostResponse,
    expected_contract_id: &str,
) -> Option<IncomingChallengeContractRead> {
    let HostResponse::ContractResponse(response) = response else {
        return None;
    };

    match response {
        ContractResponse::GetResponse { key, state, .. }
            if key.id().encode() == expected_contract_id =>
        {
            Some(IncomingChallengeContractRead::Retrieved {
                contract_key: key.clone(),
                state: state.as_ref().to_vec(),
            })
        }

        ContractResponse::NotFound { instance_id }
            if instance_id.encode() == expected_contract_id =>
        {
            Some(IncomingChallengeContractRead::NotFound)
        }

        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use freenet_stdlib::prelude::{UpdateData, WrappedState};

    use crate::game_contract_publication::calculate_expected_game_contract;

    #[test]
    fn matching_direct_get_returns_full_key_and_state() {
        let expected = calculate_expected_game_contract([41_u8; 32]).unwrap();

        let response: HostResponse =
            HostResponse::ContractResponse(ContractResponse::GetResponse {
                key: expected.full_key.clone(),
                contract: None,
                state: WrappedState::from(expected.empty_state_bytes.clone()),
            });

        assert_eq!(
            classify_incoming_challenge_contract_response(&response, &expected.contract_id,),
            Some(IncomingChallengeContractRead::Retrieved {
                contract_key: expected.full_key,
                state: expected.empty_state_bytes,
            }),
        );
    }

    #[test]
    fn matching_not_found_is_routed_explicitly() {
        let expected = calculate_expected_game_contract([42_u8; 32]).unwrap();

        let response: HostResponse = HostResponse::ContractResponse(ContractResponse::NotFound {
            instance_id: expected.full_key.id().clone(),
        });

        assert_eq!(
            classify_incoming_challenge_contract_response(&response, &expected.contract_id,),
            Some(IncomingChallengeContractRead::NotFound),
        );
    }

    #[test]
    fn unrelated_direct_get_is_ignored() {
        let expected = calculate_expected_game_contract([43_u8; 32]).unwrap();

        let unrelated = calculate_expected_game_contract([44_u8; 32]).unwrap();

        let response: HostResponse =
            HostResponse::ContractResponse(ContractResponse::GetResponse {
                key: unrelated.full_key,
                contract: None,
                state: WrappedState::from(unrelated.empty_state_bytes),
            });

        assert_eq!(
            classify_incoming_challenge_contract_response(&response, &expected.contract_id,),
            None,
        );
    }

    #[test]
    fn matching_subscription_notification_is_not_read_proof() {
        let expected = calculate_expected_game_contract([45_u8; 32]).unwrap();

        let response: HostResponse =
            HostResponse::ContractResponse(ContractResponse::UpdateNotification {
                key: expected.full_key,
                update: UpdateData::State(WrappedState::from(expected.empty_state_bytes).into()),
            });

        assert_eq!(
            classify_incoming_challenge_contract_response(&response, &expected.contract_id,),
            None,
        );
    }
}
