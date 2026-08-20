use backgammon_protocol::PlayerId;

const STORAGE_PREFIX: &str = "freenet-backgammon.presence-revision.v1";

/*
 * u64::MAX is 20 decimal digits. Keeping this bound explicit means corrupted
 * browser storage is rejected before parsing.
 */
const MAX_ENCODED_REVISION_BYTES: usize = 20;

/// Returns the browser-storage key for one cryptographic player identity.
///
/// Presence revisions belong to PlayerId, not to a display name. Resetting the
/// local identity therefore naturally starts a separate revision namespace.
pub fn presence_revision_storage_key(player_id: &PlayerId) -> String {
    format!("{STORAGE_PREFIX}.{}", encode_player_id(player_id))
}

/// Pure revision allocator used by both native tests and browser persistence.
///
/// Missing storage means no revision has ever been reserved, so the first
/// issued presence revision is 1. Revision 0 is deliberately invalid.
pub fn next_presence_revision(last_reserved_revision: Option<u64>) -> Result<u64, String> {
    last_reserved_revision
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| {
            "Presence revision counter is exhausted; refusing to reuse a revision.".to_owned()
        })
}

fn encode_revision(revision: u64) -> Result<String, String> {
    if revision == 0 {
        return Err("Presence revision zero is reserved and cannot be persisted.".to_owned());
    }

    Ok(revision.to_string())
}

fn decode_revision(encoded: &str) -> Result<u64, String> {
    if encoded.is_empty() {
        return Err("Stored presence revision is empty.".to_owned());
    }

    if encoded.len() > MAX_ENCODED_REVISION_BYTES {
        return Err(format!(
            "Stored presence revision exceeds \
             {MAX_ENCODED_REVISION_BYTES} decimal bytes."
        ));
    }

    if !encoded.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("Stored presence revision contains nondecimal data.".to_owned());
    }

    let revision = encoded
        .parse::<u64>()
        .map_err(|error| format!("Stored presence revision is invalid: {error}"))?;

    if revision == 0 {
        return Err("Stored presence revision zero is invalid.".to_owned());
    }

    let canonical = encode_revision(revision)?;

    if canonical != encoded {
        return Err("Stored presence revision is not canonically encoded.".to_owned());
    }

    Ok(revision)
}

/// Loads the highest presence revision already reserved by this PlayerId.
///
/// A stored reservation is durable issuance history. It must not be silently
/// reset merely because older network announcements have expired.
#[cfg(target_arch = "wasm32")]
pub fn load_reserved_presence_revision(player_id: &PlayerId) -> Result<Option<u64>, String> {
    let storage = browser_storage()?;
    let key = presence_revision_storage_key(player_id);

    let Some(encoded) = storage
        .get_item(&key)
        .map_err(|error| format!("Could not read the local presence revision: {error:?}"))?
    else {
        return Ok(None);
    };

    let revision = decode_revision(&encoded)?;

    if encode_revision(revision)? != encoded {
        return Err("Stored presence revision is not canonically encoded.".to_owned());
    }

    Ok(Some(revision))
}

/// Durably reserves and returns the next revision for this PlayerId.
///
/// IMPORTANT: callers must reserve first, then construct/sign/publish the
/// presence announcement using the returned revision. If a crash occurs after
/// reservation but before publication, the skipped revision is harmless. The
/// reverse order could allow a previously signed revision to be reused after
/// restart, producing authenticated same-revision equivocation.
///
/// localStorage does not provide an atomic cross-tab compare-and-swap. The
/// current alpha therefore assumes one active writer per persistent PlayerId.
/// The lobby resolver still detects contradictory same-revision announcements
/// should that assumption be violated.
#[cfg(target_arch = "wasm32")]
pub fn reserve_next_presence_revision(player_id: &PlayerId) -> Result<u64, String> {
    let storage = browser_storage()?;
    let key = presence_revision_storage_key(player_id);

    let last_reserved = match storage
        .get_item(&key)
        .map_err(|error| format!("Could not read the local presence revision: {error:?}"))?
    {
        Some(encoded) => Some(decode_revision(&encoded)?),
        None => None,
    };

    let next = next_presence_revision(last_reserved)?;
    let encoded = encode_revision(next)?;

    /*
     * Reserve before the caller signs or publishes anything using this
     * revision. A crash may create a harmless gap but never intentional reuse.
     */
    storage
        .set_item(&key, &encoded)
        .map_err(|error| format!("Could not reserve the next presence revision: {error:?}"))?;

    /*
     * Match the project's other durable-browser stores: immediately read the
     * value back and require the exact canonical representation before
     * reporting success.
     */
    let persisted = storage
        .get_item(&key)
        .map_err(|error| format!("Could not verify the reserved presence revision: {error:?}"))?
        .ok_or_else(|| {
            "Browser storage did not retain the reserved presence revision.".to_owned()
        })?;

    let persisted_revision = decode_revision(&persisted)?;

    if persisted_revision != next || persisted != encoded {
        /*
         * Do NOT remove or roll back the key here. Another browser context may
         * have advanced it. Failing closed is safer than restoring an older
         * revision.
         */
        return Err("Persisted presence revision failed exact round-trip verification.".to_owned());
    }

    Ok(next)
}

