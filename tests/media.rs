//! Tests for media registry wiring and pipeline orchestration.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use kamishibai::input::NormalizedEntry;
use kamishibai::media::{
    AudioService, AudioSource, Deck, Failure, IllustrationService, IllustrationSource, Media,
    Pipeline, PipelineProgress, Profiles, SceneSource,
};
use kamishibai::profile::{
    AudioProfile, DeckNaming, FontProfile, ImageProfile, LanguageProfile, UiLabels,
};
use kamishibai::scene::{ImageSource, Progress as SceneProgress};
use serde_json::{Value, json};
use tempfile::TempDir;

/// Fake client for media registry tests.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct FakeClient;

impl AudioService for AudioFixture {
    /// Generate one cached audio filename and cache label.
    fn generate(&self, text: &str) -> Result<(String, bool)> {
        match self {
            Self::Success(item) => item.generate(text),
            Self::Failure(item) => item.generate(text),
        }
    }

    /// Return one absolute cached audio path.
    fn filepath(&self, filename: &str) -> Result<PathBuf> {
        match self {
            Self::Success(item) => item.filepath(filename),
            Self::Failure(item) => item.filepath(filename),
        }
    }
}

impl IllustrationService for IllustrationFixture {
    /// Generate one cached illustration filename and cache label.
    fn generate(
        &self,
        sentence: &str,
        word: &str,
        target: &str,
        progress: &mut dyn SceneProgress,
    ) -> Result<(String, bool)> {
        match self {
            Self::Success(item) => item.generate(sentence, word, target, progress),
            Self::Failure(item) => item.generate(sentence, word, target, progress),
        }
    }

    /// Return one absolute cached illustration path.
    fn filepath(&self, filename: &str) -> Result<PathBuf> {
        match self {
            Self::Success(item) => item.filepath(filename),
            Self::Failure(item) => item.filepath(filename),
        }
    }
}

impl SceneSource for FakeClient {
    /// Return one translated scene JSON document.
    fn scene(&self, language: &str, sentence: &str, target: &str) -> Result<Value> {
        Ok(json!({
            "manga_panel": {
                "meta": {
                    "target_lang": target,
                    "title": sentence,
                    "description": language,
                },
                "panels": [{"scene": {"description": sentence}}],
            },
        }))
    }
}

impl kamishibai::audio::Speaker for FakeClient {
    /// Return one PCM audio payload for the prompt and source text.
    fn speech(&self, _prompt: &str, _text: &str) -> Result<Vec<u8>> {
        Ok(vec![0, 0, 1, 0])
    }
}

impl ImageSource for FakeClient {
    /// Return one encoded image payload for the scene and word.
    fn image(&self, _scene: &Value, _word: &str) -> Result<Vec<u8>> {
        Ok(include_bytes!("../tests/fixtures/reference/inputs/single-target-en.json").to_vec())
    }
}

/// Fake profile registry for media registry tests.
#[derive(Clone, Debug, Eq, PartialEq)]
struct FakeProfiles {
    fallback: String,
    item: LanguageProfile,
}

impl FakeProfiles {
    /// Create one fake profile registry.
    fn new(code: &str, fallback: &str) -> Self {
        Self {
            fallback: String::from(fallback),
            item: LanguageProfile::new(
                code,
                AudioProfile::new("Irish", "audio-ga"),
                ImageProfile::new("eng+gle", "manga-ga"),
                DeckNaming::new("Irish Vocabulary", "ga", "kamishibai.json"),
                FontProfile::new("DejaVu Sans"),
                UiLabels::new("Translation", "Context", "Hint", "Importance"),
            ),
        }
    }
}

impl Profiles for FakeProfiles {
    /// Return one language profile for the target code.
    fn item(&self, code: &str) -> Result<LanguageProfile> {
        if code == self.item.code() {
            return Ok(self.item.clone());
        }
        Err(anyhow!("Unsupported target language '{code}'"))
    }

    /// Return the fallback OCR selection.
    fn fallback(&self) -> &str {
        self.fallback.as_str()
    }
}

/// Return one normalized entry for media tests.
fn entry(word: &str, example: &str, target: &str) -> NormalizedEntry {
    NormalizedEntry {
        word: String::from(word),
        pronunciation: String::new(),
        translation: String::from("значение"),
        example: String::from(example),
        source_lang: String::from("ru"),
        target_lang: String::from(target),
        sentence: String::from("пример"),
        highlight: String::new(),
        hint: String::new(),
        context: String::new(),
        importance: String::new(),
        transcription: String::new(),
    }
}

/// Successful audio service fixture.
#[derive(Clone, Debug, Eq, PartialEq)]
struct SuccessAudio {
    cached: bool,
    filename: String,
    root: PathBuf,
}

