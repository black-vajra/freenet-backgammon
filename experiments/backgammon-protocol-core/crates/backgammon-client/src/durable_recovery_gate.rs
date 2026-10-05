/// A delegate handle and registration from an older WebSocket cannot authorize
/// recovery on its replacement. The epoch is bumped before each connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DurableRecoveryReadiness {
    pub connection_epoch: u64,
    pub socket_open: bool,
    pub delegate_registered: bool,
    pub registered_epoch: Option<u64>,
}

impl DurableRecoveryReadiness {
    pub fn can_read(self) -> bool {
        self.socket_open
            && self.delegate_registered
            && self.registered_epoch == Some(self.connection_epoch)
    }
}

pub fn is_current_connection(callback_epoch: u64, active_epoch: u64) -> bool {
    callback_epoch == active_epoch
}

/// Claim the one publication opportunity only after a durable read returned an
/// actual record. A failed read (including a send rejected while connecting)
/// does not call this function and remains eligible on the next ready connection.
pub fn claim_recovery_publication(already_attempted: &mut bool) -> bool {
    if *already_attempted {
        false
    } else {
        *already_attempted = true;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_waits_for_open_socket_and_current_delegate_registration() {
        let mut state = DurableRecoveryReadiness {
            connection_epoch: 4,
            socket_open: false,
            delegate_registered: true,
            registered_epoch: Some(4),
        };
        assert!(!state.can_read()); // CONNECTING, even with a registered delegate
        state.socket_open = true;
        state.delegate_registered = false;
        assert!(!state.can_read()); // OPEN alone is not registration
        state.delegate_registered = true;
        assert!(state.can_read());
    }

    #[test]
    fn obsolete_connection_cannot_authorize_replacement() {
        let mut state = DurableRecoveryReadiness {
            connection_epoch: 5,
            socket_open: true,
            delegate_registered: true,
            registered_epoch: Some(4),
        };
        assert!(!state.can_read());
        assert!(!is_current_connection(4, state.connection_epoch));
        assert!(is_current_connection(5, state.connection_epoch));
        state.registered_epoch = Some(5);
        assert!(state.can_read());
        state.socket_open = false;
        assert!(!state.can_read());
    }

    #[test]
    fn failed_read_can_retry_after_open_without_republishing() {
        let mut attempted = false;
        let mut readiness = DurableRecoveryReadiness {
            connection_epoch: 9,
            socket_open: false,
            delegate_registered: false,
            registered_epoch: None,
        };
        assert!(!readiness.can_read());
        // A failed read does not claim the publication slot.
        assert!(!attempted);
        readiness.socket_open = true;
        readiness.delegate_registered = true;
        readiness.registered_epoch = Some(9);
        assert!(readiness.can_read());
        assert!(claim_recovery_publication(&mut attempted));
        assert!(!claim_recovery_publication(&mut attempted));
    }
}

/// Called only after the exact record was verified in authoritative lobby state.
/// A replacement belongs to another operation and must not be deleted or have
/// its pending status cleared by this observer.
pub fn confirmed_cleanup_readback(
    expected: &[u8],
    current: Option<&[u8]>,
) -> Result<bool, String> {
    match current {
        None => Ok(true),
        Some(value) if value == expected => {
            Err("Confirmed durable record remained after exact cleanup".to_owned())
        }
        Some(_) => Ok(false),
    }
}

#[cfg(test)]
mod cleanup_tests {
    use super::confirmed_cleanup_readback;

    #[test]
    fn concurrent_observer_already_removed_record() {
        assert_eq!(confirmed_cleanup_readback(b"accepted", None), Ok(true));
    }

    #[test]
    fn replacement_record_is_preserved_and_not_marked_complete() {
        assert_eq!(confirmed_cleanup_readback(b"accepted", Some(b"next")), Ok(false));
    }

    #[test]
    fn original_record_remaining_is_a_real_failure() {
        assert!(confirmed_cleanup_readback(b"accepted", Some(b"accepted")).is_err());
    }
}
