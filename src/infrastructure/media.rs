use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::Result;

use crate::application::media::{MediaSource, SceneSource};
use crate::domain::entry::NormalizedEntry;
use crate::domain::profile::{ProfileRegistry, profiles};
use crate::infrastructure::assets;
use crate::infrastructure::audio::{Audio, Speaker};
use crate::infrastructure::cache::Cache;
use crate::infrastructure::scene::{
    BorderDetector, Illustration, ImageSource, MangaRenderer, TextDetector, TextDetectors,
    Translator,
};

type CachedAudio<C> = Audio<C>;
type CachedIllustration<C> =
    Illustration<SceneTranslator<C>, MangaRenderer<TextDetectors<TextDetector>>>;

/// Wrap one scene client with one fixed prompt language.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SceneTranslator<C> {
    client: C,
    language: String,
}

impl<C> SceneTranslator<C> {
    /// Create one scene translator.
    pub fn new(client: C, language: impl Into<String>) -> Self {
        Self {
            client,
            language: language.into(),
        }
    }
}

impl<C> Translator for SceneTranslator<C>
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
    translator: BTreeMap<String, SceneTranslator<C>>,
}

/// Build lazy per-language audio and illustration services.
#[derive(Clone, Debug)]
pub struct Media<C> {
    cache: PathBuf,
    client: C,
    profiles: ProfileRegistry,
    state: State<C>,
}

impl<C> Media<C> {
    /// Create one media registry with the default profile registry.
    pub fn new(client: C, cache: impl Into<PathBuf>) -> Self {
        Self::configured(client, cache, profiles())
    }

    /// Create one media registry with an injected profile registry.
    pub fn configured(client: C, cache: impl Into<PathBuf>, profiles: ProfileRegistry) -> Self {
        Self {
            cache: cache.into(),
            client,
            profiles,
            state: State {
                audio: BTreeMap::new(),
                detector: BTreeMap::new(),
                illustration: BTreeMap::new(),
                translator: BTreeMap::new(),
            },
        }
    }
}

impl<C> Media<C>
where
    C: Clone + Speaker,
{
    /// Return the cached audio service for one entry target.
    pub fn audio(&mut self, entry: &NormalizedEntry) -> Result<CachedAudio<C>> {
        let code = entry.target_lang.clone();
        let item = self.profiles.item(code.as_str())?;
        let client = self.client.clone();
        let cache = self.cache.clone();
        if !self.state.audio.contains_key(code.as_str()) {
            self.state.audio.insert(
                code.clone(),
                Audio::new(
                    Cache::new(item.audio.cache.as_str(), cache),
                    assets::render_audio_prompt(item.audio.language.as_str()),
                    client,
                ),
            );
        }
        Ok(self
            .state
            .audio
            .get(code.as_str())
            .cloned()
            .expect("media must keep the requested audio service"))
    }
}

impl<C> Media<C>
where
    C: Clone + ImageSource + SceneSource + 'static,
{
    /// Return the cached illustration service for one entry target.
    pub fn illustration(&mut self, entry: &NormalizedEntry) -> Result<CachedIllustration<C>> {
        let code = entry.target_lang.clone();
        let item = self.profiles.item(code.as_str())?;
        let client = self.client.clone();
        let cache = self.cache.clone();
        let fallback = String::from(self.profiles.fallback_ocr());
        if !self.state.translator.contains_key(code.as_str()) {
            self.state.translator.insert(
                code.clone(),
                SceneTranslator::new(client.clone(), item.audio.language.as_str()),
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
                .expect("media must keep the requested detector");
            let translator = self
                .state
                .translator
                .get(code.as_str())
                .cloned()
                .expect("media must keep the requested translator");
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
            .expect("media must keep the requested illustration service"))
    }
}

impl<C> MediaSource for Media<C>
where
    C: Clone + ImageSource + SceneSource + Speaker + 'static,
{
    type Audio = CachedAudio<C>;
    type Illustration = CachedIllustration<C>;

    /// Return the audio service for one entry.
    fn audio(&mut self, entry: &NormalizedEntry) -> Result<Self::Audio> {
        Media::<C>::audio(self, entry)
    }

    /// Return the illustration service for one entry.
    fn illustration(&mut self, entry: &NormalizedEntry) -> Result<Self::Illustration> {
        Media::<C>::illustration(self, entry)
    }
}
