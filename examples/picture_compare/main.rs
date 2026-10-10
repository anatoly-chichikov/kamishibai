//! Compare two Gemini image models through the complete production picture pipeline.
//!
//! Each card gets one scene, composed once by the baseline arm and installed
//! unchanged for the candidate arm. Both arms then run the full production
//! picture stage: render, literal-text gate, recall, fidelity and zoom judges,
//! retries, scene recomposition and terminal salvage. Only the image model
//! differs. Run
//! `cargo run --release --example picture_compare -- --cards cards.json --candidate <model>`,
//! then open `index.html` in the output directory. A rerun keeps every recorded
//! card and resumes the rest from the per-model caches. `--thinking` pins the image
//! model's thinking level through a local relay that rewrites only image
//! requests, and `--scenes` reuses the scenes of an earlier comparison so both
//! experiments draw exactly the same scenes.

use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use clap::Parser;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use kamishibai::application::{
    CardProduction, GenerationCostLedger, GenerationRun, GenerationScope,
};
use kamishibai::config::default_store;
use kamishibai::gemini::{
    GeminiClient, GeminiProfile, GenerationStage, HttpTransport, StageModels, workflow,
};
use kamishibai::runtime::locations::{LocationArgs, Locations, SystemContext};
use kamishibai::session::{
    Artifact, ArtifactAttempt, ArtifactFile, ArtifactSlot, CardDraft, CardMeta, CardRevision,
    CostRecord, LanguagePair, drafts_from_document,
};
use kamishibai::vocabulary::VocabularyDocument;

const GALLERY: &str = include_str!("gallery.html");
const ENDPOINT: &str = "https://generativelanguage.googleapis.com/v1beta/models";
const UPSTREAM: &str = "https://generativelanguage.googleapis.com";

/// Render every card once per image model and keep a local side-by-side gallery.
#[derive(Parser)]
struct Arguments {
    /// Strict cards JSON in the `--build` format; every entry becomes one compared card.
    #[arg(long)]
    cards: PathBuf,
    /// Directory for the per-model caches, copied frames, manifest, and HTML gallery.
    #[arg(long, default_value = "output/picture-compare")]
    output: PathBuf,
    /// Image model used in production today.
    #[arg(long, default_value = "gemini-nano-banana-2.1")]
    baseline: String,
    /// Image model under evaluation.
    #[arg(long)]
    candidate: String,
    /// Cards processed concurrently; each card runs its two models in sequence.
    #[arg(long, default_value_t = 4)]
    jobs: usize,
    /// Thinking level pinned on every image request of both models; omitted keeps each model's default.
    #[arg(long, value_parser = ["minimal", "medium", "high"])]
    thinking: Option<String>,
    /// Fixed local port of the thinking relay, so a resumed run keeps its cache namespace.
    #[arg(long, default_value_t = 18779)]
    relay_port: u16,
    /// Output directory of an earlier comparison whose scenes both models must draw.
    #[arg(long)]
    scenes: Option<PathBuf>,
}

/// One image model under comparison and the gallery slot it fills.
#[derive(Clone, Debug, Deserialize, Serialize)]
struct Arm {
    slot: String,
    model: String,
    thinking: Option<String>,
}

impl Arm {
    fn new(slot: &str, model: &str, thinking: Option<String>) -> Self {
        Self {
            slot: String::from(slot),
            model: String::from(model),
            thinking,
        }
    }

    fn baseline(&self) -> bool {
        self.slot == "baseline"
    }
}

/// Credential and models endpoint every comparison client talks to.
struct Gateway {
    key: String,
    endpoint: String,
}

impl Gateway {
    fn new(key: String, endpoint: String) -> Self {
        Self { key, endpoint }
    }
}

/// Read-only settings shared by every comparison worker.
struct Bench {
    output: PathBuf,
    gateway: Gateway,
    arms: [Arm; 2],
    ocr: Option<PathBuf>,
}

impl Bench {
    fn new(output: PathBuf, gateway: Gateway, arms: [Arm; 2], ocr: Option<PathBuf>) -> Self {
        Self {
            output,
            gateway,
            arms,
            ocr,
        }
    }

