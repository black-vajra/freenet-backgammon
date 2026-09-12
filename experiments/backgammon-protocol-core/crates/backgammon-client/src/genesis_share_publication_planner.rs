use backgammon_lobby_core::ChallengeOfferState;
use backgammon_protocol::{Action, ChallengeResolution, ChallengeTerminalEvidence};

use crate::genesis_handshake_store::StoredGenesisHandshake;
use crate::lobby_codec::build_encoded_challenge_state_update;

/// One deterministic, already-signed local genesis-share publication.
///
/// Constructing this value performs no signing, persistence, or network I/O.
/// The signature is recovered from the verified durable handshake.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GenesisSharePublicationPlan {
    pub local_share_evidence: ChallengeTerminalEvidence,
    pub encoded_lobby_state_update: Vec<u8>,
}

/// Plans publication of the durable local genesis share into one exact
/// authoritative accepted challenge.
///
/// `Ok(None)` means that the exact local share is already authoritative.
/// Otherwise, the returned update preserves all authenticated evidence already
/// present and adds the local role-bound share canonically.
///
/// This function performs no transport operation. In particular, callers must
/// not emit its payload to a lobby contract that predates genesis-share
/// evidence support.
pub fn plan_genesis_share_publication(
    authoritative_offer: &ChallengeOfferState,
    handshake: &StoredGenesisHandshake,
) -> Result<Option<GenesisSharePublicationPlan>, String> {
    authoritative_offer.verify().map_err(|error| {
        format!(
            "Authoritative challenge failed verification before \
             genesis-share planning: {error}"
        )
    })?;

    handshake.verify().map_err(|error| {
        format!(
            "Stored genesis handshake failed verification before \
             publication planning: {error}"
        )
    })?;

    let ChallengeResolution::Accepted { proposal } =
        authoritative_offer.resolution().map_err(|error| {
            format!(
                "Could not resolve authoritative challenge before \
                 genesis-share planning: {error}"
            )
        })?
    else {
        return Err("Genesis-share publication requires an authoritative \
             accepted challenge."
            .to_owned());
    };

    if proposal != handshake.proposal
        || authoritative_offer.offer.body.proposal != handshake.proposal
    {
        return Err("Authoritative challenge and stored genesis handshake \
             authenticate different proposals."
            .to_owned());
    }

    let local_share = handshake
        .shares
        .iter()
        .find(|share| share.player_id == handshake.local_player_id)
        .cloned()
        .ok_or_else(|| {
            "Stored genesis handshake does not contain the local \
             participant's signature share."
                .to_owned()
        })?;

    let configuration = &handshake.proposal.configuration;

    let (local_share_evidence, authoritative_local_evidence) =
        if handshake.local_player_id == configuration.white.id {
            (
                ChallengeTerminalEvidence::WhiteGenesisShare(local_share),
                authoritative_offer
                    .terminal_evidence
                    .iter()
                    .find(|evidence| {
                        matches!(evidence, ChallengeTerminalEvidence::WhiteGenesisShare(_))
                    }),
            )
        } else if handshake.local_player_id == configuration.black.id {
            (
                ChallengeTerminalEvidence::BlackGenesisShare(local_share),
                authoritative_offer
                    .terminal_evidence
                    .iter()
                    .find(|evidence| {
                        matches!(evidence, ChallengeTerminalEvidence::BlackGenesisShare(_))
                    }),
            )
        } else {
            /*
             * handshake.verify() already rejects this case. Keep the planner's
             * identity boundary explicit.
             */
            return Err("Stored genesis handshake local identity is not a \
                 configured participant."
                .to_owned());
        };

    if let Some(authoritative) = authoritative_local_evidence {
        if authoritative == &local_share_evidence {
            return Ok(None);
        }

        return Err("Authoritative local-role genesis share differs from the \
             durable local share."
            .to_owned());
    }

    let mut evidence = authoritative_offer.terminal_evidence.clone();
    evidence.push(local_share_evidence.clone());

    let publication_state =
        ChallengeOfferState::new(authoritative_offer.offer.clone(), evidence)
            .map_err(|error| format!("Could not construct genesis-share lobby update: {error}"))?;

    let encoded_lobby_state_update = build_encoded_challenge_state_update(publication_state)
        .map_err(|error| format!("Could not encode genesis-share lobby update: {error}"))?;

    Ok(Some(GenesisSharePublicationPlan {
        local_share_evidence,
        encoded_lobby_state_update,
    }))
}