impl SuccessAudio {
    /// Create one successful audio fixture.
    fn new(root: &Path, filename: &str, cached: bool) -> Self {
        Self {
            cached,
            filename: String::from(filename),
            root: root.to_path_buf(),
        }
    }
}

impl AudioService for SuccessAudio {
    /// Generate one cached audio filename and cache label.
    fn generate(&self, text: &str) -> Result<(String, bool)> {
        if text.trim().is_empty() {
            return Err(anyhow!("Cannot generate audio for empty text"));
        }
        Ok((self.filename.clone(), self.cached))
    }

    /// Return one absolute cached audio path.
    fn filepath(&self, filename: &str) -> Result<PathBuf> {
        Ok(self.root.join(filename))
    }
}

/// Failing audio service fixture.
#[derive(Clone, Debug, Eq, PartialEq)]
struct FailingAudio {
    reason: String,
    root: PathBuf,
}

impl FailingAudio {
    /// Create one failing audio fixture.
    fn new(root: &Path, reason: &str) -> Self {
        Self {
            reason: String::from(reason),
            root: root.to_path_buf(),
        }
    }
}

impl AudioService for FailingAudio {
    /// Generate one cached audio filename and cache label.
    fn generate(&self, _text: &str) -> Result<(String, bool)> {
        Err(anyhow!(self.reason.clone()))
    }

    /// Return one absolute cached audio path.
    fn filepath(&self, filename: &str) -> Result<PathBuf> {
        Ok(self.root.join(filename))
    }
}

/// Audio fixture enum for switching providers.
#[derive(Clone, Debug, Eq, PartialEq)]
enum AudioFixture {
    Failure(FailingAudio),
    Success(SuccessAudio),
}

/// Successful illustration service fixture.
#[derive(Clone, Debug, Eq, PartialEq)]
struct SuccessIllustration {
    cached: bool,
    filename: String,
    root: PathBuf,
}

impl SuccessIllustration {
    /// Create one successful illustration fixture.
    fn new(root: &Path, filename: &str, cached: bool) -> Self {
        Self {
            cached,
            filename: String::from(filename),
            root: root.to_path_buf(),
        }
    }
}

impl IllustrationService for SuccessIllustration {
    /// Generate one cached illustration filename and cache label.
    fn generate(
        &self,
        _sentence: &str,
        _word: &str,
        _target: &str,
        _progress: &mut dyn SceneProgress,
    ) -> Result<(String, bool)> {
        Ok((self.filename.clone(), self.cached))
    }

    /// Return one absolute cached illustration path.
    fn filepath(&self, filename: &str) -> Result<PathBuf> {
        Ok(self.root.join(filename))
    }
}

/// Failing illustration service fixture.
#[derive(Clone, Debug, Eq, PartialEq)]
struct FailingIllustration {
    reason: String,
    root: PathBuf,
}

impl FailingIllustration {
    /// Create one failing illustration fixture.
    fn new(root: &Path, reason: &str) -> Self {
        Self {
            reason: String::from(reason),
            root: root.to_path_buf(),
        }
    }
}

impl IllustrationService for FailingIllustration {
    /// Generate one cached illustration filename and cache label.
    fn generate(
        &self,
        _sentence: &str,
        _word: &str,
        _target: &str,
        _progress: &mut dyn SceneProgress,
    ) -> Result<(String, bool)> {
        Err(anyhow!(self.reason.clone()))
    }

    /// Return one absolute cached illustration path.
    fn filepath(&self, filename: &str) -> Result<PathBuf> {
        Ok(self.root.join(filename))
    }
}

/// Illustration fixture enum for switching providers.
#[derive(Clone, Debug, Eq, PartialEq)]
enum IllustrationFixture {
    Failure(FailingIllustration),
    Success(SuccessIllustration),
}

/// Audio provider that switches by entry word.
#[derive(Clone, Debug, Eq, PartialEq)]
struct SwitchingAudio {
    items: BTreeMap<String, AudioFixture>,
}

impl AudioSource for SwitchingAudio {
    type Audio = AudioFixture;

    /// Return the audio service for one entry.
    fn audio(&self, entry: &NormalizedEntry) -> Result<Self::Audio> {
        self.items
            .get(entry.word.as_str())
            .cloned()
            .ok_or_else(|| anyhow!("Missing audio fixture for '{}'", entry.word))
    }
}

/// Illustration provider that switches by entry word.
#[derive(Clone, Debug, Eq, PartialEq)]
struct SwitchingIllustration {
    items: BTreeMap<String, IllustrationFixture>,
}

impl IllustrationSource for SwitchingIllustration {
    type Illustration = IllustrationFixture;

    /// Return the illustration service for one entry.
    fn illustration(&self, entry: &NormalizedEntry) -> Result<Self::Illustration> {
        self.items
            .get(entry.word.as_str())
            .cloned()
            .ok_or_else(|| anyhow!("Missing illustration fixture for '{}'", entry.word))
    }
}

