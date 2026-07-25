//! Keeps the durable attempt archive readable from both sides: it reads back the
//! verdict of a rejected picture, and it writes down the model reply a rejected
//! scene was decoded from, so every failed attempt can point at what it threw
//! away.

use std::fs;
use std::path::PathBuf;

use serde_json::Value;

use crate::gemini::RejectedReply;
use crate::generation::artifact_cache::{Cache, IMAGE_ATTEMPTS_DIRECTORY};
use crate::session::{ArtifactAttempt, ArtifactFile, AttemptFault};

/// One archived verdict together with the picture it judged.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ArchivedVerdict {
    sequence: usize,
    fault: AttemptFault,
}

impl ArchivedVerdict {
    /// Return the archive sequence this verdict was written for.
    pub(super) fn sequence(&self) -> usize {
        self.sequence
    }

    /// Return the verdict as the fault of one spent attempt.
    pub(super) fn fault(self) -> AttemptFault {
        self.fault
    }
}

/// Return the highest archived attempt sequence for one visual revision.
pub(super) fn archived_sequence(cache: &Cache) -> usize {
    verdicts(cache)
        .into_iter()
        .map(|(sequence, _)| sequence)
        .max()
        .unwrap_or(0)
}

/// Return the newest archived verdict that rejected or failed its picture.
pub(super) fn latest_verdict(cache: &Cache) -> Option<ArchivedVerdict> {
    let (sequence, path) = verdicts(cache)
        .into_iter()
        .max_by_key(|(sequence, _)| *sequence)?;
    let value = fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())?;
    let status = member(&value, "status")?;
    if status == "accepted" {
        return None;
    }
    let directory = path.parent()?.to_path_buf();
    Some(ArchivedVerdict {
        sequence,
        fault: AttemptFault::new(
            member(&value, "category").unwrap_or_else(|| String::from("other")),
            member(&value, "reason").unwrap_or_else(|| String::from("attempt was rejected")),
            member(&value, "image").map(|name| directory.join(name)),
        ),
    })
}

fn verdicts(cache: &Cache) -> Vec<(usize, PathBuf)> {
    let directory = cache.path().join(IMAGE_ATTEMPTS_DIRECTORY);
    let Ok(entries) = fs::read_dir(directory) else {
        return Vec::new();
    };
    entries
        .filter_map(std::result::Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let sequence = name
                .strip_prefix("attempt-")?
                .strip_suffix(".json")?
                .parse::<usize>()
                .ok()?;
            Some((sequence, entry.path()))
        })
        .collect()
}

fn member(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(String::from)
}
/// Archive the model reply behind one rejected scene and attach it to the
/// attempt.
///
/// The reply is stored under its own name so it never collides with the image
/// journal, and it keeps the shape it arrived in: `.json` when it decodes as
/// JSON that simply failed later validation, `.txt` when it never was JSON at
/// all. A failure that carries no reply (transport, cache lease) is left alone.
pub(super) fn archived_reply(
    attempt: ArtifactAttempt<ArtifactFile>,
    cache: &Cache,
) -> ArtifactAttempt<ArtifactFile> {
    let Some(error) = attempt.error() else {
        return attempt;
    };
    let reason = format!("{error:#}");
    let Some(reply) = error.downcast_ref::<RejectedReply>() else {
        return attempt;
    };
    match store_reply(cache, reply.body()) {
        Ok(path) => attempt.with_fault(AttemptFault::new("error", reason, Some(path))),
        Err(_) => attempt,
    }
}

fn store_reply(cache: &Cache, body: &str) -> std::io::Result<PathBuf> {
    let directory = cache.path().join(IMAGE_ATTEMPTS_DIRECTORY);
    fs::create_dir_all(&directory)?;
    let sequence = reply_sequence(&directory).saturating_add(1);
    let extension = if serde_json::from_str::<Value>(body.trim()).is_ok() {
        "json"
    } else {
        "txt"
    };
    let path = directory.join(format!("scene-{sequence:04}.{extension}"));
    fs::write(&path, body)?;
    Ok(path)
}

fn reply_sequence(directory: &std::path::Path) -> usize {
    let Ok(entries) = fs::read_dir(directory) else {
        return 0;
    };
    entries
        .filter_map(std::result::Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            name.strip_prefix("scene-")?
                .split('.')
                .next()?
                .parse::<usize>()
                .ok()
        })
        .max()
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    fn archive(cache: &Cache, sequence: usize, verdict: Value) {
        let directory = cache
            .filepath(IMAGE_ATTEMPTS_DIRECTORY)
            .expect("attempt archive path must resolve");
        fs::create_dir_all(&directory).expect("attempt archive must be creatable");
        fs::write(
            directory.join(format!("attempt-{sequence:04}.json")),
            serde_json::to_vec_pretty(&verdict).expect("verdict must serialize"),
        )
        .expect("verdict must be writable");
    }

    #[test]
    fn newest_rejection_names_its_category_and_archived_picture() {
        let root = tempdir().expect("temporary cache root must be creatable");
        let cache = Cache::new("card", root.path());
        archive(
            &cache,
            1,
            json!({"status": "rejected", "category": "border", "reason": "White border missing on: bottom", "image": "attempt-0001.jpg"}),
        );
        let fault = latest_verdict(&cache)
            .expect("archived rejection must be readable")
            .fault();
        assert_eq!(
            (
                fault.category(),
                fault.reason(),
                fault
                    .artifact()
                    .map(|path| path.file_name().and_then(|name| name.to_str())
                        == Some("attempt-0001.jpg")),
            ),
            ("border", "White border missing on: bottom", Some(true)),
            "archived rejection lost the category, reason, or picture the user needs"
        );
    }

    #[test]
    fn accepted_verdict_is_not_reported_as_a_fault() {
        let root = tempdir().expect("temporary cache root must be creatable");
        let cache = Cache::new("card", root.path());
        archive(
            &cache,
            1,
            json!({"status": "accepted", "category": "accepted", "reason": "", "image": "attempt-0001.jpg"}),
        );
        assert!(
            latest_verdict(&cache).is_none(),
            "an accepted picture was reported as a failed attempt"
        );
    }
}