/// Pure result of reconciling verified authoritative genesis-share evidence
/// into one exact durable local handshake.
///
/// This planner performs no signing, persistence, or network I/O.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GenesisShareIngestionPlan {
    pub updated_handshake: StoredGenesisHandshake,
    pub authenticated_genesis: Option<Action>,
    pub changed: bool,
}

/// Adds verified authoritative signature shares to the corresponding durable
/// local handshake.
///
/// The challenge must remain authoritatively accepted and authenticate the
/// exact stored proposal. The durable local share must already exist; this
/// function never signs or manufactures missing evidence. Duplicate delivery
/// is idempotent, while divergent evidence fails closed without mutating the
/// supplied handshake.
pub fn plan_authoritative_genesis_share_ingestion(
    authoritative_offer: &ChallengeOfferState,
    handshake: &StoredGenesisHandshake,
) -> Result<GenesisShareIngestionPlan, String> {
    authoritative_offer.verify().map_err(|error| {
        format!(
            "Authoritative challenge failed verification before \
             genesis-share ingestion: {error}"
        )
    })?;

    handshake.verify().map_err(|error| {
        format!(
            "Stored genesis handshake failed verification before \
             authoritative share ingestion: {error}"
        )
    })?;

    let ChallengeResolution::Accepted { proposal } =
        authoritative_offer.resolution().map_err(|error| {
            format!(
                "Could not resolve authoritative challenge before \
                 genesis-share ingestion: {error}"
            )
        })?
    else {
        return Err(
            "Genesis-share ingestion requires an authoritative accepted challenge.".to_owned(),
        );
    };

    if proposal != handshake.proposal
        || authoritative_offer.offer.body.proposal != handshake.proposal
    {
        return Err("Authoritative challenge and stored genesis handshake \
             authenticate different proposals."
            .to_owned());
    }

    if !handshake
        .shares
        .iter()
        .any(|share| share.player_id == handshake.local_player_id)
    {
        return Err("Stored genesis handshake does not contain the local \
             participant's signature share."
            .to_owned());
    }

    let mut updated_handshake = handshake.clone();

    for evidence in &authoritative_offer.terminal_evidence {
        let share = match evidence {
            ChallengeTerminalEvidence::WhiteGenesisShare(share)
            | ChallengeTerminalEvidence::BlackGenesisShare(share) => share,

            ChallengeTerminalEvidence::Acceptance(_)
            | ChallengeTerminalEvidence::Decline(_)
            | ChallengeTerminalEvidence::Cancellation(_) => continue,
        };

        updated_handshake
            .add_share(share.clone())
            .map_err(|error| {
                format!(
                    "Authoritative genesis-share evidence could not be \
                     added to the durable handshake: {error}"
                )
            })?;
    }

    updated_handshake.verify()?;
    let authenticated_genesis = updated_handshake.authenticated_genesis()?;

    Ok(GenesisShareIngestionPlan {
        changed: updated_handshake != *handshake,
        updated_handshake,
        authenticated_genesis,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use backgammon_lobby_core::LobbyEntries;
    use backgammon_protocol::{
        accept_challenge, sign_challenge_offer, ChallengeOfferBody, GameConfiguration,
        GenesisProposal, PlayerDescriptor,
    };
    use ed25519_dalek::SigningKey;

    use crate::lobby_codec::decode_verified_lobby_state;

    const CREATED: u64 = 900_000;
    const EXPIRES: u64 = CREATED + 600;

    fn accepted_fixture(seed: u8) -> (ChallengeOfferState, SigningKey, SigningKey) {
        let white_key = SigningKey::from_bytes(&[seed.wrapping_add(1); 32]);
        let black_key = SigningKey::from_bytes(&[seed.wrapping_add(2); 32]);

        let proposal = GenesisProposal::new(
            [seed.wrapping_add(3); 32],
            [seed.wrapping_add(4); 32],
            GameConfiguration {
                white: PlayerDescriptor {
                    id: white_key.verifying_key().to_bytes(),
                    display_name: "Alice".to_owned(),
                },
                black: PlayerDescriptor {
                    id: black_key.verifying_key().to_bytes(),
                    display_name: "Bob".to_owned(),
                },
                match_length: 5,
            },
        );

        let body = ChallengeOfferBody::new(
            [seed; 32],
            white_key.verifying_key().to_bytes(),
            CREATED,
            EXPIRES,
            proposal,
        );

        let offer = sign_challenge_offer(body, &white_key).unwrap();
        let acceptance = accept_challenge(&offer, &black_key, CREATED + 1).unwrap();

        let accepted = ChallengeOfferState::new(
            offer,
            vec![ChallengeTerminalEvidence::Acceptance(acceptance)],
        )
        .unwrap();

        (accepted, white_key, black_key)
    }

    fn local_handshake(
        state: &ChallengeOfferState,
        signing_key: &SigningKey,
    ) -> StoredGenesisHandshake {
        let mut handshake = StoredGenesisHandshake::new(
            state.offer.body.proposal.clone(),
            signing_key.verifying_key().to_bytes(),
        )
        .unwrap();

        assert_eq!(handshake.ensure_local_share(signing_key), Ok(true));

        handshake
    }

    #[test]
    fn white_share_plan_is_exact_verified_lobby_update() {
        let (accepted, white_key, _) = accepted_fixture(11);
        let handshake = local_handshake(&accepted, &white_key);

        let plan = plan_genesis_share_publication(&accepted, &handshake)
            .unwrap()
            .expect("White share is not yet authoritative");

        assert!(matches!(
            &plan.local_share_evidence,
            ChallengeTerminalEvidence::WhiteGenesisShare(share)
                if share.player_id
                    == white_key.verifying_key().to_bytes()
        ));

        let decoded = decode_verified_lobby_state(&plan.encoded_lobby_state_update).unwrap();

        assert_eq!(decoded.lobby, LobbyEntries::default());
        assert_eq!(decoded.challenges.offers.len(), 1);
        assert_eq!(decoded.challenges.offers[0].offer, accepted.offer);
        assert_eq!(decoded.challenges.offers[0].terminal_evidence.len(), 2);
        assert!(decoded.challenges.offers[0]
            .terminal_evidence
            .contains(&plan.local_share_evidence));
    }

    #[test]
    fn peer_share_is_preserved_and_does_not_suppress_local_share() {
        let (accepted, white_key, black_key) = accepted_fixture(21);

        let white_handshake = local_handshake(&accepted, &white_key);
        let white_plan = plan_genesis_share_publication(&accepted, &white_handshake)
            .unwrap()
            .unwrap();

        let state_with_white = decode_verified_lobby_state(&white_plan.encoded_lobby_state_update)
            .unwrap()
            .challenges
            .offers
            .remove(0);

        let black_handshake = local_handshake(&state_with_white, &black_key);
        let black_plan = plan_genesis_share_publication(&state_with_white, &black_handshake)
            .unwrap()
            .expect("White's share must not suppress Black's share");

        assert!(matches!(
            black_plan.local_share_evidence,
            ChallengeTerminalEvidence::BlackGenesisShare(_)
        ));

        let complete = decode_verified_lobby_state(&black_plan.encoded_lobby_state_update)
            .unwrap()
            .challenges
            .offers
            .remove(0);

        assert_eq!(complete.terminal_evidence.len(), 3);
        assert_eq!(complete.evidence_mask(), 0b11001);
        assert_eq!(
            complete.resolution().unwrap(),
            ChallengeResolution::Accepted {
                proposal: accepted.offer.body.proposal,
            }
        );
    }

    #[test]
    fn authoritative_local_share_makes_planning_idempotent() {
        let (accepted, _, black_key) = accepted_fixture(31);
        let handshake = local_handshake(&accepted, &black_key);

        let first = plan_genesis_share_publication(&accepted, &handshake)
            .unwrap()
            .unwrap();

        let authoritative = decode_verified_lobby_state(&first.encoded_lobby_state_update)
            .unwrap()
            .challenges
            .offers
            .remove(0);

        assert_eq!(
            plan_genesis_share_publication(&authoritative, &handshake,),
            Ok(None)
        );
    }

    #[test]
    fn open_mismatched_and_unsigned_handshakes_are_rejected() {
        let (accepted, white_key, _) = accepted_fixture(41);
        let signed_handshake = local_handshake(&accepted, &white_key);

        let open = ChallengeOfferState::new(accepted.offer.clone(), Vec::new()).unwrap();

        assert!(plan_genesis_share_publication(&open, &signed_handshake,)
            .unwrap_err()
            .contains("requires an authoritative accepted challenge"));

        let (different, different_white, _) = accepted_fixture(51);
        let different_handshake = local_handshake(&different, &different_white);

        assert!(
            plan_genesis_share_publication(&accepted, &different_handshake,)
                .unwrap_err()
                .contains("authenticate different proposals")
        );

        let unsigned = StoredGenesisHandshake::new(
            accepted.offer.body.proposal.clone(),
            white_key.verifying_key().to_bytes(),
        )
        .unwrap();

        assert!(plan_genesis_share_publication(&accepted, &unsigned,)
            .unwrap_err()
            .contains("does not contain the local"));
    }

    fn accepted_with_shares(
        accepted: &ChallengeOfferState,
        shares: &[crate::genesis_handshake::GenesisSignatureShare],
    ) -> ChallengeOfferState {
        let white_id = accepted.offer.body.proposal.configuration.white.id;
        let mut evidence = accepted.terminal_evidence.clone();

        for share in shares {
            let item = if share.player_id == white_id {
                ChallengeTerminalEvidence::WhiteGenesisShare(share.clone())
            } else {
                ChallengeTerminalEvidence::BlackGenesisShare(share.clone())
            };

            evidence.push(item);
        }

        ChallengeOfferState::new(accepted.offer.clone(), evidence).unwrap()
    }

    #[test]
    fn authoritative_peer_share_completes_and_repeats_idempotently() {
        let (accepted, white_key, black_key) = accepted_fixture(61);
        let handshake = local_handshake(&accepted, &white_key);

        let black_share =
            crate::genesis_handshake::sign_genesis_proposal(&handshake.proposal, &black_key)
                .unwrap();

        let authoritative = accepted_with_shares(&accepted, &[black_share.clone()]);

        let first = plan_authoritative_genesis_share_ingestion(&authoritative, &handshake).unwrap();

        assert!(first.changed);
        assert_eq!(first.updated_handshake.shares.len(), 2);

        let expected = crate::genesis_handshake::assemble_authenticated_genesis(
            &handshake.proposal,
            &[handshake.shares[0].clone(), black_share],
        )
        .unwrap();

        assert_eq!(first.authenticated_genesis, Some(expected));

        let repeated =
            plan_authoritative_genesis_share_ingestion(&authoritative, &first.updated_handshake)
                .unwrap();

        assert!(!repeated.changed);
        assert_eq!(repeated.updated_handshake, first.updated_handshake,);
        assert_eq!(repeated.authenticated_genesis, first.authenticated_genesis,);
    }

    #[test]
    fn authoritative_share_delivery_order_is_irrelevant() {
        let (accepted, white_key, black_key) = accepted_fixture(71);
        let handshake = local_handshake(&accepted, &white_key);

        let white_share =
            crate::genesis_handshake::sign_genesis_proposal(&handshake.proposal, &white_key)
                .unwrap();

        let black_share =
            crate::genesis_handshake::sign_genesis_proposal(&handshake.proposal, &black_key)
                .unwrap();

        let forward = accepted_with_shares(&accepted, &[white_share.clone(), black_share.clone()]);

        let reverse = accepted_with_shares(&accepted, &[black_share, white_share]);

        let forward_plan =
            plan_authoritative_genesis_share_ingestion(&forward, &handshake).unwrap();

        let reverse_plan =
            plan_authoritative_genesis_share_ingestion(&reverse, &handshake).unwrap();

        assert_eq!(
            forward_plan.updated_handshake,
            reverse_plan.updated_handshake,
        );
        assert_eq!(
            forward_plan.authenticated_genesis,
            reverse_plan.authenticated_genesis,
        );
    }

    #[test]
    fn ingestion_rejects_open_mismatched_and_unsigned_inputs() {
        let (accepted, white_key, _) = accepted_fixture(81);
        let signed = local_handshake(&accepted, &white_key);

        let open = ChallengeOfferState::new(accepted.offer.clone(), Vec::new()).unwrap();

        assert!(plan_authoritative_genesis_share_ingestion(&open, &signed,)
            .unwrap_err()
            .contains("requires an authoritative accepted challenge"));

        let (different, different_white, _) = accepted_fixture(91);
        let different_handshake = local_handshake(&different, &different_white);

        assert!(
            plan_authoritative_genesis_share_ingestion(&accepted, &different_handshake,)
                .unwrap_err()
                .contains("authenticate different proposals")
        );

        let unsigned = StoredGenesisHandshake::new(
            accepted.offer.body.proposal.clone(),
            white_key.verifying_key().to_bytes(),
        )
        .unwrap();

        assert!(
            plan_authoritative_genesis_share_ingestion(&accepted, &unsigned,)
                .unwrap_err()
                .contains("does not contain the local")
        );
    }
}