#[cfg(target_arch = "wasm32")]
fn browser_storage() -> Result<web_sys::Storage, String> {
    let window = web_sys::window().ok_or_else(|| "Browser window is unavailable.".to_owned())?;

    window
        .local_storage()
        .map_err(|error| format!("Browser storage is unavailable: {error:?}"))?
        .ok_or_else(|| "Browser local storage is disabled.".to_owned())
}

fn encode_player_id(player_id: &PlayerId) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";

    let mut encoded = String::with_capacity(player_id.len() * 2);

    for byte in player_id {
        encoded.push(HEX[usize::from(byte >> 4)] as char);
        encoded.push(HEX[usize::from(byte & 0x0f)] as char);
    }

    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_keys_are_player_identity_scoped_and_canonical() {
        let first = presence_revision_storage_key(&[0x11; 32]);
        let second = presence_revision_storage_key(&[0x22; 32]);

        assert_ne!(first, second);

        assert_eq!(first, format!("{STORAGE_PREFIX}.{}", "11".repeat(32)));

        assert_eq!(second, format!("{STORAGE_PREFIX}.{}", "22".repeat(32)));
    }

    #[test]
    fn first_revision_is_one() {
        assert_eq!(next_presence_revision(None), Ok(1));
    }

    #[test]
    fn revision_increments_strictly() {
        assert_eq!(next_presence_revision(Some(1)), Ok(2));
        assert_eq!(next_presence_revision(Some(41)), Ok(42));
        assert_eq!(next_presence_revision(Some(u64::MAX - 1)), Ok(u64::MAX));
    }

    #[test]
    fn exhausted_revision_counter_fails_closed() {
        assert!(next_presence_revision(Some(u64::MAX)).is_err());
    }

    #[test]
    fn canonical_revision_encoding_round_trips() {
        for revision in [1, 2, 42, u32::MAX as u64, u64::MAX] {
            let encoded = encode_revision(revision).unwrap();

            assert_eq!(decode_revision(&encoded), Ok(revision));
            assert_eq!(
                encode_revision(decode_revision(&encoded).unwrap()).unwrap(),
                encoded
            );
        }
    }

    #[test]
    fn zero_revision_is_rejected() {
        assert!(encode_revision(0).is_err());
        assert!(decode_revision("0").is_err());
    }

    #[test]
    fn malformed_or_noncanonical_revisions_are_rejected() {
        for encoded in ["", "00", "01", "+1", "-1", " 1", "1 ", "1.0", "abc"] {
            assert!(
                decode_revision(encoded).is_err(),
                "unexpectedly accepted {encoded:?}"
            );
        }
    }

    #[test]
    fn oversized_or_overflowing_revisions_are_rejected() {
        assert!(decode_revision("18446744073709551616").is_err());

        assert!(decode_revision(&"9".repeat(MAX_ENCODED_REVISION_BYTES + 1)).is_err());
    }

    #[test]
    fn player_id_encoding_is_fixed_width_lowercase_hex() {
        let mut player_id = [0_u8; 32];
        player_id[0] = 0xab;
        player_id[31] = 0xcd;

        let encoded = encode_player_id(&player_id);

        assert_eq!(encoded.len(), 64);
        assert!(encoded.starts_with("ab"));
        assert!(encoded.ends_with("cd"));
        assert!(encoded
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)));
    }
}
