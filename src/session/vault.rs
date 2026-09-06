//! Per-card cache folder grouping every artifact that belongs to one card.

use std::path::PathBuf;

use crate::generation::artifact_cache::Cache;

use super::{CardDraft, Sense, pair::LanguagePair};

const CARD_VERSION: &str = "v3";

/// The single cache folder that holds every artifact for one card.
///
/// `meta.json` and `audio.wav` live directly under
/// `cards/<support>-<target>/<key>`; each visual policy stores `scene.json` and
/// `picture.jpg` below `visual/<revision>`. The key is a short digest of the card
/// identity (language pair, term, understanding). Use [`CardCell::cache`] to
/// read or write the artifacts and [`CardCell::media_name`] to derive a
/// package-unique media name.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CardCell {
    key: String,
    cache: Cache,
}

impl CardCell {
    /// Resolve the cache folder for one card under the shared cache root.
    pub fn new(
        root: impl Into<PathBuf>,
        pair: &LanguagePair,
        term: &str,
        understanding: &str,
    ) -> Self {
        let key = digest(&[
            CARD_VERSION,
            pair.learning(),
            pair.known(),
            term,
            understanding,
        ]);
        Self::identified(root, pair, key)
    }

    /// Resolve the cache folder for one reviewed draft, including every sense
    /// that can change its generated explanation.
    pub fn for_draft(root: impl Into<PathBuf>, draft: &CardDraft) -> Self {
        Self::with_reviewed_senses(
            root,
            draft.pair(),
            draft.term(),
            draft.understanding(),
            draft.reviewed_senses(),
        )
    }

    /// Resolve one card cache from its complete ordered reviewed-sense context.
    /// Empty and untagged singleton contexts retain the legacy identity.
    pub fn with_reviewed_senses(
        root: impl Into<PathBuf>,
        pair: &LanguagePair,
        term: &str,
        understanding: &str,
        reviewed_senses: &[Sense],
    ) -> Self {
        if reviewed_senses.is_empty()
            || reviewed_senses.len() == 1 && reviewed_senses[0].tag().is_none()
        {
            return Self::new(root, pair, term, understanding);
        }
        let mut identity = vec![
            String::from(CARD_VERSION),
            pair.learning().to_string(),
            pair.known().to_string(),
            term.to_string(),
            understanding.to_string(),
            String::from("reviewed-senses-v1"),
            reviewed_senses.len().to_string(),
        ];
        for sense in reviewed_senses {
            identity.push(sense.understanding().to_string());
            identity.push(sense.tag().unwrap_or_default().to_string());
        }
        let parts = identity.iter().map(String::as_str).collect::<Vec<_>>();
        Self::identified(root, pair, digest(parts.as_slice()))
    }

    fn identified(root: impl Into<PathBuf>, pair: &LanguagePair, key: String) -> Self {
        let folder = format!("cards/{}-{}/{key}", pair.known(), pair.learning());
        Self {
            cache: Cache::new(folder, root),
            key,
        }
    }

    /// Return the persistent cache handle rooted at this card's folder.
    pub fn cache(&self) -> Cache {
        self.cache.clone()
    }

    /// Return the package-unique media name this card contributes to a deck.
    ///
    /// Disk files are role-named (`audio.wav`), but Anki keys media by basename,
    /// so each card renames its files to `<key>.<extension>` inside the package.
    pub fn media_name(&self, extension: &str) -> String {
        format!("{}.{extension}", self.key)
    }
}

/// Return the first twelve hex chars of the MD5 of the parts joined by NUL.
pub(crate) fn digest(parts: &[&str]) -> String {
    let full = format!("{:x}", md5::compute(parts.join("\0").as_bytes()));
    full[..12].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn different_understanding_lands_in_a_different_folder() {
        let root = PathBuf::from("/tmp/kamishibai-cell-test");
        let pair = LanguagePair::new("fr", "en");
        let duck = CardCell::new(root.clone(), &pair, "canard", "a duck");
        let hoax = CardCell::new(root, &pair, "canard", "a false news report");
        assert_ne!(
            duck.cache().path(),
            hoax.cache().path(),
            "two senses of one term must not share a card folder"
        );
    }

    #[test]
    fn model_refresh_preserves_the_existing_card_identity() {
        let cell = CardCell::new(
            "/tmp/kamishibai-cell-version-test",
            &LanguagePair::new("fr", "en"),
            "canard",
            "a duck",
        );
        assert_eq!(
            (
                cell.media_name("jpg"),
                CardCell::for_draft(
                    "/tmp/kamishibai-cell-version-test",
                    &CardDraft::new("canard", "a duck", LanguagePair::new("fr", "en")),
                )
                .media_name("jpg"),
            ),
            (
                String::from("3dfcb9807a67.jpg"),
                String::from("3dfcb9807a67.jpg"),
            ),
            "model refreshes must not orphan artifacts from existing published sessions"
        );
    }

    #[test]
    fn different_reviewed_alternatives_land_in_different_folders() {
        let root = PathBuf::from("/tmp/kamishibai-reviewed-senses-test");
        let pair = LanguagePair::new("fr", "en");
        let bird = vec![Sense::plain("a duck"), Sense::plain("a false report")];
        let newspaper = vec![Sense::plain("a duck"), Sense::plain("a newspaper hoax")];
        assert_ne!(
            CardCell::with_reviewed_senses(root.clone(), &pair, "canard", "a duck", &bird)
                .media_name("jpg"),
            CardCell::with_reviewed_senses(root, &pair, "canard", "a duck", &newspaper)
                .media_name("jpg"),
            "different reviewed alternatives unexpectedly shared one media identity"
        );
    }

    #[test]
    fn singleton_tag_participates_in_identity_while_plain_singleton_stays_legacy() {
        let root = PathBuf::from("/tmp/kamishibai-singleton-tag-test");
        let pair = LanguagePair::new("fr", "en");
        let legacy = CardCell::new(root.clone(), &pair, "canard", "a false report");
        let plain = CardCell::with_reviewed_senses(
            root.clone(),
            &pair,
            "canard",
            "a false report",
            &[Sense::plain("a false report")],
        );
        let tagged = CardCell::with_reviewed_senses(
            root.clone(),
            &pair,
            "canard",
            "a false report",
            &[Sense::tagged("a false report", "journalism")],
        );
        let regional = CardCell::with_reviewed_senses(
            root,
            &pair,
            "canard",
            "a false report",
            &[Sense::tagged("a false report", "regional")],
        );
        assert!(
            plain.media_name("jpg") == legacy.media_name("jpg")
                && tagged.media_name("jpg") != legacy.media_name("jpg")
                && regional.media_name("jpg") != tagged.media_name("jpg"),
            "a singleton tag was ignored in media identity or an untagged singleton abandoned legacy media"
        );
    }
}