    fn work(&self, drafts: &[CardDraft], next: &AtomicUsize) {
        loop {
            let index = next.fetch_add(1, Ordering::SeqCst);
            let Some(draft) = drafts.get(index) else {
                return;
            };
            for arm in &self.arms {
                if let Err(error) = self.record(arm, index, draft.clone()) {
                    eprintln!(
                        "[{:02}] {} · {}: {error:#}",
                        index + 1,
                        draft.term(),
                        arm.model
                    );
                }
            }
        }
    }

    fn record(&self, arm: &Arm, index: usize, draft: CardDraft) -> Result<()> {
        let samples = self.samples(index, arm);
        if samples.join("run.json").exists() {
            return Ok(());
        }
        let run = self.execute(arm, index, draft)?;
        fs::create_dir_all(&samples)?;
        replace(&samples, "run.json", &serde_json::to_vec_pretty(&run)?)
    }

    fn execute(&self, arm: &Arm, index: usize, draft: CardDraft) -> Result<Run> {
        let models = StageModels::default().with_model(GenerationStage::Picture, &arm.model)?;
        let profile = GeminiProfile::from_models(&self.gateway.endpoint, models)?;
        let client = GeminiClient::from_profile(&self.gateway.key, HttpTransport::new(), profile);
        let cache = self.output.join("cache");
        let root = cache.join("profiles").join(client.profile_identity());
        self.prepare(index, &root)?;
        let ledger = Arc::new(Ledger::default());
        let costs: Arc<dyn GenerationCostLedger> = ledger.clone();
        let stash = arm
            .baseline()
            .then(|| Stash::new(root.clone(), self.scenes(index)));
        let production = Isolated::new(
            workflow(client, cache, self.output.join("decks"), Some(costs)),
            self.output.join("placeholder.wav"),
            stash,
        );
        let term = String::from(draft.term());
        let mut run = GenerationRun::new(vec![draft])?;
        let mut seconds = 0.0;
        while let Some((_, artifact)) = run.next() {
            ledger.enter(artifact)?;
            let start = Instant::now();
            let step = run
                .advance(&production)?
                .context("A scheduled artifact produced no generation step")?;
            let (_, _, _, error) = step.into_parts();
            if artifact == Artifact::Picture {
                seconds += start.elapsed().as_secs_f64();
            }
            if artifact == Artifact::Scene || artifact == Artifact::Picture {
                eprintln!(
                    "[{:02}] {term} · {} · {}{}",
                    index + 1,
                    arm.model,
                    artifact.label(),
                    error.map_or_else(|| String::from(" ✓"), |error| format!(" ✗ {error}"))
                );
            }
        }
        let finished = run
            .into_drafts()
            .into_iter()
            .next()
            .context("A single-card run returned no draft")?;
        Run::observe(
            arm,
            &finished,
            Effort::new(seconds, ledger.spend(&arm.model)?, false),
            &self.samples(index, arm),
            &self.output,
        )
    }

    fn prepare(&self, index: usize, root: &Path) -> Result<()> {
        fs::create_dir_all(root)?;
        if let Some(ocr) = &self.ocr {
            seed(ocr, &root.join("ocr-models"))?;
        }
        install(&self.scenes(index), root)
    }

    fn samples(&self, index: usize, arm: &Arm) -> PathBuf {
        self.output
            .join("samples")
            .join(format!("{:02}", index + 1))
            .join(&arm.slot)
    }

    fn scenes(&self, index: usize) -> PathBuf {
        self.output.join("scenes").join(format!("{:02}", index + 1))
    }
}

/// Delegates card production but stubs speech and stashes the first committed scene.
struct Isolated<P> {
    production: P,
    placeholder: PathBuf,
    stash: Option<Stash>,
}

impl<P> Isolated<P> {
    fn new(production: P, placeholder: PathBuf, stash: Option<Stash>) -> Self {
        Self {
            production,
            placeholder,
            stash,
        }
    }
}

impl<P: CardProduction> CardProduction for Isolated<P> {
    fn generate_draft_meta_in(
        &self,
        slot: usize,
        draft: &CardDraft,
    ) -> ArtifactAttempt<(CardRevision, Option<ArtifactFile>)> {
        self.production.generate_draft_meta_in(slot, draft)
    }

    fn generate_scene_in(&self, slot: usize, draft: &CardDraft) -> ArtifactAttempt<ArtifactFile> {
        let attempt = self.production.generate_scene_in(slot, draft);
        match &self.stash {
            Some(stash) => stash.keep(attempt),
            None => attempt,
        }
    }

