#![forbid(unsafe_code)]

use backgammon_core::CoreProtocolMarker;
use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u16 = 1;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ProtocolMarker {
    pub core: CoreProtocolMarker,
    pub protocol_version: u16,
}

impl Default for ProtocolMarker {
    fn default() -> Self {
        Self {
            core: CoreProtocolMarker::default(),
            protocol_version: PROTOCOL_VERSION,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_protocol_marker_is_consistent() {
        let marker = ProtocolMarker::default();

        assert_eq!(marker.core.version, PROTOCOL_VERSION);
        assert_eq!(marker.protocol_version, PROTOCOL_VERSION);
    }
}
