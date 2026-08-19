use backgammon_protocol::{
    build_genesis_game_action, encode_action_signing_message_v4, verify_action_authentication_v4,
    verify_action_signature_v4, verify_typed_action_history, Action, ActionAuthentication,
    ActionId, ActionSignature, ActionSigningBody, GameActionRecord, GameConfiguration, GameId,
    PlayerId, PROTOCOL_VERSION,
};
use ed25519_dalek::{Signer, SigningKey};
use serde::{Deserialize, Serialize};

/// Exact data that both peers must agree on before either signs genesis.
///
/// No signature or private material is contained here. Each peer independently
/// reconstructs the canonical sequence-zero GameActionRecord from this
/// proposal before signing or accepting a signature share.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenesisProposal {
    pub protocol_version: u16,
    pub game_id: GameId,
    pub action_id: ActionId,
    pub configuration: GameConfiguration,
}

impl GenesisProposal {
    pub fn new(game_id: GameId, action_id: ActionId, configuration: GameConfiguration) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            game_id,
            action_id,
            configuration,
        }
    }

    pub fn build_record(&self) -> Result<GameActionRecord, String> {
        if self.protocol_version != PROTOCOL_VERSION {
            return Err(format!(
                "Genesis proposal protocol version mismatch: expected {}, got {}.",
                PROTOCOL_VERSION, self.protocol_version
            ));
        }

        build_genesis_game_action(self.game_id, self.action_id, self.configuration.clone())
            .map_err(|error| format!("Could not build canonical genesis action: {error:?}"))
    }

    pub fn verify(&self) -> Result<(), String> {
        self.build_record().map(|_| ())
    }
}

/// One peer's signature over the canonical finalized genesis action.
///
/// The PlayerId identifies which configured participant supplied the share.
/// It is a public Ed25519 verifying key, not secret key material.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenesisSignatureShare {
    pub player_id: PlayerId,
    pub signature: ActionSignature,
}

fn signing_body(record: &GameActionRecord) -> ActionSigningBody {
    ActionSigningBody {
        protocol_version: record.protocol_version,
        game_id: record.game_id,
        action_id: record.action_id,
        sequence: record.sequence,
        previous_state_hash: record.previous_state_hash,
        resulting_state_hash: record.resulting_state_hash,
        payload: record.payload.clone(),
    }
}