    fn generate_picture_in(&self, slot: usize, draft: &CardDraft) -> ArtifactAttempt<ArtifactFile> {
        self.production.generate_picture_in(slot, draft)
    }

    fn generate_sound_in(&self, _slot: usize, _draft: &CardDraft) -> ArtifactAttempt<ArtifactFile> {
        ArtifactAttempt::unmetered(Ok(ArtifactFile::new(
            "audio.wav",
            self.placeholder.clone(),
            "4 B",
            true,
        )))
    }

    fn store_card_meta(
        &self,
        term: &str,
        understanding: &str,
        pair: &LanguagePair,
        meta: &CardMeta,
    ) -> Result<ArtifactFile> {
        self.production
            .store_card_meta(term, understanding, pair, meta)
    }
}

/// Keeps the baseline arm's first committed scene so the candidate draws the same one.
struct Stash {
    root: PathBuf,
    directory: PathBuf,
}

impl Stash {
    fn new(root: PathBuf, directory: PathBuf) -> Self {
        Self { root, directory }
    }

    fn keep(&self, attempt: ArtifactAttempt<ArtifactFile>) -> ArtifactAttempt<ArtifactFile> {
        let mut failure = None;
        let attempt = attempt.map(|file| {
            failure = self.save(file.path()).err();
            file
        });
        match failure {
            Some(error) => ArtifactAttempt::unmetered(Err(error)),
            None => attempt,
        }
    }

    fn save(&self, scene: &Path) -> Result<()> {
        let visual = scene
            .parent()
            .context("A committed scene needs its visual directory")?;
        let relative = visual.strip_prefix(&self.root).with_context(|| {
            format!(
                "Scene {} is outside the profile root {}",
                scene.display(),
                self.root.display()
            )
        })?;
        fs::create_dir_all(&self.directory)?;
        fs::copy(scene, self.directory.join("scene.json"))?;
        if visual.join("scene-attempt.json").exists() {
            fs::copy(
                visual.join("scene-attempt.json"),
                self.directory.join("scene-attempt.json"),
            )?;
        }
        replace(
            &self.directory,
            "location.txt",
            relative.to_string_lossy().as_bytes(),
        )
    }
}

/// Records every provider request together with the artifact stage that issued it.
#[derive(Default)]
struct Ledger {
    stage: Mutex<Option<Artifact>>,
    entries: Mutex<Vec<(Option<Artifact>, Value)>>,
}

impl Ledger {
    fn enter(&self, artifact: Artifact) -> Result<()> {
        *self
            .stage
            .lock()
            .map_err(|_| anyhow!("Ledger stage lock is poisoned"))? = Some(artifact);
        Ok(())
    }

    fn spend(&self, image: &str) -> Result<Spend> {
        let entries = self
            .entries
            .lock()
            .map_err(|_| anyhow!("Ledger entry lock is poisoned"))?;
        Ok(entries
            .iter()
            .filter(|(stage, _)| *stage == Some(Artifact::Picture))
            .fold(Spend::default(), |spend, (_, record)| {
                spend.with(record, image)
            }))
    }
}

impl GenerationCostLedger for Ledger {
    fn record(&self, _scope: GenerationScope, usage: &CostRecord) -> Result<()> {
        let stage = *self
            .stage
            .lock()
            .map_err(|_| anyhow!("Ledger stage lock is poisoned"))?;
        self.entries
            .lock()
            .map_err(|_| anyhow!("Ledger entry lock is poisoned"))?
            .push((stage, serde_json::to_value(usage)?));
        Ok(())
    }
}

/// Provider spend recorded while the picture stage ran, in nano-dollars.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct Spend {
    image: u64,
    review: u64,
    requests: u64,
    pricing: Pricing,
}

impl Spend {
    fn with(self, record: &Value, image: &str) -> Self {
        let model = record["model"].as_str().unwrap_or_default();
        let requests = record["requests"].as_u64().unwrap_or(1);
        let nanos = record["cost"]["nanos"].as_u64().unwrap_or(0);
        let pricing = if record["cost"]["incomplete"].as_bool().unwrap_or(false) {
            self.pricing.max(Pricing::Unknown)
        } else {
            self.pricing
        };
        if model == image {
            Self {
                image: self.image + nanos,
                requests: self.requests + requests,
                pricing,
                ..self
            }
        } else {
            Self {
                review: self.review + nanos,
                pricing,
                ..self
            }
        }
    }
}

