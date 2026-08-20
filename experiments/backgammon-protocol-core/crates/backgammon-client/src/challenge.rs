use backgammon_protocol::{PlayerId, ED25519_SIGNATURE_BYTES};
use ciborium::ser::into_writer;
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};

use crate::genesis_handshake::GenesisProposal;

/*
 * Lobby/challenge messages are application-layer negotiation messages.
 * Their version is deliberately independent of the replicated game-action
 * protocol version. Changing lobby transport or presence rules must not
 * implicitly change protocol-v4 game history.
 */
pub const CHALLENGE_PROTOCOL_VERSION: u16 = 1;

/*
 * This is an abuse/staleness ceiling, not the normal UI challenge timeout.
 * The future lobby policy may choose a much shorter lifetime. Keeping the
 * protocol ceiling broad avoids coupling this transport-independent core to
 * assumptions about Freenet delivery latency before real transport testing.
 */
pub const MAX_CHALLENGE_LIFETIME_SECONDS: u64 = 24 * 60 * 60;

const CHALLENGE_OFFER_SIGNATURE_DOMAIN_V1: &[u8] = b"freenet-backgammon/challenge-offer/v1";

const CHALLENGE_ACCEPTANCE_SIGNATURE_DOMAIN_V1: &[u8] =
    b"freenet-backgammon/challenge-acceptance/v1";

pub type ChallengeId = [u8; 32];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChallengeSignature(pub Vec<u8>);

impl ChallengeSignature {
    fn from_bytes(bytes: [u8; ED25519_SIGNATURE_BYTES]) -> Self {
        Self(bytes.to_vec())
    }

    pub fn verify_structure(&self) -> Result<(), String> {
        if self.0.len() != ED25519_SIGNATURE_BYTES {
            return Err(format!(
                "Invalid challenge signature length: expected {ED25519_SIGNATURE_BYTES} bytes, got {}.",
                self.0.len()
            ));
        }

        Ok(())
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

/// Exact immutable challenge contents signed by the challenger.
///
/// `proposal` is already the canonical bridge into authenticated game
/// creation. It contains the future game ID, genesis action ID, both player
/// descriptors, colors, match length, and game protocol version.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChallengeOfferBody {
    pub protocol_version: u16,
    pub challenge_id: ChallengeId,
    pub challenger_id: PlayerId,
    pub created_at_unix_seconds: u64,
    pub expires_at_unix_seconds: u64,
    pub proposal: GenesisProposal,
}

impl ChallengeOfferBody {
    pub fn new(
        challenge_id: ChallengeId,
        challenger_id: PlayerId,
        created_at_unix_seconds: u64,
        expires_at_unix_seconds: u64,
        proposal: GenesisProposal,
    ) -> Self {
        Self {
            protocol_version: CHALLENGE_PROTOCOL_VERSION,
            challenge_id,
            challenger_id,
            created_at_unix_seconds,
            expires_at_unix_seconds,
            proposal,
        }
    }

    pub fn verify(&self) -> Result<(), String> {
        if self.protocol_version != CHALLENGE_PROTOCOL_VERSION {
            return Err(format!(
                "Challenge protocol version mismatch: expected {}, got {}.",
                CHALLENGE_PROTOCOL_VERSION, self.protocol_version
            ));
        }

        self.proposal.verify()?;

        let configuration = &self.proposal.configuration;

        if self.challenger_id != configuration.white.id
            && self.challenger_id != configuration.black.id
        {
            return Err("Challenge signer is not a proposed game participant.".to_owned());
        }

        if self.expires_at_unix_seconds <= self.created_at_unix_seconds {
            return Err("Challenge expiration must be later than its creation time.".to_owned());
        }

        let lifetime = self
            .expires_at_unix_seconds
            .checked_sub(self.created_at_unix_seconds)
            .ok_or_else(|| "Challenge lifetime underflowed.".to_owned())?;

        if lifetime > MAX_CHALLENGE_LIFETIME_SECONDS {
            return Err(format!(
                "Challenge lifetime exceeds the maximum of {MAX_CHALLENGE_LIFETIME_SECONDS} seconds."
            ));
        }

        Ok(())
    }

