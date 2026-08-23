/*
 * Compatibility re-export.
 *
 * The authenticated genesis negotiation types and helpers are
 * transport-independent protocol primitives. Their implementation now lives
 * in backgammon-protocol so both the browser client and Freenet contracts can
 * use the same wire representation and verification rules.
 */
pub use backgammon_protocol::{
    assemble_authenticated_genesis, sign_genesis_proposal, verify_genesis_signature_share,
    GenesisProposal, GenesisSignatureShare,
};
