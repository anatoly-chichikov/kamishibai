//! Attempt accounting for one artifact: how many tries were spent and why the
//! spent ones failed.
//!
//! One artifact gets one plain try plus a bounded number of retries on top of
//! it, so a surface never numbers the first try — it only starts counting once
//! a retry is actually under way. A retry is only meaningful to the user when
//! it carries its reason, so the tally never travels alone: every spent attempt
//! records one [`AttemptFault`], and image artifacts keep the archived picture
//! that was rejected so the shell can open it.

use std::path::{Path, PathBuf};

/// Absolute attempt ceiling shared by every generated artifact series: the
/// plain first try plus [`AttemptTally::retries`] retries on top of it.
pub(crate) const ARTIFACT_ATTEMPT_CEILING: u8 = 4;

/// Number of attempts already spent versus the absolute cap.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AttemptTally {
    done: u8,
    ceiling: u8,
}

impl AttemptTally {
    /// Start one tally with an explicit ceiling.
    pub fn new(ceiling: u8) -> Self {
        Self { done: 0, ceiling }
    }

    /// Return the number of attempts already spent.
    pub fn done(&self) -> u8 {
        self.done
    }

    /// Return the ceiling (typically 4).
    pub fn ceiling(&self) -> u8 {
        self.ceiling
    }

    /// Return how many retries the first try may be followed by (typically 3).
    pub fn retries(&self) -> u8 {
        self.ceiling.saturating_sub(1)
    }

    /// Return which retry is under way, or none while the first try still runs.
    pub fn retry(&self) -> Option<u8> {
        (self.done > 0 && self.done <= self.retries()).then_some(self.done)
    }

    /// Return whether the artifact has run out of retry budget.
    pub fn exhausted(&self) -> bool {
        self.done >= self.ceiling
    }

    /// Record one more attempt.
    pub fn spent(self) -> Self {
        let next = self.done.saturating_add(1);
        Self {
            done: next.min(self.ceiling),
            ceiling: self.ceiling,
        }
    }
}

/// Why one spent attempt did not produce its artifact.
///
/// `category` is the stable slug the renderer and the console share (`border`,
/// `topology`, `ocr`, `recall_text`, `color`, `legacy_gutter`, `other`, `error`);
/// `reason` is the sentence shown to the user; `artifact` points at whatever
/// this attempt did produce before it was rejected — the archived picture of a
/// picture attempt — and stays empty when the attempt failed before producing
/// anything at all.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttemptFault {
    category: String,
    reason: String,
    artifact: Option<PathBuf>,
}

impl AttemptFault {
    /// Create one fault from its category slug, reason, and rejected artifact.
    pub fn new(
        category: impl Into<String>,
        reason: impl Into<String>,
        artifact: Option<PathBuf>,
    ) -> Self {
        Self {
            category: category.into(),
            reason: reason.into(),
            artifact,
        }
    }

    /// Create one fault for a failure that never reached a provider verdict.
    pub fn failed(reason: impl Into<String>) -> Self {
        Self::new("error", reason, None)
    }

    /// Return the stable category slug.
    pub fn category(&self) -> &str {
        self.category.as_str()
    }

    /// Return the user-facing reason sentence.
    pub fn reason(&self) -> &str {
        self.reason.as_str()
    }

    /// Return what this attempt produced before it was rejected, when anything
    /// survived it.
    pub fn artifact(&self) -> Option<&Path> {
        self.artifact.as_deref()
    }
}

/// Attempts already spent for one artifact together with why they failed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttemptLog {
    tally: AttemptTally,
    faults: Vec<AttemptFault>,
}

impl AttemptLog {
    /// Start one empty log bounded by `ceiling` attempts.
    pub fn new(ceiling: u8) -> Self {
        Self {
            tally: AttemptTally::new(ceiling),
            faults: Vec::new(),
        }
    }

    /// Return the attempt tally.
    pub fn tally(&self) -> AttemptTally {
        self.tally
    }

    /// Return every recorded fault in attempt order.
    pub fn faults(&self) -> &[AttemptFault] {
        self.faults.as_slice()
    }

    /// Return the fault of the most recently spent attempt.
    pub fn latest(&self) -> Option<&AttemptFault> {
        self.faults.last()
    }

    /// Return the log after one more spent attempt with no diagnosed cause.
    pub fn spent(self) -> Self {
        Self {
            tally: self.tally.spent(),
            faults: self.faults,
        }
    }

    /// Return the log after one more spent attempt that failed for `fault`.
    pub fn faulted(mut self, fault: AttemptFault) -> Self {
        self.faults.push(fault);
        Self {
            tally: self.tally.spent(),
            faults: self.faults,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn faulted_log_pairs_every_spent_attempt_with_its_reason() {
        let log = AttemptLog::new(3)
            .faulted(AttemptFault::new(
                "border",
                "White border missing on: bottom",
                None,
            ))
            .faulted(AttemptFault::failed("cache lock timeout"));
        assert_eq!(
            (
                log.tally().done(),
                log.faults().len(),
                log.latest().map(AttemptFault::category)
            ),
            (2, 2, Some("error")),
            "spent attempts lost the reasons they failed for"
        );
    }

    #[test]
    fn undiagnosed_attempt_still_spends_its_share_of_the_ceiling() {
        let log = AttemptLog::new(3).spent().spent().spent();
        assert!(
            log.tally().exhausted(),
            "undiagnosed attempts stopped counting against the retry ceiling"
        );
    }

    #[test]
    fn the_first_try_is_not_a_retry_and_the_rest_are_numbered_from_one() {
        let ceiling = AttemptLog::new(ARTIFACT_ATTEMPT_CEILING);
        let numbered = (0..ARTIFACT_ATTEMPT_CEILING)
            .scan(ceiling, |log, _| {
                let current = log.tally().retry();
                *log = log.clone().spent();
                Some(current)
            })
            .chain(std::iter::once(None))
            .collect::<Vec<_>>();
        assert_eq!(
            numbered,
            vec![None, Some(1), Some(2), Some(3), None],
            "the plain first try was numbered as a retry, or a retry lost its number"
        );
    }
}
