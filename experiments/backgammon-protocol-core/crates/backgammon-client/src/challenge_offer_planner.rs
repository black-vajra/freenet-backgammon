use backgammon_lobby_core::ChallengeOfferState;
use backgammon_protocol::{
    sign_challenge_offer, verify_challenge_offer_at, ActionId, ChallengeId, ChallengeOfferBody,
    GameConfiguration, GameId, GenesisProposal, PlayerDescriptor, PlayerId, SignedChallengeOffer,
    MAX_CHALLENGE_LIFETIME_SECONDS,
};
use ed25519_dalek::SigningKey;

use crate::game_contract_publication::{
    prepare_game_contract_publication, GameContractPublicationInputs,
};
use crate::lobby_codec::build_encoded_challenge_state_update;

/// Complete deterministic material that must be persisted before publication.
///
/// Nothing in this value indicates that either the game contract or challenge
/// offer has reached Freenet. The browser workflow will persist this plan,
/// publish the game contract, confirm its exact key, and only then submit
/// `encoded_lobby_state_update`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutboundChallengePublicationPlan {
    pub signed_offer: SignedChallengeOffer,
    pub contract_publication: GameContractPublicationInputs,
    pub encoded_lobby_state_update: Vec<u8>,
}

pub struct OutboundChallengePlannerInput<'a> {
    pub signing_key: &'a SigningKey,
    pub challenger_display_name: &'a str,
    pub recipient_id: PlayerId,
    pub recipient_display_name: &'a str,
    pub match_length: u16,
    pub challenge_id: ChallengeId,
    pub game_id: GameId,
    pub genesis_action_id: ActionId,
    pub created_at_unix_seconds: u64,
    pub expires_at_unix_seconds: u64,
}

fn verify_independent_identifiers(
    challenge_id: &ChallengeId,
    game_id: &GameId,
    genesis_action_id: &ActionId,
) -> Result<(), String> {
    let identifiers = [
        ("challenge ID", challenge_id),
        ("game ID", game_id),
        ("genesis action ID", genesis_action_id),
    ];

    for (label, identifier) in identifiers {
        if *identifier == [0_u8; 32] {
            return Err(format!("Outbound challenge {label} must not be zero."));
        }
    }

    if challenge_id == game_id || challenge_id == genesis_action_id || game_id == genesis_action_id
    {
        return Err(
            "Challenge, game, and genesis action IDs must use independent random values."
                .to_owned(),
        );
    }

    Ok(())
}

