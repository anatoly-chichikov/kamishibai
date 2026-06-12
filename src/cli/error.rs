//! Typed CLI refusals that decide a process exit code.
//!
//! A [`Refusal`] marks an error whose exit code carries meaning for scripts:
//! `2` — the invocation itself is wrong ([`usage`]), `3` — the named session
//! does not exist ([`not_found`]), `4` — the session is not in the right
//! lifecycle state yet ([`not_ready`], the only retryable refusal). Anything
//! else is an operational failure and exits `1`. The top-level runner maps an
//! error to its code through [`exit_code`], keeping the code↔meaning table in
//! one place.

use std::error::Error;
use std::fmt;

/// A refusal carrying its process exit code: the caller must change the
/// invocation (2), the session id (3), or wait for the session (4).
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::cli) struct Refusal {
    exit: u8,
    message: String,
}

impl fmt::Display for Refusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.message)
    }
}

impl Error for Refusal {}

/// Build an `anyhow` error for a malformed invocation (process exit code 2).
pub(in crate::cli) fn usage(message: impl Into<String>) -> anyhow::Error {
    anyhow::Error::new(Refusal {
        exit: 2,
        message: message.into(),
    })
}

/// Build an `anyhow` error for a session that does not exist (exit code 3).
pub(in crate::cli) fn not_found(message: impl Into<String>) -> anyhow::Error {
    anyhow::Error::new(Refusal {
        exit: 3,
        message: message.into(),
    })
}

/// Build an `anyhow` error for a session not in the right state yet (exit 4).
pub(in crate::cli) fn not_ready(message: impl Into<String>) -> anyhow::Error {
    anyhow::Error::new(Refusal {
        exit: 4,
        message: message.into(),
    })
}

/// Return the refusal exit code an error carries, or None for an operational
/// failure (the catch-all exit 1).
#[must_use]
pub(in crate::cli) fn exit_code(error: &anyhow::Error) -> Option<u8> {
    error.downcast_ref::<Refusal>().map(|refusal| refusal.exit)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_usage_refusal_maps_to_exit_two() {
        assert_eq!(
            exit_code(&usage("nothing to generate")),
            Some(2),
            "a usage refusal carried through anyhow must map to exit code 2"
        );
    }

    #[test]
    fn a_not_found_refusal_maps_to_exit_three() {
        assert_eq!(
            exit_code(&not_found("no session 'ghost'")),
            Some(3),
            "a not-found refusal must map to exit code 3"
        );
    }

    #[test]
    fn a_not_ready_refusal_maps_to_exit_four() {
        assert_eq!(
            exit_code(&not_ready("not ready (phase understood)")),
            Some(4),
            "a not-ready refusal must map to exit code 4"
        );
    }

    #[test]
    fn an_operational_error_has_no_dedicated_exit_code() {
        assert_eq!(
            exit_code(&anyhow::anyhow!("disk on fire")),
            None,
            "a plain operational error must fall through to the catch-all exit 1"
        );
    }
}