/// Recorded deck fixture.
#[derive(Clone, Debug, Default)]
struct RecorderDeck {
    added: Vec<(String, String, String)>,
    attached: Vec<PathBuf>,
}

impl Deck for RecorderDeck {
    /// Attach one media file path.
    fn attach(&mut self, path: &Path) {
        self.attached.push(path.to_path_buf());
    }

    /// Add one note to the deck.
    fn add(&mut self, entry: &NormalizedEntry, audio: &str, image: &str) {
        self.added
            .push((entry.word.clone(), String::from(audio), String::from(image)));
    }
}

/// Recorded progress fixture.
#[derive(Clone, Debug, Default)]
struct RecorderProgress {
    events: Vec<String>,
}

impl SceneProgress for RecorderProgress {
    /// Signal the start of one step.
    fn step(&mut self, name: &str) {
        self.events.push(format!("step:{name}"));
    }

    /// Signal the completion of one step.
    fn done(&mut self, name: &str, label: &str, path: Option<&Path>) {
        self.events.push(format!(
            "done:{name}:{label}:{}",
            path.map(|item| item.display().to_string())
                .unwrap_or_default()
        ));
    }

    /// Signal one retry within rendering.
    fn retry(&mut self, name: &str, attempt: usize, reason: &str) {
        self.events.push(format!("retry:{name}:{attempt}:{reason}"));
    }
}

impl PipelineProgress for RecorderProgress {
    /// Signal the card position within the batch.
    fn card(&mut self, index: usize, total: usize, word: &str) {
        self.events.push(format!("card:{index}:{total}:{word}"));
    }

    /// Signal one skipped entry.
    fn skip(&mut self, word: &str, reason: &str) {
        self.events.push(format!("skip:{word}:{reason}"));
    }
}

/// Media uses injected profile cache names for both service types.
#[test]
fn media_uses_injected_profile_cache_names_for_both_service_types() -> Result<()> {
    let directory = TempDir::new()?;
    let media = Media::configured(FakeClient, directory.path(), FakeProfiles::new("ga", "osd"));
    let audio = media.audio(&entry("focal", "frása", "ga"))?;
    let illustration = media.illustration(&entry("focal", "frása", "ga"))?;
    assert_eq!(
        (
            audio
                .filepath("tone.wav")?
                .to_string_lossy()
                .ends_with("audio-ga/tone.wav"),
            illustration
                .filepath("panel.jpg")?
                .to_string_lossy()
                .ends_with("manga-ga/panel.jpg"),
        ),
        (true, true),
        "media no longer uses the injected profile cache names for both service types"
    );
    Ok(())
}

/// Media keeps the injected fallback OCR selection in illustration wiring.
#[test]
fn media_keeps_the_injected_fallback_ocr_selection() -> Result<()> {
    let directory = TempDir::new()?;
    let media = Media::configured(FakeClient, directory.path(), FakeProfiles::new("ga", "osd"));
    assert!(
        format!("{:?}", media.illustration(&entry("focal", "frása", "ga"))?).contains("\"osd\""),
        "media no longer keeps the injected fallback ocr selection in illustration wiring"
    );
    Ok(())
}

/// Pipeline records failures for empty example text.
#[test]
fn pipeline_records_failures_for_empty_example_text() {
    let directory = TempDir::new().expect("temp directory must exist");
    let mut pipeline = Pipeline::new(
        SuccessAudio::new(directory.path(), "demo.wav", false),
        SuccessIllustration::new(directory.path(), "demo.jpg", false),
        RecorderDeck::default(),
        RecorderProgress::default(),
    );
    let entries = vec![entry("слово", "", "en")];
    assert_eq!(
        pipeline.process(&entries).0,
        vec![Failure::new(
            "слово",
            "Cannot generate audio for empty text"
        )],
        "pipeline no longer records failures for empty example text"
    );
}

/// Pipeline records audio generation failures.
#[test]
fn pipeline_records_audio_generation_failures() {
    let directory = TempDir::new().expect("temp directory must exist");
    let mut pipeline = Pipeline::new(
        FailingAudio::new(directory.path(), "503 UNAVAILABLE: сервер недоступен"),
        SuccessIllustration::new(directory.path(), "demo.jpg", false),
        RecorderDeck::default(),
        RecorderProgress::default(),
    );
    let entries = vec![entry("слово", "фраза", "en")];
    assert_eq!(
        pipeline.process(&entries).0,
        vec![Failure::new("слово", "503 UNAVAILABLE: сервер недоступен")],
        "pipeline no longer records audio generation failures"
    );
}

