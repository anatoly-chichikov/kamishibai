use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;

use anyhow::Result;

use crate::application::media::{AudioSource, Profiles, SceneSource};
use crate::domain::entry::NormalizedEntry;
use crate::domain::profile::{ProfileRegistry, profiles};
use crate::infrastructure::assets;
use crate::infrastructure::audio::{Audio, Speaker};
use crate::infrastructure::cache::Cache;
use crate::infrastructure::scene::{
    BorderDetector, Illustration, ImageSource, MangaRenderer, TextDetector, TextDetectors,
    Translator,
};

type CachedAudio<C> = Audio<Cache, C>;
type CachedIllustration<C> =
    Illustration<Cache, SceneTranslator<C>, MangaRenderer<C, TextDetectors<TextDetector>>>;

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
pub struct Media<C, P> {
    cache: PathBuf,
    client: C,
    profiles: P,
    state: Rc<RefCell<State<C>>>,
}

impl<C> Media<C, ProfileRegistry> {
    /// Create one media registry with the default profile registry.
    pub fn new(client: C, cache: impl Into<PathBuf>) -> Self {
        Self::configured(client, cache, profiles())
    }
}

impl<C, P> Media<C, P> {
    /// Create one media registry with an injected profile registry.
    pub fn configured(client: C, cache: impl Into<PathBuf>, profiles: P) -> Self {
        Self {
            cache: cache.into(),
            client,
            profiles,
            state: Rc::new(RefCell::new(State {
                audio: BTreeMap::new(),
                detector: BTreeMap::new(),
                illustration: BTreeMap::new(),
                translator: BTreeMap::new(),
            })),
        }
    }
}

impl<C, P> Media<C, P>
where
    C: Clone + Speaker,
    P: Profiles,
{
    /// Return the cached audio service for one entry target.
    pub fn audio(&self, entry: &NormalizedEntry) -> Result<CachedAudio<C>> {
        let code = entry.target_lang.clone();
        let item = self.profiles.item(code.as_str())?;
        let mut state = self.state.borrow_mut();
        if !state.audio.contains_key(code.as_str()) {
            state.audio.insert(
                code.clone(),
                Audio::new(
                    Cache::new(item.audio().cache(), self.cache.clone()),
                    assets::render_audio_prompt(item.audio().language()),
                    self.client.clone(),
                ),
            );
        }
        Ok(state
            .audio
            .get(code.as_str())
            .cloned()
            .expect("media must keep the requested audio service"))
    }
}

impl<C, P> Media<C, P>
where
    C: Clone + ImageSource + SceneSource,
    P: Profiles,
{
    /// Return the cached illustration service for one entry target.
    pub fn illustration(&self, entry: &NormalizedEntry) -> Result<CachedIllustration<C>> {
        let code = entry.target_lang.clone();
        let item = self.profiles.item(code.as_str())?;
        let mut state = self.state.borrow_mut();
        if !state.translator.contains_key(code.as_str()) {
            state.translator.insert(
                code.clone(),
                SceneTranslator::new(self.client.clone(), item.audio().language()),
            );
        }
        if !state.detector.contains_key(code.as_str()) {
            state.detector.insert(
                code.clone(),
                TextDetector::cached(60, item.imagery().ocr(), self.cache.clone()),
            );
        }
        if !state.illustration.contains_key(code.as_str()) {
            let detector = state
                .detector
                .get(code.as_str())
                .cloned()
                .expect("media must keep the requested detector");
            let translator = state
                .translator
                .get(code.as_str())
                .cloned()
                .expect("media must keep the requested translator");
            let mut detectors = BTreeMap::new();
            detectors.insert(code.clone(), detector);
            state.illustration.insert(
                code.clone(),
                Illustration::new(
                    Cache::new(item.imagery().cache(), self.cache.clone()),
                    translator,
                    MangaRenderer::new(
                        self.client.clone(),
                        3,
                        TextDetectors::new(
                            detectors,
                            TextDetector::cached(60, self.profiles.fallback(), self.cache.clone()),
                        ),
                        BorderDetector::new(6, 240, 10),
                    ),
                ),
            );
        }
        Ok(state
            .illustration
            .get(code.as_str())
            .cloned()
            .expect("media must keep the requested illustration service"))
    }
}

impl<C, P> AudioSource for Media<C, P>
where
    C: Clone + Speaker,
    P: Profiles,
{
    type Audio = CachedAudio<C>;

    /// Return the audio service for one entry.
    fn audio(&self, entry: &NormalizedEntry) -> Result<Self::Audio> {
        Media::<C, P>::audio(self, entry)
    }
}

impl<C, P> crate::application::media::IllustrationSource for Media<C, P>
where
    C: Clone + ImageSource + SceneSource,
    P: Profiles,
{
    type Illustration = CachedIllustration<C>;

    /// Return the illustration service for one entry.
    fn illustration(&self, entry: &NormalizedEntry) -> Result<Self::Illustration> {
        Media::<C, P>::illustration(self, entry)
    }
}