/// How trustworthy a recorded spend is.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
enum Pricing {
    #[default]
    Exact,
    Unknown,
}

/// Wall time, spend, and scene provenance of one picture stage.
#[derive(Clone, Debug, Deserialize, Serialize)]
struct Effort {
    seconds: f64,
    spend: Spend,
    shared_scene: bool,
}

impl Effort {
    fn new(seconds: f64, spend: Spend, shared_scene: bool) -> Self {
        Self {
            seconds,
            spend,
            shared_scene,
        }
    }
}

/// Final picture outcome of one card under one image model.
#[derive(Clone, Debug, Deserialize, Serialize)]
struct Verdict {
    outcome: String,
    attempts: u8,
    picture: Option<String>,
    note: Option<String>,
}

impl Verdict {
    fn new(outcome: &str, attempts: u8, picture: Option<String>, note: Option<String>) -> Self {
        Self {
            outcome: String::from(outcome),
            attempts,
            picture,
            note,
        }
    }
}

/// One archived picture attempt with its judged verdict as the renderer stored it.
#[derive(Clone, Debug, Deserialize, Serialize)]
struct Frame {
    sequence: u64,
    image: Option<String>,
    verdict: Value,
}

impl Frame {
    fn new(sequence: u64, image: Option<String>, verdict: Value) -> Self {
        Self {
            sequence,
            image,
            verdict,
        }
    }

    fn salvaged(&self) -> bool {
        self.verdict["status"].as_str() == Some("salvaged")
    }
}

/// Everything the gallery shows for one card under one image model.
#[derive(Clone, Debug, Deserialize, Serialize)]
struct Run {
    model: String,
    verdict: Verdict,
    frames: Vec<Frame>,
    effort: Effort,
}

impl Run {
    fn new(model: String, verdict: Verdict, frames: Vec<Frame>, effort: Effort) -> Self {
        Self {
            model,
            verdict,
            frames,
            effort,
        }
    }

    fn observe(
        arm: &Arm,
        draft: &CardDraft,
        effort: Effort,
        samples: &Path,
        output: &Path,
    ) -> Result<Self> {
        fs::create_dir_all(samples)?;
        let artifacts = draft.artifacts();
        let picture = artifacts.picture();
        let visual = picture
            .file()
            .or_else(|| artifacts.scene().file())
            .and_then(|file| file.path().parent().map(Path::to_path_buf));
        let frames = match &visual {
            Some(visual) => frames(&visual.join("attempts"), samples, output)?,
            None => Vec::new(),
        };
        let image = match picture.file() {
            Some(file) => Some(copy(file.path(), &samples.join("picture.jpg"), output)?),
            None => None,
        };
        let shared = artifacts.scene().file().is_some_and(ArtifactFile::cached);
        let verdict = Verdict::new(
            outcome(picture, &frames),
            picture.tally().done() + u8::from(picture.ready()),
            image,
            picture
                .latest_fault()
                .map(|fault| String::from(fault.reason())),
        );
        Ok(Self::new(
            arm.model.clone(),
            verdict,
            frames,
            Effort {
                shared_scene: shared,
                ..effort
            },
        ))
    }
}

fn outcome(picture: &ArtifactSlot, frames: &[Frame]) -> &'static str {
    match (picture.ready(), frames.iter().any(Frame::salvaged)) {
        (true, true) => "salvaged",
        (true, false) => "accepted",
        (false, _) => "failed",
    }
}

fn frames(attempts: &Path, samples: &Path, output: &Path) -> Result<Vec<Frame>> {
    if !attempts.is_dir() {
        return Ok(Vec::new());
    }
    let mut frames = Vec::new();
    for entry in fs::read_dir(attempts)? {
        let path = entry?.path();
        let Some(sequence) = path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_prefix("attempt-"))
            .and_then(|rest| rest.strip_suffix(".json"))
            .and_then(|digits| digits.parse::<u64>().ok())
        else {
            continue;
        };
        let verdict: Value = serde_json::from_slice(&fs::read(&path)?)
            .with_context(|| format!("Attempt verdict {} is not JSON", path.display()))?;
        let image = match verdict["image"].as_str() {
            Some(name) if attempts.join(name).is_file() => {
                Some(copy(&attempts.join(name), &samples.join(name), output)?)
            }
            _ => None,
        };
        frames.push(Frame::new(sequence, image, verdict));
    }
    frames.sort_by_key(|frame| frame.sequence);
    Ok(frames)
}

