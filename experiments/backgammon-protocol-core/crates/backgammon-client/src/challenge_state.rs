/*
 * Compatibility re-export.
 *
 * Challenge resolution is transport-independent protocol behavior. Its
 * implementation lives in backgammon-protocol so clients and Freenet
 * contracts interpret identical authenticated evidence the same way.
 */
pub use backgammon_protocol::{
    resolve_challenge, resolve_challenge_at, ChallengeResolution, ChallengeTerminalEvidence,
};
