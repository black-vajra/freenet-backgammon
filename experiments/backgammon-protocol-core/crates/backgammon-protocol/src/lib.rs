#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u16 = 1;

#[derive(Serialize, Deserialize, Clone, Default, PartialEq, Eq, Debug)]
pub struct LedgerParameters {
    pub protocol_version: u16,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct Action {
    pub id: [u8; 32],
    pub sequence: u32,
    pub payload: Vec<u8>,
}

pub fn verify_action_sequences(actions: &[Action]) -> Result<(), String> {
    let mut sequences: Vec<u32> = actions.iter().map(|action| action.sequence).collect();
    sequences.sort_unstable();

    for (expected, actual) in sequences.into_iter().enumerate() {
        let expected =
            u32::try_from(expected).map_err(|_| "action sequence exceeds supported range")?;

        if actual < expected {
            return Err("duplicate action sequence".into());
        }

        if actual > expected {
            return Err("action sequence gap".into());
        }
    }

    Ok(())
}

impl LedgerParameters {
    pub const fn current() -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
        }
    }

    pub fn verify(&self) -> Result<(), String> {
        if self.protocol_version != PROTOCOL_VERSION {
            return Err("unsupported protocol version".into());
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_parameters_use_supported_version() {
        let parameters = LedgerParameters::current();

        assert_eq!(parameters.protocol_version, PROTOCOL_VERSION);
        assert_eq!(parameters.verify(), Ok(()));
    }

    #[test]
    fn unsupported_version_is_rejected() {
        let parameters = LedgerParameters {
            protocol_version: PROTOCOL_VERSION + 1,
        };

        assert_eq!(
            parameters.verify(),
            Err("unsupported protocol version".into())
        );
    }

    fn action(id: u8, sequence: u32) -> Action {
        Action {
            id: [id; 32],
            sequence,
            payload: vec![id],
        }
    }

    #[test]
    fn contiguous_sequences_are_valid_regardless_of_storage_order() {
        let actions = vec![action(3, 2), action(1, 0), action(2, 1)];

        assert_eq!(verify_action_sequences(&actions), Ok(()));
    }

    #[test]
    fn empty_sequence_is_valid() {
        assert_eq!(verify_action_sequences(&[]), Ok(()));
    }

    #[test]
    fn sequence_must_start_at_zero() {
        let actions = vec![action(1, 1)];

        assert_eq!(
            verify_action_sequences(&actions),
            Err("action sequence gap".into())
        );
    }

    #[test]
    fn duplicate_sequence_is_rejected() {
        let actions = vec![action(1, 0), action(2, 0)];

        assert_eq!(
            verify_action_sequences(&actions),
            Err("duplicate action sequence".into())
        );
    }

    #[test]
    fn sequence_gap_is_rejected() {
        let actions = vec![action(1, 0), action(3, 2)];

        assert_eq!(
            verify_action_sequences(&actions),
            Err("action sequence gap".into())
        );
    }
}