fn copy(source: &Path, target: &Path, output: &Path) -> Result<String> {
    fs::copy(source, target)
        .with_context(|| format!("Cannot copy {} into the gallery", source.display()))?;
    Ok(target
        .strip_prefix(output)
        .context("Gallery files must live under the output directory")?
        .to_string_lossy()
        .replace('\\', "/"))
}

fn install(stash: &Path, root: &Path) -> Result<()> {
    let location = stash.join("location.txt");
    if !location.exists() {
        return Ok(());
    }
    let visual = root.join(fs::read_to_string(location)?.trim());
    if visual.join("scene.json").exists() {
        return Ok(());
    }
    fs::create_dir_all(&visual)?;
    for name in ["scene.json", "scene-attempt.json"] {
        if stash.join(name).exists() {
            fs::copy(stash.join(name), visual.join(name))?;
        }
    }
    Ok(())
}

fn seed(source: &Path, target: &Path) -> Result<()> {
    if target.exists() {
        return Ok(());
    }
    fs::create_dir_all(target)?;
    for entry in fs::read_dir(source)? {
        let path = entry?.path();
        if path.is_file() {
            fs::copy(
                &path,
                target.join(path.file_name().context("OCR model needs a name")?),
            )?;
        }
    }
    Ok(())
}

/// One compared card with both runs, as the gallery renders it.
#[derive(Debug, Deserialize, Serialize)]
struct Card {
    term: String,
    target: String,
    source: String,
    runs: serde_json::Map<String, Value>,
}

/// The complete comparison document embedded into the gallery.
#[derive(Debug, Deserialize, Serialize)]
struct Gallery {
    arms: [Arm; 2],
    cards: Vec<Card>,
}

impl Gallery {
    fn collect(bench: &Bench, document: &VocabularyDocument) -> Result<Self> {
        let mut cards = Vec::with_capacity(document.entries.len());
        for (index, entry) in document.entries.iter().enumerate() {
            let mut runs = serde_json::Map::new();
            for arm in &bench.arms {
                let path = bench.samples(index, arm).join("run.json");
                if path.exists() {
                    runs.insert(arm.slot.clone(), serde_json::from_slice(&fs::read(path)?)?);
                }
            }
            cards.push(Card {
                term: String::from(entry.term.as_str()),
                target: String::from(entry.target.sentence.as_str()),
                source: String::from(entry.source.sentence.as_str()),
                runs,
            });
        }
        Ok(Self {
            arms: bench.arms.clone(),
            cards,
        })
    }

    fn page(&self) -> Result<String> {
        Ok(GALLERY.replace(
            "__DATA__",
            &serde_json::to_string(self)?.replace("</", "<\\/"),
        ))
    }
}

