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
}