fn participant_slot(
    configuration: &GameConfiguration,
    player_id: &PlayerId,
) -> Result<ParticipantSlot, String> {
    if *player_id == configuration.white.id {
        Ok(ParticipantSlot::White)
    } else if *player_id == configuration.black.id {
        Ok(ParticipantSlot::Black)
    } else {
        Err("Genesis signature share was produced by a nonparticipant.".to_owned())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ParticipantSlot {
    White,
    Black,
}

/// Signs the exact canonical genesis action represented by `proposal`.
///
/// The supplied identity must be one of the two PlayerIds in the proposal.
/// Only the signature and public PlayerId leave this function.
pub fn sign_genesis_proposal(
    proposal: &GenesisProposal,
    signing_key: &SigningKey,
) -> Result<GenesisSignatureShare, String> {
    let record = proposal.build_record()?;
    let player_id = signing_key.verifying_key().to_bytes();

    participant_slot(&proposal.configuration, &player_id)?;

    let body = signing_body(&record);
    let message = encode_action_signing_message_v4(&body)?;

    let share = GenesisSignatureShare {
        player_id,
        signature: ActionSignature::from_bytes(signing_key.sign(&message).to_bytes()),
    };

    /*
     * Verify our own output through the same protocol verifier that will be
     * used for hostile peer input.
     */
    verify_action_signature_v4(&share.player_id, share.signature.as_bytes(), &body)?;

    Ok(share)
}

/// Verifies one received genesis signature share against this exact proposal.
pub fn verify_genesis_signature_share(
    proposal: &GenesisProposal,
    share: &GenesisSignatureShare,
) -> Result<(), String> {
    let record = proposal.build_record()?;

    participant_slot(&proposal.configuration, &share.player_id)?;
    share.signature.verify()?;

    let body = signing_body(&record);

    verify_action_signature_v4(&share.player_id, share.signature.as_bytes(), &body)
}

/// Combines exactly one valid White share and one valid Black share into the
/// authenticated sequence-zero wire action.
///
/// Share ordering is irrelevant. Duplicate participants, nonparticipants,
/// malformed signatures, signatures over a different proposal, and incomplete
/// handshakes are rejected before an Action is returned.
pub fn assemble_authenticated_genesis(
    proposal: &GenesisProposal,
    shares: &[GenesisSignatureShare],
) -> Result<Action, String> {
    if shares.len() != 2 {
        return Err(format!(
            "Authenticated genesis requires exactly two signature shares; found {}.",
            shares.len()
        ));
    }

    let record = proposal.build_record()?;
    let body = signing_body(&record);

    let mut white_signature = None;
    let mut black_signature = None;

    for share in shares {
        verify_genesis_signature_share(proposal, share)?;

        match participant_slot(&proposal.configuration, &share.player_id)? {
            ParticipantSlot::White => {
                if white_signature.is_some() {
                    return Err("Duplicate White genesis signature share.".to_owned());
                }

                white_signature = Some(share.signature.clone());
            }
            ParticipantSlot::Black => {
                if black_signature.is_some() {
                    return Err("Duplicate Black genesis signature share.".to_owned());
                }

                black_signature = Some(share.signature.clone());
            }
        }
    }

    let authentication = ActionAuthentication::Genesis {
        white_signature: white_signature
            .ok_or_else(|| "Missing White genesis signature share.".to_owned())?,
        black_signature: black_signature
            .ok_or_else(|| "Missing Black genesis signature share.".to_owned())?,
    };

    /*
     * Enforce the complete protocol-v4 genesis authorization policy locally
     * before constructing wire data.
     */
    verify_action_authentication_v4(&body, &authentication, &proposal.configuration)?;

    let action = Action::from_authenticated_game_action_record(&record, authentication)?;

    /*
     * Finally verify the complete authenticated sequence-zero history exactly
     * as the Freenet contract will verify it.
     */
    verify_typed_action_history(std::slice::from_ref(&action))?;

    Ok(action)
}

#[cfg(test)]
mod tests {
    use super::*;
    use backgammon_protocol::PlayerDescriptor;

    fn fixture() -> (GenesisProposal, SigningKey, SigningKey) {
        let white_key = SigningKey::from_bytes(&[41; 32]);
        let black_key = SigningKey::from_bytes(&[42; 32]);

        let proposal = GenesisProposal::new(
            [7; 32],
            [1; 32],
            GameConfiguration {
                white: PlayerDescriptor {
                    id: white_key.verifying_key().to_bytes(),
                    display_name: "White".to_owned(),
                },
                black: PlayerDescriptor {
                    id: black_key.verifying_key().to_bytes(),
                    display_name: "Black".to_owned(),
                },
                match_length: 1,
            },
        );

        (proposal, white_key, black_key)
    }

    #[test]
    fn proposal_builds_canonical_genesis_record() {
        let (proposal, _, _) = fixture();

        let first = proposal.build_record().unwrap();
        let second = proposal.build_record().unwrap();

        assert_eq!(first, second);
        assert_eq!(first.sequence, 0);
        assert_eq!(first.game_id, proposal.game_id);
        assert_eq!(first.action_id, proposal.action_id);
    }

    #[test]
    fn valid_signature_shares_assemble_in_either_order() {
        let (proposal, white_key, black_key) = fixture();

        let white = sign_genesis_proposal(&proposal, &white_key).unwrap();
        let black = sign_genesis_proposal(&proposal, &black_key).unwrap();

        let white_first =
            assemble_authenticated_genesis(&proposal, &[white.clone(), black.clone()]).unwrap();

        let black_first = assemble_authenticated_genesis(&proposal, &[black, white]).unwrap();

        assert_eq!(white_first, black_first);
        verify_typed_action_history(std::slice::from_ref(&white_first)).unwrap();
    }

    #[test]
    fn nonparticipant_cannot_sign_genesis_proposal() {
        let (proposal, _, _) = fixture();
        let outsider = SigningKey::from_bytes(&[99; 32]);

        assert!(sign_genesis_proposal(&proposal, &outsider).is_err());
    }

    #[test]
    fn duplicate_participant_share_is_rejected() {
        let (proposal, white_key, _) = fixture();

        let white = sign_genesis_proposal(&proposal, &white_key).unwrap();

        assert!(assemble_authenticated_genesis(&proposal, &[white.clone(), white]).is_err());
    }

    #[test]
    fn incomplete_genesis_signature_set_is_rejected() {
        let (proposal, white_key, _) = fixture();

        let white = sign_genesis_proposal(&proposal, &white_key).unwrap();

        assert!(assemble_authenticated_genesis(&proposal, &[white]).is_err());
    }

    #[test]
    fn malformed_signature_share_is_rejected() {
        let (proposal, white_key, black_key) = fixture();

        let white = sign_genesis_proposal(&proposal, &white_key).unwrap();
        let mut black = sign_genesis_proposal(&proposal, &black_key).unwrap();

        black.signature = ActionSignature(vec![0; 63]);

        assert!(verify_genesis_signature_share(&proposal, &black).is_err());
        assert!(assemble_authenticated_genesis(&proposal, &[white, black]).is_err());
    }

    #[test]
    fn signature_share_from_different_proposal_is_rejected() {
        let (proposal, white_key, black_key) = fixture();

        let white = sign_genesis_proposal(&proposal, &white_key).unwrap();

        let mut different = proposal.clone();
        different.action_id = [2; 32];

        let black = sign_genesis_proposal(&different, &black_key).unwrap();

        assert!(assemble_authenticated_genesis(&proposal, &[white, black]).is_err());
    }
}
