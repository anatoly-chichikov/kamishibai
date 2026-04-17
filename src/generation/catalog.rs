use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::Result;

use crate::generation::artifact_cache::Cache;
use crate::generation::manga::{
    BorderDetector, Illustration, ImageSource, MangaRenderer, TextDetector, Translator,
};
use crate::generation::render_audio_prompt;
use crate::generation::speech::{Audio, Speaker};
use crate::generation::{GeneratorSource, SceneSource};
use crate::languages::{LanguageCatalog, catalog};
use crate::vocabulary::VocabularyEntry;

type CachedAudio<C> = Audio<C>;
type CachedIllustration<C> = Illustration<SceneComposer<C>, MangaRenderer<TextDetector>>;

/// Wrap one scene client with one fixed prompt language.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SceneComposer<C> {
    client: C,
    language: String,
}

impl<C> SceneComposer<C> {
    /// Create one scene translator.
    pub fn new(client: C, language: impl Into<String>) -> Self {
        Self {
            client,
            language: language.into(),
        }
    }
}

impl<C> Translator for SceneComposer<C>
where
    C: SceneSource,
{
    /// Return one translated scene JSON document.
    fn translate(&self, sentence: &str, target: &str) -> Result<serde_json::Value> {
        self.client.scene(self.language.as_str(), sentence, target)
    }
}

#[derive(Clone, Debug)]
struct LanguageRuntime<C> {
    audio: CachedAudio<C>,
    illustration: CachedIllustration<C>,
}

/// Build lazy per-language audio and illustration services.
#[derive(Clone, Debug)]
pub struct GeneratorCatalog<C> {
    cache: PathBuf,
    client: C,
    catalog: LanguageCatalog,
    runtimes: BTreeMap<String, LanguageRuntime<C>>,
}

impl<C> GeneratorCatalog<C> {
    /// Create one generator catalog with the default language catalog.
    pub fn new(client: C, cache: impl Into<PathBuf>) -> Self {
        Self::configured(client, cache, catalog())
    }

    /// Create one generator catalog with an injected language catalog.
    pub fn configured(client: C, cache: impl Into<PathBuf>, catalog: LanguageCatalog) -> Self {
        Self {
            cache: cache.into(),
            client,
            catalog,
            runtimes: BTreeMap::new(),
        }
    }
}

impl<C> GeneratorCatalog<C>
where
    C: Clone + ImageSource + SceneSource + Speaker + 'static,
{
    /// Return the cached runtime for one entry target.
    fn runtime(&mut self, entry: &VocabularyEntry) -> Result<&LanguageRuntime<C>> {
        let code = String::from(entry.target.lang.as_str());
        if !self.runtimes.contains_key(code.as_str()) {
            let item = self.catalog.item(code.as_str())?;
            let client = self.client.clone();
            let cache = self.cache.clone();
            self.runtimes.insert(
                code.clone(),
                LanguageRuntime {
                    audio: Audio::new(
                        Cache::new(item.audio_cache.as_str(), cache.clone()),
                        render_audio_prompt(item.prompt.as_str()),
                        client.clone(),
                    ),
                    illustration: Illustration::new(
                        Cache::new(item.image_cache.as_str(), cache.clone()),
                        SceneComposer::new(client.clone(), item.prompt.as_str()),
                        MangaRenderer::new(
                            client,
                            3,
                            TextDetector::cached(60, item.ocr.as_str(), cache),
                            BorderDetector::new(6, 240, 10),
                        ),
                    ),
                },
            );
        }
        Ok(self
            .runtimes
            .get(code.as_str())
            .expect("generator catalog must keep the requested runtime"))
    }

    /// Return the cached audio service for one entry target.
    pub fn audio(&mut self, entry: &VocabularyEntry) -> Result<CachedAudio<C>> {
        Ok(self.runtime(entry)?.audio.clone())
    }

    /// Return the cached illustration service for one entry target.
    pub fn illustration(&mut self, entry: &VocabularyEntry) -> Result<CachedIllustration<C>> {
        Ok(self.runtime(entry)?.illustration.clone())
    }
}

impl<C> GeneratorSource for GeneratorCatalog<C>
where
    C: Clone + ImageSource + SceneSource + Speaker + 'static,
{
    type Audio = CachedAudio<C>;
    type Illustration = CachedIllustration<C>;

    /// Return the audio service for one entry.
    fn audio(&mut self, entry: &VocabularyEntry) -> Result<Self::Audio> {
        GeneratorCatalog::<C>::audio(self, entry)
    }

    /// Return the illustration service for one entry.
    fn illustration(&mut self, entry: &VocabularyEntry) -> Result<Self::Illustration> {
        GeneratorCatalog::<C>::illustration(self, entry)
    }
}