    pub fn recipient_id(&self) -> Result<PlayerId, String> {
        self.verify()?;

        let configuration = &self.proposal.configuration;

        if self.challenger_id == configuration.white.id {
            Ok(configuration.black.id)
        } else if self.challenger_id == configuration.black.id {
            Ok(configuration.white.id)
        } else {
            Err("Challenge signer is not a proposed game participant.".to_owned())
        }
    }

    pub fn verify_not_expired_at(&self, now_unix_seconds: u64) -> Result<(), String> {
        self.verify()?;

        if now_unix_seconds >= self.expires_at_unix_seconds {
            return Err("Challenge has expired.".to_owned());
        }

        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedChallengeOffer {
    pub body: ChallengeOfferBody,
    pub signature: ChallengeSignature,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChallengeAcceptance {
    pub protocol_version: u16,
    pub challenge_id: ChallengeId,
    pub player_id: PlayerId,
    pub signature: ChallengeSignature,
}

#[derive(Serialize)]
struct ChallengeAcceptanceSigningBody<'a> {
    protocol_version: u16,
    challenge_id: ChallengeId,
    player_id: PlayerId,
    offer: &'a ChallengeOfferBody,
}

fn encode_domain_separated_message<T: Serialize>(
    domain: &[u8],
    value: &T,
) -> Result<Vec<u8>, String> {
    let mut encoded = Vec::new();

    into_writer(value, &mut encoded)
        .map_err(|error| format!("Could not encode challenge signing body: {error}"))?;

    let mut message = Vec::with_capacity(domain.len() + 1 + encoded.len());
    message.extend_from_slice(domain);
    message.push(0);
    message.extend_from_slice(&encoded);

    Ok(message)
}

fn offer_signing_message(body: &ChallengeOfferBody) -> Result<Vec<u8>, String> {
    encode_domain_separated_message(CHALLENGE_OFFER_SIGNATURE_DOMAIN_V1, body)
}

fn acceptance_signing_message(
    offer: &ChallengeOfferBody,
    acceptance_protocol_version: u16,
    challenge_id: ChallengeId,
    player_id: PlayerId,
) -> Result<Vec<u8>, String> {
    let body = ChallengeAcceptanceSigningBody {
        protocol_version: acceptance_protocol_version,
        challenge_id,
        player_id,
        offer,
    };

    encode_domain_separated_message(CHALLENGE_ACCEPTANCE_SIGNATURE_DOMAIN_V1, &body)
}

fn verify_ed25519_signature(
    player_id: &PlayerId,
    signature: &ChallengeSignature,
    message: &[u8],
    context: &str,
) -> Result<(), String> {
    signature.verify_structure()?;

    let signature_bytes: [u8; ED25519_SIGNATURE_BYTES] = signature
        .as_bytes()
        .try_into()
        .map_err(|_| "Challenge signature has an invalid byte length.".to_owned())?;

    let verifying_key = VerifyingKey::from_bytes(player_id)
        .map_err(|error| format!("Invalid Ed25519 challenge identity: {error}"))?;

    let signature = Signature::from_bytes(&signature_bytes);

    verifying_key
        .verify_strict(message, &signature)
        .map_err(|error| format!("Invalid {context} signature: {error}"))
}

/// Signs an exact challenge offer.
///
/// The signing key must be the `challenger_id` embedded in the offer.
pub fn sign_challenge_offer(
    body: ChallengeOfferBody,
    signing_key: &SigningKey,
) -> Result<SignedChallengeOffer, String> {
    body.verify()?;

    let player_id = signing_key.verifying_key().to_bytes();

    if player_id != body.challenger_id {
        return Err(
            "Challenge offer cannot be signed by an identity other than its challenger.".to_owned(),
        );
    }

    let message = offer_signing_message(&body)?;
    let signature = ChallengeSignature::from_bytes(signing_key.sign(&message).to_bytes());

    let signed = SignedChallengeOffer { body, signature };

    /*
     * Verify our own output through the same verifier used for hostile input.
     */
    verify_challenge_offer(&signed)?;

    Ok(signed)
}

/// Cryptographically verifies the immutable offer, without applying wall-clock
/// expiry policy.
pub fn verify_challenge_offer(offer: &SignedChallengeOffer) -> Result<(), String> {
    offer.body.verify()?;

    let message = offer_signing_message(&offer.body)?;

    verify_ed25519_signature(
        &offer.body.challenger_id,
        &offer.signature,
        &message,
        "challenge-offer",
    )
}

/// Verifies the signed offer and rejects it once its signed expiration time has
/// been reached.
pub fn verify_challenge_offer_at(
    offer: &SignedChallengeOffer,
    now_unix_seconds: u64,
) -> Result<(), String> {
    verify_challenge_offer(offer)?;
    offer.body.verify_not_expired_at(now_unix_seconds)
}

/// Produces the recipient's signed acceptance of this exact offer.
///
/// Expiry is checked before signing. The caller supplies time explicitly so
/// tests and non-browser transports remain deterministic.
pub fn accept_challenge(
    offer: &SignedChallengeOffer,
    signing_key: &SigningKey,
    now_unix_seconds: u64,
) -> Result<ChallengeAcceptance, String> {
    verify_challenge_offer_at(offer, now_unix_seconds)?;

    let expected_recipient = offer.body.recipient_id()?;
    let player_id = signing_key.verifying_key().to_bytes();

    if player_id != expected_recipient {
        return Err("Only the challenged recipient may accept this challenge.".to_owned());
    }

    let protocol_version = CHALLENGE_PROTOCOL_VERSION;
    let challenge_id = offer.body.challenge_id;

    let message =
        acceptance_signing_message(&offer.body, protocol_version, challenge_id, player_id)?;

    let acceptance = ChallengeAcceptance {
        protocol_version,
        challenge_id,
        player_id,
        signature: ChallengeSignature::from_bytes(signing_key.sign(&message).to_bytes()),
    };

    verify_challenge_acceptance_at(offer, &acceptance, now_unix_seconds)?;

    Ok(acceptance)
}

/// Cryptographically verifies that the challenged participant accepted the
/// exact signed offer.
///
/// This does not establish when the signature was created and therefore does
/// not apply wall-clock expiry policy.
/// A successfully processed acceptance transitions immediately into the
/// genesis-handshake layer. Challenge expiry is therefore lobby liveness
/// policy; it does not later invalidate an already-authenticated game history.
pub fn verify_challenge_acceptance(
    offer: &SignedChallengeOffer,
    acceptance: &ChallengeAcceptance,
) -> Result<(), String> {
    /*
     * Cryptographic validity is independent of the wall clock. Once the
     * recipient has signed this exact authentic offer, later challenge expiry
     * must not invalidate persisted acceptance or an in-progress genesis
     * handshake.
     */
    verify_challenge_offer(offer)?;

    if acceptance.protocol_version != CHALLENGE_PROTOCOL_VERSION {
        return Err(format!(
            "Challenge acceptance protocol version mismatch: expected {}, got {}.",
            CHALLENGE_PROTOCOL_VERSION, acceptance.protocol_version
        ));
    }

    if acceptance.challenge_id != offer.body.challenge_id {
        return Err("Challenge acceptance refers to a different challenge ID.".to_owned());
    }

    let expected_recipient = offer.body.recipient_id()?;

    if acceptance.player_id != expected_recipient {
        return Err(
            "Challenge acceptance was not produced by the challenged recipient.".to_owned(),
        );
    }

    let message = acceptance_signing_message(
        &offer.body,
        acceptance.protocol_version,
        acceptance.challenge_id,
        acceptance.player_id,
    )?;

    verify_ed25519_signature(
        &acceptance.player_id,
        &acceptance.signature,
        &message,
        "challenge-acceptance",
    )
}

/// Applies the live challenge-expiry policy in addition to cryptographic
/// acceptance verification.
///
/// Use this while accepting or processing a still-open lobby challenge. Once
/// an acceptance has already been durably established, callers should use
/// `verify_challenge_acceptance()` instead so ordinary passage of time cannot
/// roll back the negotiation.
pub fn verify_challenge_acceptance_at(
    offer: &SignedChallengeOffer,
    acceptance: &ChallengeAcceptance,
    now_unix_seconds: u64,
) -> Result<(), String> {
    verify_challenge_offer_at(offer, now_unix_seconds)?;
    verify_challenge_acceptance(offer, acceptance)
}

/// Verifies an authenticated accepted challenge and returns the exact proposal
/// that both peers authenticated.
///
/// This is the explicit bridge from lobby/challenge negotiation into the
/// existing durable two-party genesis handshake.
pub fn accepted_genesis_proposal(
    offer: &SignedChallengeOffer,
    acceptance: &ChallengeAcceptance,
) -> Result<GenesisProposal, String> {
    verify_challenge_acceptance(offer, acceptance)?;
    Ok(offer.body.proposal.clone())
}

/// Live-processing variant used while an offer is still in its challenge
/// window. Persisted accepted negotiations should use
/// `accepted_genesis_proposal()` instead.
pub fn accepted_genesis_proposal_at(
    offer: &SignedChallengeOffer,
    acceptance: &ChallengeAcceptance,
    now_unix_seconds: u64,
) -> Result<GenesisProposal, String> {
    verify_challenge_acceptance_at(offer, acceptance, now_unix_seconds)?;
    Ok(offer.body.proposal.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use backgammon_protocol::{GameConfiguration, PlayerDescriptor};

    use crate::genesis_handshake::{assemble_authenticated_genesis, sign_genesis_proposal};

    const CREATED: u64 = 1_000;
    const EXPIRES: u64 = 1_600;

    fn fixture() -> (ChallengeOfferBody, SigningKey, SigningKey) {
        let white_key = SigningKey::from_bytes(&[61; 32]);
        let black_key = SigningKey::from_bytes(&[62; 32]);

        let proposal = GenesisProposal::new(
            [21; 32],
            [22; 32],
            GameConfiguration {
                white: PlayerDescriptor {
                    id: white_key.verifying_key().to_bytes(),
                    display_name: "Alice".to_owned(),
                },
                black: PlayerDescriptor {
                    id: black_key.verifying_key().to_bytes(),
                    display_name: "Bob".to_owned(),
                },
                match_length: 3,
            },
        );

        let offer = ChallengeOfferBody::new(
            [23; 32],
            white_key.verifying_key().to_bytes(),
            CREATED,
            EXPIRES,
            proposal,
        );

        (offer, white_key, black_key)
    }

    #[test]
    fn challenger_can_sign_canonical_offer() {
        let (body, white_key, _) = fixture();

        let offer = sign_challenge_offer(body.clone(), &white_key).unwrap();

        assert_eq!(offer.body, body);
        assert_eq!(verify_challenge_offer(&offer), Ok(()));
        assert_eq!(verify_challenge_offer_at(&offer, CREATED), Ok(()));
    }

    #[test]
    fn nonparticipant_cannot_be_declared_as_challenger() {
        let (mut body, _, _) = fixture();
        let outsider = SigningKey::from_bytes(&[63; 32]);

        body.challenger_id = outsider.verifying_key().to_bytes();

        assert!(body.verify().is_err());
    }

    #[test]
    fn wrong_identity_cannot_sign_challenge_offer() {
        let (body, _, black_key) = fixture();

        assert!(sign_challenge_offer(body, &black_key).is_err());
    }

    #[test]
    fn signed_offer_does_not_survive_mutation() {
        let (body, white_key, _) = fixture();

        let mut offer = sign_challenge_offer(body, &white_key).unwrap();

        offer.body.proposal.configuration.match_length = 5;

        assert!(verify_challenge_offer(&offer).is_err());
    }

    #[test]
    fn expired_offer_is_rejected() {
        let (body, white_key, _) = fixture();

        let offer = sign_challenge_offer(body, &white_key).unwrap();

        assert!(verify_challenge_offer_at(&offer, EXPIRES).is_err());
    }

    #[test]
    fn invalid_or_excessive_lifetime_is_rejected() {
        let (mut body, _, _) = fixture();

        body.expires_at_unix_seconds = body.created_at_unix_seconds;
        assert!(body.verify().is_err());

        body.expires_at_unix_seconds =
            body.created_at_unix_seconds + MAX_CHALLENGE_LIFETIME_SECONDS + 1;

        assert!(body.verify().is_err());
    }

    #[test]
    fn proposed_opponent_is_the_only_valid_recipient() {
        let (body, white_key, black_key) = fixture();

        assert_eq!(
            body.recipient_id().unwrap(),
            black_key.verifying_key().to_bytes()
        );

        let offer = sign_challenge_offer(body, &white_key).unwrap();

        assert!(accept_challenge(&offer, &white_key, CREATED + 1).is_err());

        let outsider = SigningKey::from_bytes(&[64; 32]);

        assert!(accept_challenge(&offer, &outsider, CREATED + 1).is_err());
    }

    #[test]
    fn recipient_can_accept_exact_live_offer() {
        let (body, white_key, black_key) = fixture();

        let offer = sign_challenge_offer(body, &white_key).unwrap();

        let acceptance = accept_challenge(&offer, &black_key, CREATED + 1).unwrap();

        assert_eq!(acceptance.player_id, black_key.verifying_key().to_bytes());

        assert_eq!(
            verify_challenge_acceptance_at(&offer, &acceptance, CREATED + 1),
            Ok(())
        );
    }

    #[test]
    fn acceptance_does_not_survive_offer_substitution() {
        let (body, white_key, black_key) = fixture();

        let first = sign_challenge_offer(body.clone(), &white_key).unwrap();
        let acceptance = accept_challenge(&first, &black_key, CREATED + 1).unwrap();

        let mut different_body = body;
        different_body.challenge_id = [24; 32];

        let different = sign_challenge_offer(different_body, &white_key).unwrap();

        assert!(verify_challenge_acceptance_at(&different, &acceptance, CREATED + 1).is_err());
    }

    #[test]
    fn malformed_acceptance_signature_is_rejected() {
        let (body, white_key, black_key) = fixture();

        let offer = sign_challenge_offer(body, &white_key).unwrap();
        let mut acceptance = accept_challenge(&offer, &black_key, CREATED + 1).unwrap();

        acceptance.signature.0.pop();

        assert!(verify_challenge_acceptance_at(&offer, &acceptance, CREATED + 1).is_err());
    }

    #[test]
    fn accepted_challenge_remains_cryptographically_valid_after_offer_expiry() {
        let (body, white_key, black_key) = fixture();
        let expected = body.proposal.clone();

        let offer = sign_challenge_offer(body, &white_key).unwrap();
        let acceptance = accept_challenge(&offer, &black_key, CREATED + 1).unwrap();

        /*
         * The live lobby window has expired...
         */
        assert!(verify_challenge_acceptance_at(&offer, &acceptance, EXPIRES).is_err());

        /*
         * ...but the already-established cryptographic acceptance remains
         * valid and can still recover the exact genesis negotiation.
         */
        assert_eq!(verify_challenge_acceptance(&offer, &acceptance), Ok(()));

        assert_eq!(
            accepted_genesis_proposal(&offer, &acceptance).unwrap(),
            expected
        );
    }

    #[test]
    fn accepted_challenge_returns_exact_genesis_proposal() {
        let (body, white_key, black_key) = fixture();
        let expected = body.proposal.clone();

        let offer = sign_challenge_offer(body, &white_key).unwrap();
        let acceptance = accept_challenge(&offer, &black_key, CREATED + 1).unwrap();

        let proposal = accepted_genesis_proposal_at(&offer, &acceptance, CREATED + 1).unwrap();

        assert_eq!(proposal, expected);
    }

    #[test]
    fn accepted_challenge_feeds_existing_genesis_handshake() {
        let (body, white_key, black_key) = fixture();

        let offer = sign_challenge_offer(body, &white_key).unwrap();
        let acceptance = accept_challenge(&offer, &black_key, CREATED + 1).unwrap();

        let proposal = accepted_genesis_proposal_at(&offer, &acceptance, CREATED + 1).unwrap();

        let white_share = sign_genesis_proposal(&proposal, &white_key).unwrap();
        let black_share = sign_genesis_proposal(&proposal, &black_key).unwrap();

        assert!(assemble_authenticated_genesis(&proposal, &[white_share, black_share]).is_ok());
    }
}
