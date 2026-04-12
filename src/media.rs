//! Media service registry and batch orchestration.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use anyhow::Result;
use serde_json::Value;

use crate::anki::{NoteFormat, VocabularyDeck};
use crate::assets;
use crate::audio::{Audio, Speaker};
use crate::cache::Cache;
use crate::gemini::{GeminiClient, Transport};
use crate::input::NormalizedEntry;
use crate::profile::{LanguageProfile, ProfileRegistry, profiles};
use crate::scene::{
    BorderDetector, Illustration, ImageSource, MangaRenderer, Progress as SceneProgress, Renderer,
    TextDetector, TextDetectors, Translator,
};

const IMAGE_STYLE: &str = "max-width: 100%; height: auto; border-radius: 10px";

type CachedAudio<C> = Audio<Cache, C>;
type CachedIllustration<C> =
    Illustration<Cache, SceneTranslator<C>, MangaRenderer<C, TextDetectors<TextDetector>>>;

/// Translate scenes with one prompt language bound to one client.
pub trait SceneSource {
    /// Return one translated scene JSON document.
    fn scene(&self, language: &str, sentence: &str, target: &str) -> Result<Value>;
}

impl<T> SceneSource for GeminiClient<T>
where
    T: Transport,
{
    /// Return one translated scene JSON document.
    fn scene(&self, language: &str, sentence: &str, target: &str) -> Result<Value> {
        GeminiClient::<T>::scene(self, language, sentence, target)
    }
}

impl<T> Speaker for GeminiClient<T>
where
    T: Transport,
{
    /// Return one PCM audio payload for the prompt and source text.
    fn speech(&self, prompt: &str, text: &str) -> Result<Vec<u8>> {
        GeminiClient::<T>::speech(self, prompt, text)
    }
}

/// Resolve language profiles for media services.
pub trait Profiles {
    /// Return one language profile for the target code.
    fn item(&self, code: &str) -> Result<LanguageProfile>;
    /// Return the fallback OCR selection.
    fn fallback(&self) -> &str;
}

impl Profiles for ProfileRegistry {
    /// Return one language profile for the target code.
    fn item(&self, code: &str) -> Result<LanguageProfile> {
        ProfileRegistry::item(self, code)
    }

    /// Return the fallback OCR selection.
    fn fallback(&self) -> &str {
        self.fallback_ocr()
    }
}

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
    fn translate(&self, sentence: &str, target: &str) -> Result<Value> {
        self.client.scene(self.language.as_str(), sentence, target)
    }
}

/// Expose one audio generator to the pipeline.
pub trait AudioService {
    /// Generate one cached audio filename and cache label.
    fn generate(&self, text: &str) -> Result<(String, bool)>;
    /// Return one absolute cached audio path.
    fn filepath(&self, filename: &str) -> Result<PathBuf>;
}

impl<C, S> AudioService for Audio<C, S>
where
    C: crate::cache::FileCache,
    S: Speaker,
{
    /// Generate one cached audio filename and cache label.
    fn generate(&self, text: &str) -> Result<(String, bool)> {
        Audio::<C, S>::generate(self, text)
    }

    /// Return one absolute cached audio path.
    fn filepath(&self, filename: &str) -> Result<PathBuf> {
        Audio::<C, S>::filepath(self, filename)
    }
}

/// Expose one illustration generator to the pipeline.
pub trait IllustrationService {
    /// Generate one cached illustration filename and cache label.
    fn generate(
        &self,
        sentence: &str,
        word: &str,
        target: &str,
        progress: &mut dyn SceneProgress,
    ) -> Result<(String, bool)>;
    /// Return one absolute cached illustration path.
    fn filepath(&self, filename: &str) -> Result<PathBuf>;
}

impl<C, T, R> IllustrationService for Illustration<C, T, R>
where
    C: crate::cache::FileCache,
    T: Translator,
    R: Renderer,
{
    /// Generate one cached illustration filename and cache label.
    fn generate(
        &self,
        sentence: &str,
        word: &str,
        target: &str,
        progress: &mut dyn SceneProgress,
    ) -> Result<(String, bool)> {
        Illustration::<C, T, R>::generate(self, sentence, word, target, progress)
    }

    /// Return one absolute cached illustration path.
    fn filepath(&self, filename: &str) -> Result<PathBuf> {
        Illustration::<C, T, R>::filepath(self, filename)
    }
}

/// Resolve one audio generator for one entry.
pub trait AudioSource {
    /// The returned audio service type.
    type Audio: AudioService;
    /// Return the audio service for one entry.
    fn audio(&self, entry: &NormalizedEntry) -> Result<Self::Audio>;
}

impl<S> AudioSource for S
where
    S: AudioService + Clone,
{
    type Audio = S;

    /// Return the audio service for one entry.
    fn audio(&self, _entry: &NormalizedEntry) -> Result<Self::Audio> {
        Ok(self.clone())
    }
}

/// Resolve one illustration generator for one entry.
pub trait IllustrationSource {
    /// The returned illustration service type.
    type Illustration: IllustrationService;
    /// Return the illustration service for one entry.
    fn illustration(&self, entry: &NormalizedEntry) -> Result<Self::Illustration>;
}

impl<S> IllustrationSource for S
where
    S: IllustrationService + Clone,
{
    type Illustration = S;

    /// Return the illustration service for one entry.
    fn illustration(&self, _entry: &NormalizedEntry) -> Result<Self::Illustration> {
        Ok(self.clone())
    }
}

