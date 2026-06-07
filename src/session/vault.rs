//! Per-card cache folder grouping every artifact that belongs to one card.

use std::path::PathBuf;

use crate::generation::artifact_cache::Cache;

use super::pair::LanguagePair;

const CARD_VERSION: &str = "v3";

/// The single cache folder that holds every artifact for one card.
///
/// `meta.json`, `scene.json`, `voice.wav`, and `illustration.jpg` for one card
/// live together under `cards/<support>-<target>/<key>`, where `key` is a short
/// digest of the card identity (language pair, term, understanding). Browsing
/// the cache root therefore shows one folder per card instead of the older
/// artifact-type-major directories. Use [`CardCell::cache`] to read or write the
/// artifacts and [`CardCell::media_name`] to derive a package-unique media name.
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
            pair.target(),
            pair.support(),
            term,
            understanding,
        ]);
        let folder = format!("cards/{}-{}/{key}", pair.support(), pair.target());
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
    /// Disk files are role-named (`voice.wav`), but Anki keys media by basename,
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
}
