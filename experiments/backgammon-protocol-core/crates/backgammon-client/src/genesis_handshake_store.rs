use std::io::Cursor;

use backgammon_protocol::{Action, GameId, PlayerId};
use ciborium::{de::from_reader, ser::into_writer};
use serde::{Deserialize, Serialize};

use crate::genesis_handshake::{
    assemble_authenticated_genesis, verify_genesis_signature_share, GenesisProposal,
    GenesisSignatureShare,
};

/*
 * A genesis handshake contains only fixed-size identifiers/signatures plus two
 * display names whose protocol limit is 48 bytes each. Four KiB therefore
 * leaves generous encoding headroom while sharply bounding hostile or corrupt
 * persisted input before CBOR decoding.
 */
pub const MAX_GENESIS_HANDSHAKE_BYTES: usize = 4096;
const STORAGE_PREFIX: &str = "freenet-backgammon.genesis-handshake.v1";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredGenesisHandshake {
    pub proposal: GenesisProposal,
    pub local_player_id: PlayerId,
    pub shares: Vec<GenesisSignatureShare>,
}

impl StoredGenesisHandshake {
    pub fn new(proposal: GenesisProposal, local_player_id: PlayerId) -> Result<Self, String> {
        let stored = Self {
            proposal,
            local_player_id,
            shares: Vec::new(),
        };

        stored.verify()?;
        Ok(stored)
    }

    pub fn verify(&self) -> Result<(), String> {
        self.proposal.verify()?;

        let configuration = &self.proposal.configuration;

        if self.local_player_id != configuration.white.id
            && self.local_player_id != configuration.black.id
        {
            return Err(
                "Stored genesis handshake belongs to a nonparticipant local identity.".to_owned(),
            );
        }

        if self.shares.len() > 2 {
            return Err(format!(
                "Stored genesis handshake contains too many signature shares: {}.",
                self.shares.len()
            ));
        }

        let mut saw_white = false;
        let mut saw_black = false;

        for share in &self.shares {
            verify_genesis_signature_share(&self.proposal, share)?;

            if share.player_id == configuration.white.id {
                if saw_white {
                    return Err(
                        "Stored genesis handshake contains duplicate White signature shares."
                            .to_owned(),
                    );
                }
                saw_white = true;
            } else if share.player_id == configuration.black.id {
                if saw_black {
                    return Err(
                        "Stored genesis handshake contains duplicate Black signature shares."
                            .to_owned(),
                    );
                }
                saw_black = true;
            } else {
                /*
                 * verify_genesis_signature_share() already rejects this case,
                 * but keep the storage invariant explicit.
                 */
                return Err(
                    "Stored genesis handshake contains a nonparticipant signature share."
                        .to_owned(),
                );
            }
        }

        /*
         * Exactly two shares must constitute a fully valid authenticated
         * genesis action, not merely two individually valid signatures.
         */
        if self.shares.len() == 2 {
            assemble_authenticated_genesis(&self.proposal, &self.shares)?;
        }

        Ok(())
    }

    /*
     * Exact duplicate delivery is idempotent. A second distinct share from the
     * same participant is rejected by verify(), as is any hostile share.
     */
    pub fn add_share(&mut self, share: GenesisSignatureShare) -> Result<(), String> {
        self.verify()?;

        if self.shares.iter().any(|existing| existing == &share) {
            return Ok(());
        }

        let mut candidate = self.clone();
        candidate.shares.push(share);
        candidate.verify()?;

        *self = candidate;
        Ok(())
    }

    pub fn authenticated_genesis(&self) -> Result<Option<Action>, String> {
        self.verify()?;

        if self.shares.len() != 2 {
            return Ok(None);
        }

        assemble_authenticated_genesis(&self.proposal, &self.shares).map(Some)
    }

    pub fn encode(&self) -> Result<Vec<u8>, String> {
        self.verify()?;

        let mut encoded = Vec::new();
        into_writer(self, &mut encoded)
            .map_err(|error| format!("Could not encode stored genesis handshake: {error:?}"))?;

        if encoded.len() > MAX_GENESIS_HANDSHAKE_BYTES {
            return Err(format!(
                "Encoded genesis handshake exceeds {} bytes.",
                MAX_GENESIS_HANDSHAKE_BYTES
            ));
        }

        Ok(encoded)
    }

