use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::Result;

use crate::generation::artifact_cache::Cache;
use crate::generation::manga::{
    BorderDetector, Illustration, ImageSource, MangaRenderer, TextDetector, TextDetectors,
    Translator,
};
use crate::generation::render_audio_prompt;
use crate::generation::speech::{Audio, Speaker};
use crate::generation::{GeneratorSource, SceneSource};
use crate::languages::{LanguageCatalog, catalog};
use crate::vocabulary::VocabularyEntry;

type CachedAudio<C> = Audio<C>;
type CachedIllustration<C> =
    Illustration<SceneComposer<C>, MangaRenderer<TextDetectors<TextDetector>>>;

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
struct State<C> {
    audio: BTreeMap<String, CachedAudio<C>>,
    detector: BTreeMap<String, TextDetector>,
    illustration: BTreeMap<String, CachedIllustration<C>>,
    translator: BTreeMap<String, SceneComposer<C>>,
}

/// Build lazy per-language audio and illustration services.
#[derive(Clone, Debug)]
pub struct GeneratorCatalog<C> {
    cache: PathBuf,
    client: C,
    catalog: LanguageCatalog,
    state: State<C>,
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
            state: State {
                audio: BTreeMap::new(),
                detector: BTreeMap::new(),
                illustration: BTreeMap::new(),
                translator: BTreeMap::new(),
            },
        }
    }
}

impl<C> GeneratorCatalog<C>
where
    C: Clone + Speaker,
{
    /// Return the cached audio service for one entry target.
    pub fn audio(&mut self, entry: &VocabularyEntry) -> Result<CachedAudio<C>> {
        let code = String::from(entry.target.lang.as_str());
        let item = self.catalog.item(code.as_str())?;
        let client = self.client.clone();
        let cache = self.cache.clone();
        if !self.state.audio.contains_key(code.as_str()) {
            self.state.audio.insert(
                code.clone(),
                Audio::new(
                    Cache::new(item.audio.cache.as_str(), cache),
                    render_audio_prompt(item.audio.language.as_str()),
                    client,
                ),
            );
        }
        Ok(self
            .state
            .audio
            .get(code.as_str())
            .cloned()
            .expect("generator catalog must keep the requested audio service"))
    }
}

impl<C> GeneratorCatalog<C>
where
    C: Clone + ImageSource + SceneSource + 'static,
{
    /// Return the cached illustration service for one entry target.
    pub fn illustration(&mut self, entry: &VocabularyEntry) -> Result<CachedIllustration<C>> {
        let code = String::from(entry.target.lang.as_str());
        let item = self.catalog.item(code.as_str())?;
        let client = self.client.clone();
        let cache = self.cache.clone();
        let fallback = String::from(self.catalog.fallback_ocr());
        if !self.state.translator.contains_key(code.as_str()) {
            self.state.translator.insert(
                code.clone(),
                SceneComposer::new(client.clone(), item.audio.language.as_str()),
            );
        }
        if !self.state.detector.contains_key(code.as_str()) {
            self.state.detector.insert(
                code.clone(),
                TextDetector::cached(60, item.imagery.ocr.as_str(), cache.clone()),
            );
        }
        if !self.state.illustration.contains_key(code.as_str()) {
            let detector = self
                .state
                .detector
                .get(code.as_str())
                .cloned()
                .expect("generator catalog must keep the requested detector");
            let translator = self
                .state
                .translator
                .get(code.as_str())
                .cloned()
                .expect("generator catalog must keep the requested translator");
            let mut detectors = BTreeMap::new();
            detectors.insert(code.clone(), detector);
            self.state.illustration.insert(
                code.clone(),
                Illustration::new(
                    Cache::new(item.imagery.cache.as_str(), cache.clone()),
                    translator,
                    MangaRenderer::new(
                        client,
                        3,
                        TextDetectors::new(detectors, TextDetector::cached(60, fallback, cache)),
                        BorderDetector::new(6, 240, 10),
                    ),
                ),
            );
        }
        Ok(self
            .state
            .illustration
            .get(code.as_str())
            .cloned()
            .expect("generator catalog must keep the requested illustration service"))
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
