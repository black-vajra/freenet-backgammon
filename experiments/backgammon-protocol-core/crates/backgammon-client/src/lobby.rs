//! Compatibility re-export for transport-independent lobby presence.
//!
//! The canonical authenticated presence model now lives in
//! `backgammon-protocol` so both the browser client and Freenet lobby contract
//! use the exact same wire types and verification rules.

pub use backgammon_protocol::{
    resolve_player_presence, sign_presence_announcement, validate_display_name,
    verify_presence_announcement, verify_presence_announcement_at, PresenceAnnouncementBody,
    PresenceResolution, PresenceSignature, SignedPresenceAnnouncement, LOBBY_PROTOCOL_VERSION,
    MAX_PRESENCE_LIFETIME_SECONDS,
};