    pub fn decode(encoded: &[u8]) -> Result<Self, String> {
        if encoded.len() > MAX_GENESIS_HANDSHAKE_BYTES {
            return Err(format!(
                "Stored genesis handshake exceeds {} bytes.",
                MAX_GENESIS_HANDSHAKE_BYTES
            ));
        }

        let mut cursor = Cursor::new(encoded);

        let decoded: Self = from_reader(&mut cursor)
            .map_err(|error| format!("Could not decode stored genesis handshake: {error:?}"))?;

        if cursor.position() != encoded.len() as u64 {
            return Err("Stored genesis handshake contains trailing noncanonical data.".to_owned());
        }

        decoded.verify()?;

        let canonical = decoded.encode()?;
        if canonical != encoded {
            return Err("Stored genesis handshake is not canonically encoded.".to_owned());
        }

        Ok(decoded)
    }

    pub fn game_id(&self) -> GameId {
        self.proposal.game_id
    }
}

pub fn genesis_handshake_storage_key(game_id: &GameId) -> String {
    format!("{STORAGE_PREFIX}.{}", encode_hex(game_id))
}

#[cfg(target_arch = "wasm32")]
pub fn store_genesis_handshake(stored: &StoredGenesisHandshake) -> Result<(), String> {
    stored.verify()?;

    let storage = browser_storage()?;
    let key = genesis_handshake_storage_key(&stored.game_id());
    let encoded = stored.encode()?;
    let encoded_hex = encode_hex(&encoded);

    storage
        .set_item(&key, &encoded_hex)
        .map_err(|error| format!("Could not persist the genesis handshake: {error:?}"))?;

    /*
     * Read the exact value back before reporting success. A refresh-safe
     * handshake must not depend on state that browser storage failed to retain
     * byte-for-byte.
     */
    let persisted_hex = storage
        .get_item(&key)
        .map_err(|error| format!("Could not verify the persisted genesis handshake: {error:?}"))?
        .ok_or_else(|| "Browser storage did not retain the genesis handshake.".to_owned())?;

    let persisted_bytes = decode_hex_bounded(&persisted_hex, MAX_GENESIS_HANDSHAKE_BYTES)?;
    let persisted = StoredGenesisHandshake::decode(&persisted_bytes)?;

    if persisted != *stored
        || persisted_bytes != encoded
        || encode_hex(&persisted_bytes) != persisted_hex
    {
        let _ = storage.remove_item(&key);

        return Err("Persisted genesis handshake failed exact round-trip verification.".to_owned());
    }

    Ok(())
}

#[cfg(target_arch = "wasm32")]
pub fn load_genesis_handshake(
    game_id: &GameId,
    local_player_id: &PlayerId,
) -> Result<Option<StoredGenesisHandshake>, String> {
    let storage = browser_storage()?;
    let key = genesis_handshake_storage_key(game_id);

    let Some(encoded_hex) = storage
        .get_item(&key)
        .map_err(|error| format!("Could not read the genesis handshake: {error:?}"))?
    else {
        return Ok(None);
    };

    let encoded = decode_hex_bounded(&encoded_hex, MAX_GENESIS_HANDSHAKE_BYTES)?;
    let stored = StoredGenesisHandshake::decode(&encoded)?;

    if stored.game_id() != *game_id {
        return Err("Stored genesis handshake belongs to a different game.".to_owned());
    }

    /*
     * A local identity reset must never silently inherit a handshake signed or
     * negotiated under the previous identity.
     */
    if stored.local_player_id != *local_player_id {
        return Err("Stored genesis handshake belongs to a different local identity.".to_owned());
    }

    if encode_hex(&encoded) != encoded_hex {
        return Err("Stored genesis handshake hexadecimal encoding is noncanonical.".to_owned());
    }

    Ok(Some(stored))
}