/// Builds one authenticated challenge and all deterministic publication bytes.
///
/// The challenger is White and the selected recipient is Black for the first
/// playable challenge workflow. Color selection can be added later without
/// changing the authenticated wire format.
pub fn plan_outbound_challenge(
    input: OutboundChallengePlannerInput<'_>,
) -> Result<OutboundChallengePublicationPlan, String> {
    verify_independent_identifiers(
        &input.challenge_id,
        &input.game_id,
        &input.genesis_action_id,
    )?;

    let challenger_id = input.signing_key.verifying_key().to_bytes();

    if challenger_id == input.recipient_id {
        return Err("A player cannot create a challenge against the same identity.".to_owned());
    }

    if input.expires_at_unix_seconds <= input.created_at_unix_seconds {
        return Err("Challenge expiry must be later than its creation time.".to_owned());
    }

    let lifetime = input
        .expires_at_unix_seconds
        .checked_sub(input.created_at_unix_seconds)
        .ok_or_else(|| "Challenge lifetime underflowed.".to_owned())?;

    if lifetime > MAX_CHALLENGE_LIFETIME_SECONDS {
        return Err(format!(
            "Challenge lifetime exceeds the maximum of \
             {MAX_CHALLENGE_LIFETIME_SECONDS} seconds."
        ));
    }

    let proposal = GenesisProposal::new(
        input.game_id,
        input.genesis_action_id,
        GameConfiguration {
            white: PlayerDescriptor {
                id: challenger_id,
                display_name: input.challenger_display_name.to_owned(),
            },
            black: PlayerDescriptor {
                id: input.recipient_id,
                display_name: input.recipient_display_name.to_owned(),
            },
            match_length: input.match_length,
        },
    );

    proposal
        .verify()
        .map_err(|error| format!("Challenge genesis proposal is invalid: {error}"))?;

    let body = ChallengeOfferBody::new(
        input.challenge_id,
        challenger_id,
        input.created_at_unix_seconds,
        input.expires_at_unix_seconds,
        proposal,
    );

    let signed_offer = sign_challenge_offer(body, input.signing_key)
        .map_err(|error| format!("Could not sign challenge offer: {error}"))?;

    verify_challenge_offer_at(&signed_offer, input.created_at_unix_seconds)
        .map_err(|error| format!("Newly signed challenge failed live verification: {error}"))?;

    if signed_offer.body.recipient_id()? != input.recipient_id {
        return Err("Signed challenge resolved to an unexpected recipient.".to_owned());
    }

    let contract_publication = prepare_game_contract_publication(input.game_id)?;

    if contract_publication.game_id != signed_offer.body.proposal.game_id {
        return Err("Challenge game ID does not match contract publication parameters.".to_owned());
    }

    let offer_state = ChallengeOfferState::new(signed_offer.clone(), Vec::new())
        .map_err(|error| format!("Could not construct open challenge state: {error}"))?;

    let encoded_lobby_state_update = build_encoded_challenge_state_update(offer_state)
        .map_err(|error| format!("Could not encode challenge lobby update: {error}"))?;

    Ok(OutboundChallengePublicationPlan {
        signed_offer,
        contract_publication,
        encoded_lobby_state_update,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lobby_codec::decode_verified_lobby_state;

    const CREATED: u64 = 200_000;
    const EXPIRES: u64 = CREATED + 600;

    fn challenger() -> SigningKey {
        SigningKey::from_bytes(&[101_u8; 32])
    }

    fn recipient() -> SigningKey {
        SigningKey::from_bytes(&[102_u8; 32])
    }

    fn input<'a>(
        signing_key: &'a SigningKey,
        recipient_id: PlayerId,
        challenge_id: ChallengeId,
        game_id: GameId,
        genesis_action_id: ActionId,
    ) -> OutboundChallengePlannerInput<'a> {
        OutboundChallengePlannerInput {
            signing_key,
            challenger_display_name: "Alice",
            recipient_id,
            recipient_display_name: "Bob",
            match_length: 5,
            challenge_id,
            game_id,
            genesis_action_id,
            created_at_unix_seconds: CREATED,
            expires_at_unix_seconds: EXPIRES,
        }
    }

    #[test]
    fn plan_binds_signed_offer_contract_and_lobby_update() {
        let challenger = challenger();
        let recipient = recipient();
        let game_id = [12_u8; 32];

        let plan = plan_outbound_challenge(input(
            &challenger,
            recipient.verifying_key().to_bytes(),
            [11_u8; 32],
            game_id,
            [13_u8; 32],
        ))
        .unwrap();

        assert_eq!(plan.signed_offer.body.proposal.game_id, game_id);
        assert_eq!(plan.contract_publication.game_id, game_id);
        assert_eq!(
            plan.signed_offer.body.proposal.configuration.white.id,
            challenger.verifying_key().to_bytes(),
        );
        assert_eq!(
            plan.signed_offer.body.proposal.configuration.black.id,
            recipient.verifying_key().to_bytes(),
        );
        assert_eq!(
            plan.signed_offer.body.proposal.configuration.match_length,
            5,
        );

        let decoded = decode_verified_lobby_state(&plan.encoded_lobby_state_update).unwrap();

        assert_eq!(decoded.lobby, Default::default());
        assert_eq!(decoded.challenges.offers.len(), 1);
        assert_eq!(decoded.challenges.offers[0].offer, plan.signed_offer,);
        assert!(decoded.challenges.offers[0].terminal_evidence.is_empty());
    }

    #[test]
    fn identical_inputs_produce_identical_publication_plan() {
        let challenger = challenger();
        let recipient_id = recipient().verifying_key().to_bytes();

        let first = plan_outbound_challenge(input(
            &challenger,
            recipient_id,
            [21_u8; 32],
            [22_u8; 32],
            [23_u8; 32],
        ))
        .unwrap();

        let second = plan_outbound_challenge(input(
            &challenger,
            recipient_id,
            [21_u8; 32],
            [22_u8; 32],
            [23_u8; 32],
        ))
        .unwrap();

        assert_eq!(first, second);
    }

    #[test]
    fn distinct_game_ids_change_offer_and_contract_parameters() {
        let challenger = challenger();
        let recipient_id = recipient().verifying_key().to_bytes();

        let first = plan_outbound_challenge(input(
            &challenger,
            recipient_id,
            [31_u8; 32],
            [32_u8; 32],
            [33_u8; 32],
        ))
        .unwrap();

        let second = plan_outbound_challenge(input(
            &challenger,
            recipient_id,
            [31_u8; 32],
            [42_u8; 32],
            [33_u8; 32],
        ))
        .unwrap();

        assert_ne!(first.signed_offer, second.signed_offer);
        assert_ne!(
            first.contract_publication.parameter_bytes,
            second.contract_publication.parameter_bytes,
        );
        assert_eq!(
            first.contract_publication.state_bytes,
            second.contract_publication.state_bytes,
        );
    }

    #[test]
    fn zero_or_reused_identifiers_are_rejected() {
        let challenger = challenger();
        let recipient_id = recipient().verifying_key().to_bytes();

        assert!(plan_outbound_challenge(input(
            &challenger,
            recipient_id,
            [0_u8; 32],
            [52_u8; 32],
            [53_u8; 32],
        ))
        .is_err());

        assert!(plan_outbound_challenge(input(
            &challenger,
            recipient_id,
            [61_u8; 32],
            [61_u8; 32],
            [63_u8; 32],
        ))
        .is_err());
    }

    #[test]
    fn invalid_participants_and_settings_are_rejected() {
        let challenger = challenger();
        let challenger_id = challenger.verifying_key().to_bytes();

        assert!(plan_outbound_challenge(input(
            &challenger,
            challenger_id,
            [71_u8; 32],
            [72_u8; 32],
            [73_u8; 32],
        ))
        .is_err());

        let recipient_id = recipient().verifying_key().to_bytes();

        let mut invalid_match = input(
            &challenger,
            recipient_id,
            [81_u8; 32],
            [82_u8; 32],
            [83_u8; 32],
        );
        invalid_match.match_length = 0;

        assert!(plan_outbound_challenge(invalid_match).is_err());

        let mut invalid_window = input(
            &challenger,
            recipient_id,
            [91_u8; 32],
            [92_u8; 32],
            [93_u8; 32],
        );
        invalid_window.expires_at_unix_seconds = CREATED;

        assert!(plan_outbound_challenge(invalid_window).is_err());
    }
}