fn main() -> Result<()> {
    let arguments = Arguments::parse();
    fs::create_dir_all(&arguments.output)?;
    let output = fs::canonicalize(&arguments.output)?;
    let lock = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(output.join(".lock"))?;
    lock.try_lock()
        .context("Another comparison already owns this output directory")?;
    let document = VocabularyDocument::load(&arguments.cards)?;
    let (_, drafts) = drafts_from_document(&document)?;
    if let Some(earlier) = &arguments.scenes {
        adopt(&earlier.join("scenes"), &output.join("scenes"))?;
    }
    let endpoint = match &arguments.thinking {
        Some(level) => {
            let listener =
                TcpListener::bind(("127.0.0.1", arguments.relay_port)).with_context(|| {
                    format!(
                        "Cannot bind the thinking relay to 127.0.0.1:{}",
                        arguments.relay_port
                    )
                })?;
            let relay = Arc::new(Relay::new(
                level.to_uppercase(),
                vec![arguments.baseline.clone(), arguments.candidate.clone()],
                reqwest::blocking::Client::builder()
                    .timeout(Duration::from_secs(300))
                    .build()?,
                Mutex::new(
                    OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(output.join("relay.jsonl"))?,
                ),
            ));
            thread::spawn(move || relay.serve(listener));
            format!(
                "http://127.0.0.1:{}/{level}/v1beta/models",
                arguments.relay_port
            )
        }
        None => endpoint(),
    };
    let bench = Bench::new(
        output.clone(),
        Gateway::new(credential()?, endpoint),
        [
            Arm::new("baseline", &arguments.baseline, arguments.thinking.clone()),
            Arm::new(
                "candidate",
                &arguments.candidate,
                arguments.thinking.clone(),
            ),
        ],
        ocr_models()?,
    );
    replace(&output, "placeholder.wav", b"RIFF")?;
    let next = AtomicUsize::new(0);
    thread::scope(|scope| {
        for _ in 0..arguments.jobs.max(1) {
            scope.spawn(|| bench.work(&drafts, &next));
        }
    });
    let gallery = Gallery::collect(&bench, &document)?;
    replace(
        &output,
        "manifest.json",
        &serde_json::to_vec_pretty(&gallery)?,
    )?;
    replace(&output, "index.html", gallery.page()?.as_bytes())?;
    println!("Gallery: {}", output.join("index.html").display());
    Ok(())
}

/// Forwards Gemini requests upstream and pins the thinking level of image renders.
struct Relay {
    level: String,
    images: Vec<String>,
    client: reqwest::blocking::Client,
    log: Mutex<fs::File>,
}

impl Relay {
    fn new(
        level: String,
        images: Vec<String>,
        client: reqwest::blocking::Client,
        log: Mutex<fs::File>,
    ) -> Self {
        Self {
            level,
            images,
            client,
            log,
        }
    }

    fn serve(self: Arc<Self>, listener: TcpListener) {
        for stream in listener.incoming().flatten() {
            let relay = Arc::clone(&self);
            thread::spawn(move || {
                if let Err(error) = relay.answer(stream) {
                    eprintln!("relay: {error:#}");
                }
            });
        }
    }

    fn answer(&self, mut stream: TcpStream) -> Result<()> {
        stream.set_read_timeout(Some(Duration::from_secs(60)))?;
        let (path, key, body) = request(&stream)?;
        let path = path
            .strip_prefix(&format!("/{}", self.level.to_lowercase()))
            .context("Relay request misses its thinking-level prefix")?
            .to_string();
        let image = self
            .images
            .iter()
            .find(|model| path.ends_with(&format!("/{model}:generateContent")));
        let body = match image {
            Some(_) => self.pin(&body)?,
            None => body,
        };
        let start = Instant::now();
        let response = self
            .client
            .post(format!("{UPSTREAM}{path}"))
            .header("x-goog-api-key", key)
            .header("content-type", "application/json")
            .body(body)
            .send()?;
        let status = response.status();
        let reply = response.bytes()?;
        if let Some(model) = image {
            self.note(model, status.as_u16(), start.elapsed(), &reply)?;
        }
        write!(
            stream,
            "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            status.as_u16(),
            status.canonical_reason().unwrap_or("Status"),
            reply.len()
        )?;
        stream.write_all(&reply)?;
        Ok(())
    }

    fn pin(&self, body: &[u8]) -> Result<Vec<u8>> {
        let mut request: Value = serde_json::from_slice(body)?;
        let config = request
            .get_mut("generationConfig")
            .and_then(Value::as_object_mut)
            .context("Image request has no generation config to pin")?;
        config.insert(
            String::from("thinkingConfig"),
            serde_json::json!({"thinkingLevel": self.level}),
        );
        Ok(serde_json::to_vec(&request)?)
    }

    fn note(&self, model: &str, status: u16, elapsed: Duration, reply: &[u8]) -> Result<()> {
        let parsed = serde_json::from_slice::<Value>(reply).unwrap_or(Value::Null);
        let line = serde_json::json!({
            "model": model,
            "level": self.level,
            "status": status,
            "ms": u64::try_from(elapsed.as_millis())?,
            "version": parsed["modelVersion"],
            "usage": parsed["usageMetadata"],
        });
        let mut log = self
            .log
            .lock()
            .map_err(|_| anyhow!("Relay log lock is poisoned"))?;
        writeln!(log, "{line}")?;
        Ok(())
    }
}