/// Record pipeline-level progress events and failures.
pub trait PipelineProgress: SceneProgress {
    /// Signal the card position within the batch.
    fn card(&mut self, index: usize, total: usize, word: &str);
    /// Signal one skipped entry.
    fn skip(&mut self, word: &str, reason: &str);
}

/// Attach generated media and add one note.
pub trait Deck {
    /// Attach one media file path.
    fn attach(&mut self, path: &Path);
    /// Add one note to the deck.
    fn add(&mut self, entry: &NormalizedEntry, audio: &str, image: &str);
}

impl<F> Deck for VocabularyDeck<F>
where
    F: NoteFormat,
{
    /// Attach one media file path.
    fn attach(&mut self, path: &Path) {
        VocabularyDeck::<F>::attach(self, path.to_path_buf());
    }

    /// Add one note to the deck.
    fn add(&mut self, entry: &NormalizedEntry, audio: &str, image: &str) {
        VocabularyDeck::<F>::add(self, entry, audio, image);
    }
}

/// Record one skipped entry and its failure reason.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Failure {
    /// The skipped entry word.
    pub word: String,
    /// The captured failure reason.
    pub reason: String,
}

impl Failure {
    /// Create one recorded pipeline failure.
    pub fn new(word: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            word: word.into(),
            reason: reason.into(),
        }
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
            state
                .detector
                .insert(code.clone(), TextDetector::custom(60, item.imagery().ocr()));
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
                            TextDetector::custom(60, self.profiles.fallback()),
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

impl<C, P> IllustrationSource for Media<C, P>
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

/// Orchestrate audio, illustration, deck, and progress for one batch.
#[derive(Clone, Debug)]
pub struct Pipeline<A, I, D, P> {
    audio: A,
    illustration: I,
    deck: D,
    progress: P,
}

impl<A, I, D, P> Pipeline<A, I, D, P> {
    /// Create one media pipeline.
    pub fn new(audio: A, illustration: I, deck: D, progress: P) -> Self {
        Self {
            audio,
            illustration,
            deck,
            progress,
        }
    }

    /// Return the accumulated deck.
    pub fn deck(&self) -> &D {
        &self.deck
    }

    /// Return the accumulated progress recorder.
    pub fn progress(&self) -> &P {
        &self.progress
    }
}

impl<A, I, D, P> Pipeline<A, I, D, P>
where
    A: AudioSource,
    I: IllustrationSource,
    D: Deck,
    P: PipelineProgress,
{
    /// Process one batch and return failures plus successful image payloads.
    pub fn process(
        &mut self,
        entries: &[NormalizedEntry],
    ) -> (Vec<Failure>, Vec<(NormalizedEntry, PathBuf)>) {
        let mut failed = Vec::new();
        let mut processed = Vec::new();
        for (index, entry) in entries.iter().enumerate() {
            self.progress
                .card(index + 1, entries.len(), entry.word.as_str());
            let audio = match self.audio.audio(entry) {
                Ok(item) => item,
                Err(error) => {
                    let reason = error.to_string();
                    self.progress.skip(entry.word.as_str(), reason.as_str());
                    failed.push(Failure::new(entry.word.clone(), reason));
                    continue;
                }
            };
            let (audiofile, audiopath) = match self.audio(entry, &audio) {
                Ok(item) => item,
                Err(error) => {
                    let reason = error.to_string();
                    self.progress.skip(entry.word.as_str(), reason.as_str());
                    failed.push(Failure::new(entry.word.clone(), reason));
                    continue;
                }
            };
            let illustration = match self.illustration.illustration(entry) {
                Ok(item) => item,
                Err(error) => {
                    let reason = error.to_string();
                    self.progress.skip(entry.word.as_str(), reason.as_str());
                    failed.push(Failure::new(entry.word.clone(), reason));
                    continue;
                }
            };
            let (imagefile, imagepath) = match self.image(entry, &illustration) {
                Ok(item) => item,
                Err(error) => {
                    let reason = error.to_string();
                    self.progress.skip(entry.word.as_str(), reason.as_str());
                    failed.push(Failure::new(entry.word.clone(), reason));
                    continue;
                }
            };
            self.deck.attach(audiopath.as_path());
            self.deck.attach(imagepath.as_path());
            self.deck.add(
                entry,
                format!("[sound:{audiofile}]").as_str(),
                format!("<img src='{imagefile}' style='{IMAGE_STYLE}'>").as_str(),
            );
            processed.push((entry.clone(), imagepath));
        }
        (failed, processed)
    }

    /// Generate audio and return the filename plus absolute path.
    fn audio(&mut self, entry: &NormalizedEntry, audio: &A::Audio) -> Result<(String, PathBuf)> {
        self.progress.step("Generating audio");
        let (filename, cached) = audio.generate(entry.example.as_str())?;
        let path = audio.filepath(filename.as_str())?;
        self.progress.done(
            "Generating audio",
            if cached { "cached" } else { "generated" },
            Some(path.as_path()),
        );
        Ok((filename, path))
    }

    /// Generate one illustration and return the filename plus absolute path.
    fn image(
        &mut self,
        entry: &NormalizedEntry,
        illustration: &I::Illustration,
    ) -> Result<(String, PathBuf)> {
        let (filename, _cached) = illustration.generate(
            entry.example.as_str(),
            entry.word.as_str(),
            entry.target_lang.as_str(),
            &mut self.progress,
        )?;
        Ok((filename.clone(), illustration.filepath(filename.as_str())?))
    }
}
