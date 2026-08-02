#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct CoreProtocolMarker {
    pub version: u16,
}

impl Default for CoreProtocolMarker {
    fn default() -> Self {
        Self { version: 1 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_marker_uses_version_one() {
        assert_eq!(CoreProtocolMarker::default().version, 1);
    }
}
