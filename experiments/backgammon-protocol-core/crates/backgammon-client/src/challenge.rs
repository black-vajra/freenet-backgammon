/*
 * Compatibility re-export.
 *
 * Authenticated challenge evidence is transport-independent protocol data.
 * Its implementation lives in backgammon-protocol so clients and Freenet
 * contracts share the same wire types and verification rules.
 */
pub use backgammon_protocol::{
    accept_challenge, accepted_genesis_proposal, accepted_genesis_proposal_at, cancel_challenge,
    decline_challenge, sign_challenge_offer, verify_challenge_acceptance,
    verify_challenge_acceptance_at, verify_challenge_cancellation,
    verify_challenge_cancellation_at, verify_challenge_decline, verify_challenge_decline_at,
    verify_challenge_offer, verify_challenge_offer_at, ChallengeAcceptance, ChallengeCancellation,
    ChallengeDecline, ChallengeId, ChallengeOfferBody, ChallengeSignature, SignedChallengeOffer,
    CHALLENGE_PROTOCOL_VERSION, MAX_CHALLENGE_LIFETIME_SECONDS,
};
