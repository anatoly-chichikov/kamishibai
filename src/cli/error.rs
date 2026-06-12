//! Typed CLI refusals that decide a process exit code.
//!
//! A [`Refusal`] marks an error whose exit code carries meaning for scripts:
//! `2` — the invocation itself is wrong ([`usage`]), `3` — the named session
//! does not exist ([`not_found`]), `4` — the session is not in the right
//! lifecycle state yet ([`not_ready`], the only retryable refusal), `5` — an
//! omitted session id matched several sessions ([`ambiguous`], which carries
//! the candidates for the JSON envelope). Anything else is an operational
//! failure and exits `1`. The top-level runner maps an error to its code
//! through [`exit_code`] and, in JSON mode, to its stdout envelope through
//! [`json_line`], keeping the code↔meaning table in one place.

use std::error::Error;
use std::fmt;

use serde::Serialize;

/// A refusal carrying its process exit code: the caller must change the
/// invocation (2), the session id (3), wait for the session (4), or pick one
/// of several sessions (5, with the candidates attached for the envelope).
#[derive(Clone, Debug, PartialEq)]
pub(in crate::cli) struct Refusal {
    exit: u8,
    message: String,
    sessions: Option<serde_json::Value>,
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
        sessions: None,
    })
}

/// Build an `anyhow` error for a session that does not exist (exit code 3).
pub(in crate::cli) fn not_found(message: impl Into<String>) -> anyhow::Error {
    anyhow::Error::new(Refusal {
        exit: 3,
        message: message.into(),
        sessions: None,
    })
}

/// Build an `anyhow` error for a session not in the right state yet (exit 4).
pub(in crate::cli) fn not_ready(message: impl Into<String>) -> anyhow::Error {
    anyhow::Error::new(Refusal {
        exit: 4,
        message: message.into(),
        sessions: None,
    })
}

/// Build an `anyhow` error for an omitted id matching several sessions (exit
/// 5); `sessions` is the pre-serialized candidate list the JSON envelope shows.
pub(in crate::cli) fn ambiguous(
    message: impl Into<String>,
    sessions: serde_json::Value,
) -> anyhow::Error {
    anyhow::Error::new(Refusal {
        exit: 5,
        message: message.into(),
        sessions: Some(sessions),
    })
}

/// Return the refusal exit code an error carries, or None for an operational
/// failure (the catch-all exit 1).
#[must_use]
pub(in crate::cli) fn exit_code(error: &anyhow::Error) -> Option<u8> {
    error.downcast_ref::<Refusal>().map(|refusal| refusal.exit)
}

/// Map one exit code to its stable JSON error-code word.
#[must_use]
fn code_word(exit: u8) -> &'static str {
    match exit {
        2 => "usage",
        3 => "not_found",
        4 => "not_ready",
        5 => "ambiguous",
        _ => "operational",
    }
}

#[derive(Serialize)]
struct Envelope {
    ok: bool,
    error: ErrorDoc,
    #[serde(skip_serializing_if = "Option::is_none")]
    sessions: Option<serde_json::Value>,
}

#[derive(Serialize)]
struct ErrorDoc {
    code: &'static str,
    exit: u8,
    message: String,
}

/// Render one error as the single-line JSON envelope `--json` mode prints on
/// stdout: `{"ok":false,"error":{"code":…,"exit":…,"message":…}}`, plus a
/// top-level `sessions` array on the ambiguous refusal.
#[must_use]
pub(in crate::cli) fn json_line(error: &anyhow::Error) -> String {
    let exit = exit_code(error).unwrap_or(1);
    let envelope = Envelope {
        ok: false,
        error: ErrorDoc {
            code: code_word(exit),
            exit,
            message: format!("{error:#}"),
        },
        sessions: error
            .downcast_ref::<Refusal>()
            .and_then(|refusal| refusal.sessions.clone()),
    };
    serde_json::to_string(&envelope).expect("invariant: the error envelope always serializes")
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

    #[test]
    fn a_refusal_renders_as_a_machine_readable_envelope() {
        let parsed: serde_json::Value =
            serde_json::from_str(json_line(&not_found("no session 'ghost'")).as_str())
                .expect("the envelope must be valid JSON");
        assert_eq!(
            (
                parsed["ok"].as_bool(),
                parsed["error"]["code"].as_str(),
                parsed["error"]["exit"].as_u64(),
                parsed["error"]["message"].as_str(),
            ),
            (
                Some(false),
                Some("not_found"),
                Some(3),
                Some("no session 'ghost'")
            ),
            "a refusal must render as the ok:false envelope with its code word and exit"
        );
    }

    #[test]
    fn an_ambiguous_refusal_maps_to_exit_five() {
        assert_eq!(
            exit_code(&ambiguous("2 sessions; pass an id", serde_json::json!([]))),
            Some(5),
            "an ambiguous refusal must map to exit code 5"
        );
    }

    #[test]
    fn an_ambiguous_envelope_carries_its_sessions_payload() {
        let payload = serde_json::json!([{"id": "fr-1"}, {"id": "fr-2"}]);
        let parsed: serde_json::Value =
            serde_json::from_str(json_line(&ambiguous("2 sessions; pass an id", payload)).as_str())
                .expect("the envelope must be valid JSON");
        assert_eq!(
            (
                parsed["error"]["code"].as_str(),
                parsed["sessions"][1]["id"].as_str(),
            ),
            (Some("ambiguous"), Some("fr-2")),
            "an ambiguous envelope must carry the candidate sessions at the top level"
        );
    }

    #[test]
    fn a_non_ambiguous_envelope_omits_the_sessions_key() {
        let parsed: serde_json::Value =
            serde_json::from_str(json_line(&not_found("no session 'ghost'")).as_str())
                .expect("the envelope must be valid JSON");
        assert_eq!(
            parsed.get("sessions"),
            None,
            "envelopes other than ambiguous must stay byte-identical, without a sessions key"
        );
    }

    #[test]
    fn an_operational_error_envelope_falls_back_to_exit_one() {
        let parsed: serde_json::Value =
            serde_json::from_str(json_line(&anyhow::anyhow!("disk on fire")).as_str())
                .expect("the envelope must be valid JSON");
        assert_eq!(
            (
                parsed["error"]["code"].as_str(),
                parsed["error"]["exit"].as_u64()
            ),
            (Some("operational"), Some(1)),
            "an operational error must render code operational with exit 1"
        );
    }
}