/// Pipeline records illustration generation failures.
#[test]
fn pipeline_records_illustration_generation_failures() {
    let directory = TempDir::new().expect("temp directory must exist");
    let mut pipeline = Pipeline::new(
        SuccessAudio::new(directory.path(), "demo.wav", false),
        FailingIllustration::new(directory.path(), "503 UNAVAILABLE: сервер недоступен"),
        RecorderDeck::default(),
        RecorderProgress::default(),
    );
    let entries = vec![entry("слово", "фраза", "en")];
    assert_eq!(
        pipeline.process(&entries).0,
        vec![Failure::new("слово", "503 UNAVAILABLE: сервер недоступен")],
        "pipeline no longer records illustration generation failures"
    );
}

/// Pipeline continues after one audio failure and keeps the later success.
#[test]
fn pipeline_continues_after_one_audio_failure_and_keeps_the_later_success() {
    let directory = TempDir::new().expect("temp directory must exist");
    let mut pipeline = Pipeline::new(
        SwitchingAudio {
            items: BTreeMap::from([
                (
                    String::from("провал"),
                    AudioFixture::Failure(FailingAudio::new(
                        directory.path(),
                        "503 UNAVAILABLE: сервер недоступен",
                    )),
                ),
                (
                    String::from("успех"),
                    AudioFixture::Success(SuccessAudio::new(directory.path(), "second.wav", false)),
                ),
            ]),
        },
        SuccessIllustration::new(directory.path(), "second.jpg", false),
        RecorderDeck::default(),
        RecorderProgress::default(),
    );
    let entries = vec![
        entry("провал", "первая фраза", "en"),
        entry("успех", "вторая фраза", "en"),
    ];
    assert_eq!(
        pipeline.process(&entries),
        (
            vec![Failure::new("провал", "503 UNAVAILABLE: сервер недоступен")],
            vec![(
                entry("успех", "вторая фраза", "en"),
                directory.path().join("second.jpg")
            )],
        ),
        "pipeline no longer continues after one audio failure while keeping the later success"
    );
}

/// Pipeline continues after one illustration failure and keeps the later success.
#[test]
fn pipeline_continues_after_one_illustration_failure_and_keeps_the_later_success() {
    let directory = TempDir::new().expect("temp directory must exist");
    let mut pipeline = Pipeline::new(
        SuccessAudio::new(directory.path(), "second.wav", false),
        SwitchingIllustration {
            items: BTreeMap::from([
                (
                    String::from("провал"),
                    IllustrationFixture::Failure(FailingIllustration::new(
                        directory.path(),
                        "503 UNAVAILABLE: сервер недоступен",
                    )),
                ),
                (
                    String::from("успех"),
                    IllustrationFixture::Success(SuccessIllustration::new(
                        directory.path(),
                        "second.jpg",
                        false,
                    )),
                ),
            ]),
        },
        RecorderDeck::default(),
        RecorderProgress::default(),
    );
    let entries = vec![
        entry("провал", "первая фраза", "en"),
        entry("успех", "вторая фраза", "en"),
    ];
    assert_eq!(
        pipeline.process(&entries),
        (
            vec![Failure::new("провал", "503 UNAVAILABLE: сервер недоступен")],
            vec![(
                entry("успех", "вторая фраза", "en"),
                directory.path().join("second.jpg")
            )],
        ),
        "pipeline no longer continues after one illustration failure while keeping the later success"
    );
}

/// Pipeline attaches both media files and keeps the frozen card payload.
#[test]
fn pipeline_attaches_both_media_files_and_keeps_the_frozen_card_payload() {
    let directory = TempDir::new().expect("temp directory must exist");
    let mut pipeline = Pipeline::new(
        SuccessAudio::new(directory.path(), "demo.wav", true),
        SuccessIllustration::new(directory.path(), "demo.jpg", false),
        RecorderDeck::default(),
        RecorderProgress::default(),
    );
    let entries = vec![entry("слово", "фраза", "en")];
    let result = pipeline.process(&entries);
    assert_eq!(
        (
            result,
            pipeline.deck().attached.clone(),
            pipeline.deck().added.clone(),
            pipeline.progress().events.clone(),
        ),
        (
            (
                Vec::<Failure>::new(),
                vec![(
                    entry("слово", "фраза", "en"),
                    directory.path().join("demo.jpg")
                )],
            ),
            vec![
                directory.path().join("demo.wav"),
                directory.path().join("demo.jpg")
            ],
            vec![(
                String::from("слово"),
                String::from("[sound:demo.wav]"),
                String::from(
                    "<img src='demo.jpg' style='max-width: 100%; height: auto; border-radius: 10px'>",
                ),
            )],
            vec![
                String::from("card:1:1:слово"),
                format!("step:Generating audio"),
                format!(
                    "done:Generating audio:cached:{}",
                    directory.path().join("demo.wav").display()
                ),
            ],
        ),
        "pipeline no longer attaches both media files and keeps the frozen card payload"
    );
}
