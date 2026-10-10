//! Clock-only renewal policy. Identity, connection and pending-request gates
//! are enforced by the caller before requesting a fresh durable revision.

pub const RENEW_BEFORE_EXPIRY_SECONDS: u64 = 5 * 60;
pub const RETRY_INTERVAL_SECONDS: u64 = 30;

pub fn presence_renewal_due(
    now: u64,
    expires_at: Option<u64>,
    last_attempt: Option<u64>,
) -> bool {
    // A backward clock step must not trigger a burst of reservations.
    if last_attempt.is_some_and(|last| now.saturating_sub(last) < RETRY_INTERVAL_SECONDS) {
        return false;
    }
    expires_at.is_none_or(|expires| expires.saturating_sub(now) <= RENEW_BEFORE_EXPIRY_SECONDS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renews_at_margin_and_after_expiry() {
        assert!(!presence_renewal_due(299, Some(600), None));
        assert!(presence_renewal_due(300, Some(600), None));
        assert!(presence_renewal_due(600, Some(600), None));
        assert!(presence_renewal_due(900, Some(600), None));
    }

    #[test]
    fn missing_lease_is_due_but_retries_are_bounded() {
        assert!(presence_renewal_due(100, None, None));
        assert!(!presence_renewal_due(129, None, Some(100)));
        assert!(presence_renewal_due(130, None, Some(100)));
        assert!(!presence_renewal_due(99, None, Some(100)));
    }

    #[test]
    fn recent_submission_does_not_need_another_lease() {
        assert!(!presence_renewal_due(100, Some(700), Some(70)));
        assert!(presence_renewal_due(400, Some(700), Some(70)));
    }
}