#[cfg(target_arch = "wasm32")]
pub fn remove_genesis_handshake(game_id: &GameId) -> Result<(), String> {
    let storage = browser_storage()?;
    let key = genesis_handshake_storage_key(game_id);

    storage
        .remove_item(&key)
        .map_err(|error| format!("Could not remove the genesis handshake: {error:?}"))?;

    if storage
        .get_item(&key)
        .map_err(|error| format!("Could not verify genesis-handshake removal: {error:?}"))?
        .is_some()
    {
        return Err("Browser storage retained the genesis handshake after removal.".to_owned());
    }

    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn browser_storage() -> Result<web_sys::Storage, String> {
    let window = web_sys::window().ok_or_else(|| "Browser window is unavailable.".to_owned())?;

    window
        .local_storage()
        .map_err(|error| format!("Browser storage is unavailable: {error:?}"))?
        .ok_or_else(|| "Browser local storage is disabled.".to_owned())
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";

    let mut encoded = String::with_capacity(bytes.len() * 2);

    for byte in bytes {
        encoded.push(HEX[usize::from(byte >> 4)] as char);
        encoded.push(HEX[usize::from(byte & 0x0f)] as char);
    }

    encoded
}

#[cfg(any(test, target_arch = "wasm32"))]
fn decode_hex_bounded(encoded: &str, maximum_bytes: usize) -> Result<Vec<u8>, String> {
    if encoded.len() % 2 != 0 {
        return Err("Stored genesis handshake has an odd hexadecimal length.".to_owned());
    }

    let decoded_len = encoded.len() / 2;

    if decoded_len > maximum_bytes {
        return Err(format!(
            "Stored genesis handshake exceeds {} decoded bytes.",
            maximum_bytes
        ));
    }

    let bytes = encoded.as_bytes();
    let mut decoded = Vec::with_capacity(decoded_len);

    for pair in bytes.chunks_exact(2) {
        let high = decode_hex_nibble(pair[0])?;
        let low = decode_hex_nibble(pair[1])?;
        decoded.push((high << 4) | low);
    }

    Ok(decoded)
}

#[cfg(any(test, target_arch = "wasm32"))]
fn decode_hex_nibble(value: u8) -> Result<u8, String> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        _ => Err("Stored genesis handshake contains noncanonical hexadecimal data.".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use backgammon_protocol::{GameConfiguration, PlayerDescriptor};
    use ed25519_dalek::SigningKey;

    use crate::genesis_handshake::sign_genesis_proposal;

    fn fixture() -> (GenesisProposal, SigningKey, SigningKey) {
        let white_key = SigningKey::from_bytes(&[51; 32]);
        let black_key = SigningKey::from_bytes(&[52; 32]);

        let proposal = GenesisProposal::new(
            [17; 32],
            [18; 32],
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
    fn storage_keys_are_game_scoped_and_canonical() {
        let first = genesis_handshake_storage_key(&[17; 32]);
        let second = genesis_handshake_storage_key(&[18; 32]);

        assert_ne!(first, second);
        assert_eq!(first, format!("{STORAGE_PREFIX}.{}", "11".repeat(32)));
    }

    #[test]
    fn canonical_handshake_hex_round_trips() {
        let (proposal, white_key, _) = fixture();

        let stored =
            StoredGenesisHandshake::new(proposal, white_key.verifying_key().to_bytes()).unwrap();

        let encoded = stored.encode().unwrap();
        let encoded_hex = encode_hex(&encoded);

        assert_eq!(
            decode_hex_bounded(&encoded_hex, MAX_GENESIS_HANDSHAKE_BYTES).unwrap(),
            encoded
        );
    }

    #[test]
    fn malformed_or_noncanonical_handshake_hex_is_rejected() {
        assert!(decode_hex_bounded("0", 16).is_err());
        assert!(decode_hex_bounded("GG", 16).is_err());
        assert!(decode_hex_bounded("AA", 16).is_err());
        assert!(decode_hex_bounded("0/", 16).is_err());
    }

    #[test]
    fn oversized_handshake_hex_is_rejected_before_decoded_allocation() {
        let encoded = "00".repeat(MAX_GENESIS_HANDSHAKE_BYTES + 1);

        assert!(decode_hex_bounded(&encoded, MAX_GENESIS_HANDSHAKE_BYTES).is_err());
    }

    #[test]
    fn empty_handshake_round_trips_canonically() {
        let (proposal, white_key, _) = fixture();

        let stored =
            StoredGenesisHandshake::new(proposal, white_key.verifying_key().to_bytes()).unwrap();

        let encoded = stored.encode().unwrap();
        let decoded = StoredGenesisHandshake::decode(&encoded).unwrap();

        assert_eq!(decoded, stored);
        assert_eq!(decoded.encode().unwrap(), encoded);
        assert!(decoded.authenticated_genesis().unwrap().is_none());
    }

    #[test]
    fn one_valid_signature_share_round_trips() {
        let (proposal, white_key, _) = fixture();

        let mut stored =
            StoredGenesisHandshake::new(proposal.clone(), white_key.verifying_key().to_bytes())
                .unwrap();

        stored
            .add_share(sign_genesis_proposal(&proposal, &white_key).unwrap())
            .unwrap();

        let encoded = stored.encode().unwrap();
        let decoded = StoredGenesisHandshake::decode(&encoded).unwrap();

        assert_eq!(decoded, stored);
        assert_eq!(decoded.shares.len(), 1);
        assert!(decoded.authenticated_genesis().unwrap().is_none());
    }

    #[test]
    fn two_valid_signature_shares_recover_authenticated_genesis() {
        let (proposal, white_key, black_key) = fixture();

        let mut stored =
            StoredGenesisHandshake::new(proposal.clone(), white_key.verifying_key().to_bytes())
                .unwrap();

        stored
            .add_share(sign_genesis_proposal(&proposal, &white_key).unwrap())
            .unwrap();
        stored
            .add_share(sign_genesis_proposal(&proposal, &black_key).unwrap())
            .unwrap();

        let encoded = stored.encode().unwrap();
        let decoded = StoredGenesisHandshake::decode(&encoded).unwrap();

        assert_eq!(decoded, stored);
        assert!(decoded.authenticated_genesis().unwrap().is_some());
    }

    #[test]
    fn nonparticipant_local_identity_is_rejected() {
        let (proposal, _, _) = fixture();
        let outsider = SigningKey::from_bytes(&[99; 32]);

        assert!(
            StoredGenesisHandshake::new(proposal, outsider.verifying_key().to_bytes(),).is_err()
        );
    }

    #[test]
    fn duplicate_participant_entries_are_invalid() {
        let (proposal, white_key, _) = fixture();
        let white = sign_genesis_proposal(&proposal, &white_key).unwrap();

        let stored = StoredGenesisHandshake {
            proposal,
            local_player_id: white_key.verifying_key().to_bytes(),
            shares: vec![white.clone(), white],
        };

        assert!(stored.verify().is_err());
        assert!(stored.encode().is_err());
    }

    #[test]
    fn duplicate_delivery_does_not_bypass_existing_state_verification() {
        let (proposal, white_key, _) = fixture();
        let white = sign_genesis_proposal(&proposal, &white_key).unwrap();
        let outsider = SigningKey::from_bytes(&[99; 32]);

        let mut stored = StoredGenesisHandshake {
            proposal,
            local_player_id: outsider.verifying_key().to_bytes(),
            shares: vec![white.clone()],
        };

        assert!(stored.add_share(white).is_err());
    }

    #[test]
    fn exact_duplicate_share_delivery_is_idempotent() {
        let (proposal, white_key, _) = fixture();
        let white = sign_genesis_proposal(&proposal, &white_key).unwrap();

        let mut stored =
            StoredGenesisHandshake::new(proposal, white_key.verifying_key().to_bytes()).unwrap();

        stored.add_share(white.clone()).unwrap();
        stored.add_share(white).unwrap();

        assert_eq!(stored.shares.len(), 1);
    }

    #[test]
    fn share_from_different_proposal_is_rejected() {
        let (proposal, white_key, black_key) = fixture();

        let mut different = proposal.clone();
        different.action_id = [19; 32];

        let white = sign_genesis_proposal(&proposal, &white_key).unwrap();
        let wrong_black = sign_genesis_proposal(&different, &black_key).unwrap();

        let stored = StoredGenesisHandshake {
            proposal,
            local_player_id: white_key.verifying_key().to_bytes(),
            shares: vec![white, wrong_black],
        };

        assert!(stored.verify().is_err());
    }

    #[test]
    fn oversized_and_trailing_encoded_data_are_rejected() {
        let oversized = vec![0_u8; MAX_GENESIS_HANDSHAKE_BYTES + 1];
        assert!(StoredGenesisHandshake::decode(&oversized).is_err());

        let (proposal, white_key, _) = fixture();
        let stored =
            StoredGenesisHandshake::new(proposal, white_key.verifying_key().to_bytes()).unwrap();

        let mut encoded = stored.encode().unwrap();
        encoded.push(0);

        assert!(StoredGenesisHandshake::decode(&encoded).is_err());
    }
}