fn request(stream: &TcpStream) -> Result<(String, String, Vec<u8>)> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let path = line
        .split_whitespace()
        .nth(1)
        .context("Relay request has no path")?
        .to_string();
    let mut key = String::new();
    let mut length = 0;
    loop {
        let mut header = String::new();
        reader.read_line(&mut header)?;
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            match name.trim().to_ascii_lowercase().as_str() {
                "x-goog-api-key" => key = value.trim().to_string(),
                "content-length" => length = value.trim().parse()?,
                _ => (),
            }
        }
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    Ok((path, key, body))
}

fn adopt(source: &Path, target: &Path) -> Result<()> {
    for entry in fs::read_dir(source)
        .with_context(|| format!("Earlier scenes are missing at {}", source.display()))?
    {
        let card = entry?.path();
        let destination = target.join(card.file_name().context("Scene folder needs a name")?);
        if destination.exists() || !card.is_dir() {
            continue;
        }
        fs::create_dir_all(&destination)?;
        for file in fs::read_dir(&card)? {
            let file = file?.path();
            fs::copy(
                &file,
                destination.join(file.file_name().context("Scene file needs a name")?),
            )?;
        }
    }
    Ok(())
}

fn endpoint() -> String {
    std::env::var("KAMISHIBAI_GEMINI_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| String::from(ENDPOINT))
}

fn ocr_models() -> Result<Option<PathBuf>> {
    let models = Locations::new(LocationArgs::default(), SystemContext)
        .cache()?
        .join("ocr-models");
    Ok(models.is_dir().then_some(models))
}

fn credential() -> Result<String> {
    if let Some(key) = std::env::var("GEMINI_API_KEY")
        .ok()
        .filter(|value| !value.trim().is_empty())
    {
        return Ok(key);
    }
    default_store(&SystemContext)?
        .read()?
        .api_key
        .filter(|value| !value.trim().is_empty())
        .context(
            "No Gemini API key found; save one with kamishibai config --key or set GEMINI_API_KEY",
        )
}

fn replace(directory: &Path, filename: &str, bytes: &[u8]) -> Result<()> {
    let mut staged = tempfile::NamedTempFile::new_in(directory)?;
    staged.write_all(bytes)?;
    staged.as_file().sync_all()?;
    staged.persist(directory.join(filename))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use std::sync::Mutex;

    use super::{Pricing, Relay, Spend};

    #[test]
    fn judge_spend_cannot_leak_into_the_image_bill() {
        let record = json!({"model": "gemini-3.8-flash", "requests": 1, "input_tokens": 10, "output_tokens": 5, "total_tokens": 15, "cost": {"nanos": 777}});
        assert_eq!(
            Spend::default()
                .with(&record, "gemini-nano-banana-2.1")
                .image,
            0,
            "a judge request was billed as an image render"
        );
    }

    #[test]
    fn pinned_thinking_cannot_drop_the_image_configuration() {
        let relay = Relay::new(
            String::from("HIGH"),
            vec![String::from("gemini-nano-banana-2.1")],
            reqwest::blocking::Client::new(),
            Mutex::new(tempfile::tempfile().unwrap()),
        );
        let body = json!({"contents": [{"parts": [{"text": "draw"}]}], "generationConfig": {"responseModalities": ["IMAGE"], "imageConfig": {"aspectRatio": "1:1"}}});
        let pinned: serde_json::Value =
            serde_json::from_slice(&relay.pin(&serde_json::to_vec(&body).unwrap()).unwrap())
                .unwrap();
        assert_eq!(
            pinned["generationConfig"],
            json!({"responseModalities": ["IMAGE"], "imageConfig": {"aspectRatio": "1:1"}, "thinkingConfig": {"thinkingLevel": "HIGH"}}),
            "pinning the thinking level altered the production image configuration"
        );
    }

    #[test]
    fn one_unpriced_request_cannot_leave_the_spend_marked_exact() {
        let record = json!({"model": "gemini-unknown-image", "requests": 1, "input_tokens": 10, "output_tokens": 1_120, "total_tokens": 1_130, "cost": {"nanos": 0, "incomplete": true}});
        assert_eq!(
            Spend::default()
                .with(&record, "gemini-unknown-image")
                .pricing,
            Pricing::Unknown,
            "an unpriced image request was presented as an exact spend"
        );
    }
}
