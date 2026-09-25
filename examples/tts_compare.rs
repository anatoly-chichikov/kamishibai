//! Compare three Gemini speech models with the same voice and production WAV writer.
//!
//! Run `cargo run --example tts_compare -- [output-directory]`, then open index.html.

use std::cell::{Cell, RefCell};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use clap::Parser;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use kamishibai::config::default_store;
use kamishibai::gemini::{
    GeminiClient, GeminiProfile, GenerationStage, HttpTransport, StageModels, Transport,
    TransportResponse,
};
use kamishibai::generation::{Audio, Cache, render_audio_prompt};
use kamishibai::runtime::locations::SystemContext;

/// Generate up to twelve samples once and keep a local listening gallery.
#[derive(Parser)]
struct Arguments {
    /// Directory for the manifest, HTML gallery, and WAV samples.
    #[arg(default_value = "output/tts-compare")]
    output: PathBuf,
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
struct Comparison {
    version: u8,
    voice: String,
    endpoint: String,
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
struct Phrase {
    language: String,
    text: String,
    prompt: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct Manifest {
    comparison: Comparison,
    phrases: Vec<Phrase>,
    samples: Vec<Sample>,
}

#[derive(Debug, Deserialize, Serialize)]
struct Sample {
    phrase: usize,
    model: String,
    file: String,
    outcome: Outcome,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
enum Outcome {
    Pending,
    Requested,
    Complete {
        receipt: Receipt,
    },
    Failed {
        receipt: Option<Receipt>,
        reason: String,
    },
}

#[derive(Debug, Deserialize, Serialize)]
struct Receipt {
    provider: Provider,
    latency_ms: u64,
    usage: Usage,
}

#[derive(Debug, Deserialize, Serialize)]
struct Provider {
    status: u16,
    model_version: Option<String>,
    audio_mime: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct Usage {
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    total_tokens: Option<u64>,
}

struct FixedVoice<T> {
    transport: T,
    voice: String,
    receipt: Rc<RefCell<Option<Receipt>>>,
    attempted: Cell<bool>,
}

impl<T> FixedVoice<T> {
    fn new(
        transport: T,
        voice: String,
        receipt: Rc<RefCell<Option<Receipt>>>,
        attempted: Cell<bool>,
    ) -> Self {
        Self {
            transport,
            voice,
            receipt,
            attempted,
        }
    }
}

impl<T: Transport> Transport for FixedVoice<T> {
    fn post(&self, url: &str, key: &str, body: &str) -> Result<TransportResponse> {
        if self.attempted.replace(true) {
            bail!("A comparison sample cannot make more than one provider request");
        }
        let body = fixed_voice(body, &self.voice)?;
        let start = Instant::now();
        let response = self.transport.post(url, key, &body)?;
        let parsed = serde_json::from_str::<Value>(&response.body).unwrap_or(Value::Null);
        self.receipt.replace(Some(Receipt {
            provider: Provider {
                status: response.status,
                model_version: parsed["modelVersion"].as_str().map(String::from),
                audio_mime: parsed["candidates"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .flat_map(|candidate| {
                        candidate["content"]["parts"]
                            .as_array()
                            .into_iter()
                            .flatten()
                    })
                    .find_map(|part| part["inlineData"]["mimeType"].as_str().map(String::from)),
            },
            latency_ms: u64::try_from(start.elapsed().as_millis())?,
            usage: Usage {
                input_tokens: parsed["usageMetadata"]["promptTokenCount"].as_u64(),
                output_tokens: parsed["usageMetadata"]["candidatesTokenCount"].as_u64(),
                total_tokens: parsed["usageMetadata"]["totalTokenCount"].as_u64(),
            },
        }));
        Ok(response)
    }
}

fn fixed_voice(body: &str, voice: &str) -> Result<String> {
    let mut request: Value = serde_json::from_str(body)?;
    let config = request
        .pointer_mut("/generationConfig/speechConfig/voiceConfig")
        .context("Speech request has no voice configuration")?;
    let modern = config.get("voice").is_some_and(Value::is_string);
    let preview = config
        .pointer("/prebuiltVoiceConfig/voiceName")
        .is_some_and(Value::is_string);
    match (modern, preview) {
        (true, false) => config["voice"] = Value::String(String::from(voice)),
        (false, true) => {
            config["prebuiltVoiceConfig"]["voiceName"] = Value::String(String::from(voice))
        }
        _ => bail!("Speech request must select exactly one supported voice configuration"),
    }
    Ok(serde_json::to_string(&request)?)
}

fn main() -> Result<()> {
    let arguments = Arguments::parse();
    fs::create_dir_all(&arguments.output)?;
    let lock = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(arguments.output.join(".lock"))?;
    lock.try_lock()
        .context("Another comparison already owns this output directory")?;
    let mut manifest = load(&arguments.output)?;
    GeminiProfile::from_models(&manifest.comparison.endpoint, StageModels::default())?;
    save(&arguments.output, &manifest)?;
    if manifest
        .samples
        .iter()
        .any(|sample| matches!(sample.outcome, Outcome::Pending))
    {
        generate(&arguments.output, &mut manifest, &credential()?)?;
    }
    println!(
        "Listening gallery: {}",
        fs::canonicalize(arguments.output.join("index.html"))?.display()
    );
    let failed = manifest
        .samples
        .iter()
        .filter(|sample| matches!(sample.outcome, Outcome::Failed { .. }))
        .count();
    if failed > 0 {
        bail!(
            "{failed} samples failed; the gallery and manifest retain their status without retrying them"
        );
    }
    Ok(())
}

fn plan() -> Manifest {
    let phrases = [
        ("English", "Could you read the address aloud? I thought the next train left at half past three."),
        ("Russian", "Ты всё-таки позвонишь ей сегодня? Хорошо, я подожду у старого подъезда."),
        ("French", "Vous pourriez répéter, s’il vous plaît ? J’aimerais réserver une table pour jeudi soir."),
        ("Greek", "Θα ήθελα έναν καφέ χωρίς ζάχαρη, παρακαλώ. Μπορούμε να καθίσουμε έξω;"),
    ].into_iter().map(|(language, text)| Phrase {
        language: String::from(language),
        text: String::from(text),
        prompt: render_audio_prompt(language),
    }).collect::<Vec<_>>();
    let samples = (0..phrases.len())
        .flat_map(|phrase| {
            [
                "gemini-3.1-flash-tts-preview",
                "gemini-3.8-flash-tts",
                "gemini-3.8-flash-lite-tts",
            ]
            .into_iter()
            .map(move |model| Sample {
                phrase,
                model: String::from(model),
                file: format!("samples/{phrase}/{model}/audio.wav"),
                outcome: Outcome::Pending,
            })
        })
        .collect();
    Manifest {
        comparison: Comparison {
            version: 1,
            voice: String::from("Kore"),
            endpoint: std::env::var("KAMISHIBAI_GEMINI_URL")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| {
                    String::from("https://generativelanguage.googleapis.com/v1beta/models")
                }),
        },
        phrases,
        samples,
    }
}

fn load(output: &Path) -> Result<Manifest> {
    let expected = plan();
    let path = output.join("manifest.json");
    let manifest = if path.exists() {
        serde_json::from_slice::<Manifest>(&fs::read(path)?)?
    } else {
        expected
    };
    validate(output, &manifest, &plan())?;
    Ok(manifest)
}

fn validate(output: &Path, manifest: &Manifest, expected: &Manifest) -> Result<()> {
    if manifest.comparison != expected.comparison
        || manifest.phrases != expected.phrases
        || manifest.samples.len() != expected.samples.len()
    {
        bail!(
            "Comparison settings changed; use a new output directory to preserve the existing samples"
        );
    }
    for (sample, planned) in manifest.samples.iter().zip(&expected.samples) {
        if sample.phrase != planned.phrase
            || sample.model != planned.model
            || sample.file != planned.file
        {
            bail!("Comparison sample identity changed; use a new output directory");
        }
        match sample.outcome {
            Outcome::Requested => bail!(
                "A prior request has uncertain completion; inspect the manifest and use a new output directory"
            ),
            Outcome::Complete { .. } => validate_wav(&output.join(&sample.file))?,
            Outcome::Pending | Outcome::Failed { .. } if output.join(&sample.file).exists() => {
                bail!(
                    "An unrecorded WAV exists for {}; preserve it and use a new output directory",
                    sample.model
                )
            }
            Outcome::Pending | Outcome::Failed { .. } => (),
        }
    }
    Ok(())
}

fn validate_wav(path: &Path) -> Result<()> {
    let bytes =
        fs::read(path).with_context(|| format!("Cannot read recorded WAV {}", path.display()))?;
    if bytes.len() <= 44
        || &bytes[..4] != b"RIFF"
        || &bytes[8..16] != b"WAVEfmt "
        || bytes[20..24] != [1, 0, 1, 0]
        || bytes[24..28] != 24_000_u32.to_le_bytes()
        || bytes[32..36] != [2, 0, 16, 0]
        || &bytes[36..40] != b"data"
        || !bytes.len().is_multiple_of(2)
    {
        bail!(
            "Recorded WAV is not nonempty 24 kHz mono PCM16 at {}",
            path.display()
        );
    }
    let size = u32::from_le_bytes(bytes[40..44].try_into()?);
    if usize::try_from(size)? != bytes.len() - 44 {
        bail!(
            "Recorded WAV length does not match its payload at {}",
            path.display()
        );
    }
    Ok(())
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

fn generate(output: &Path, manifest: &mut Manifest, key: &str) -> Result<()> {
    let transport = HttpTransport::from_timeout(Duration::from_secs(60))?;
    for index in 0..manifest.samples.len() {
        if !matches!(manifest.samples[index].outcome, Outcome::Pending) {
            continue;
        }
        let sample = &manifest.samples[index];
        let phrase = &manifest.phrases[sample.phrase];
        let models = StageModels::default().with_model(GenerationStage::Speech, &sample.model)?;
        let profile = GeminiProfile::from_models(&manifest.comparison.endpoint, models)?;
        let receipt = Rc::new(RefCell::new(None));
        let client = GeminiClient::from_profile(
            key,
            FixedVoice::new(
                transport.clone(),
                manifest.comparison.voice.clone(),
                receipt.clone(),
                Cell::new(false),
            ),
            profile,
        );
        let directory = Path::new(&sample.file)
            .parent()
            .context("Sample WAV needs a parent directory")?;
        let audio = Audio::new(
            Cache::new(directory.to_string_lossy(), output),
            &phrase.prompt,
            client,
        );
        let text = phrase.text.clone();
        eprintln!("{} · {}", phrase.language, sample.model);
        manifest.samples[index].outcome = Outcome::Requested;
        save(output, manifest)?;
        let result = audio
            .generate(&text)
            .and_then(|_| validate_wav(&output.join(&manifest.samples[index].file)));
        manifest.samples[index].outcome = match result {
            Ok(()) => Outcome::Complete {
                receipt: receipt
                    .take()
                    .context("Successful generation did not record a provider response")?,
            },
            Err(error) => {
                let receipt = receipt.take();
                let reason = match &receipt {
                    Some(receipt) if !(200..300).contains(&receipt.provider.status) => format!(
                        "Gemini rejected the speech request (HTTP {})",
                        receipt.provider.status
                    ),
                    Some(_) => String::from(
                        "Gemini returned a response that could not be decoded or saved as a valid WAV",
                    ),
                    None => error.to_string(),
                };
                eprintln!(
                    "{} · {}: {reason}",
                    manifest.phrases[manifest.samples[index].phrase].language,
                    manifest.samples[index].model
                );
                Outcome::Failed { receipt, reason }
            }
        };
        save(output, manifest)?;
    }
    Ok(())
}

fn save(output: &Path, manifest: &Manifest) -> Result<()> {
    replace(
        output,
        "manifest.json",
        &serde_json::to_vec_pretty(manifest)?,
    )?;
    replace(output, "index.html", gallery(manifest).as_bytes())
}

fn replace(output: &Path, filename: &str, bytes: &[u8]) -> Result<()> {
    let mut staged = tempfile::NamedTempFile::new_in(output)?;
    staged.write_all(bytes)?;
    staged.as_file().sync_all()?;
    staged.persist(output.join(filename))?;
    Ok(())
}

fn gallery(manifest: &Manifest) -> String {
    let mut html = String::from(
        r#"<!doctype html><html lang="ru"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Сравнение озвучки Gemini</title><style>
body{margin:0;background:#f4f2ec;color:#202923;font:16px/1.55 system-ui,sans-serif}main{max-width:1150px;margin:0 auto;padding:48px 24px}h1{font-size:clamp(30px,5vw,48px);line-height:1.1;margin:10px 0 20px}h2{font-size:17px;margin:0 0 10px;color:#426b50}p{max-width:820px}.intro{color:#586259}.phrase{font-size:21px;margin:0 0 20px}.row{border-top:1px solid #d2d9ce;margin-top:34px;padding-top:28px}.players{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));gap:16px}.player{padding:20px;background:#fff;border-radius:12px;border:1px solid #dce1d6;min-width:0}.model{font-size:18px;font-weight:650}.id{font-size:12px;color:#687466;overflow-wrap:anywhere;min-height:36px}audio{width:100%;margin-top:16px}.meta{font-size:13px;color:#687466}.badge{font-size:12px;letter-spacing:.1em;text-transform:uppercase;color:#426b50}footer{font-size:14px;color:#687466;margin-top:36px}a{color:#426b50}@media(max-width:760px){.players{grid-template-columns:1fr}main{padding:28px 16px}.id{min-height:0}}
</style><main><div class="badge">Kamishibai · аудиопробы</div><h1>Сравнение озвучки Gemini</h1><p class="intro">Одинаковые фразы, голос Kore и инструкция из приложения. Сравните чёткость, ударения, паузы и интонацию. Каждый вариант получен одним запросом; это образцы для прослушивания, а не статистический тест качества или скорости.</p>"#,
    );
    for (index, phrase) in manifest.phrases.iter().enumerate() {
        html.push_str(&format!(
            "<section class=\"row\"><h2>{}</h2><p class=\"phrase\">{}</p><div class=\"players\">",
            escape(&phrase.language),
            escape(&phrase.text)
        ));
        for sample in manifest
            .samples
            .iter()
            .filter(|sample| sample.phrase == index)
        {
            let title = match sample.model.as_str() {
                "gemini-3.1-flash-tts-preview" => "3.1 Flash Preview",
                "gemini-3.8-flash-tts" => "3.8 Flash · в приложении",
                _ => "3.8 Flash Lite",
            };
            html.push_str(&format!("<div class=\"player\"><div class=\"model\">{title}</div><div class=\"id\">{}</div>", escape(&sample.model)));
            match &sample.outcome {
                Outcome::Complete { receipt } => html.push_str(&format!("<audio controls preload=\"metadata\" aria-label=\"{} — {}\" src=\"{}\"></audio><div class=\"meta\">Запрос: {} мс · токены: {} → {}</div>", escape(&phrase.language), escape(&sample.model), escape(&sample.file), receipt.latency_ms, receipt.usage.input_tokens.map_or_else(|| String::from("—"), |tokens| tokens.to_string()), receipt.usage.output_tokens.map_or_else(|| String::from("—"), |tokens| tokens.to_string()))),
                Outcome::Failed { reason, .. } => html.push_str(&format!("<p class=\"meta\">{}</p>", escape(reason))),
                Outcome::Pending => html.push_str("<p class=\"meta\">Ожидает генерации</p>"),
                Outcome::Requested => html.push_str("<p class=\"meta\">Запрос отправлен</p>"),
            }
            html.push_str("</div>");
        }
        html.push_str("</div></section>");
    }
    html.push_str("<footer>WAV · 24 кГц · моно · PCM 16 бит. <a href=\"manifest.json\">Фразы, модели и данные запросов</a>. Повторный запуск использует записанные результаты.</footer></main></html>");
    html
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::{Outcome, fixed_voice, plan, validate};

    #[test]
    fn modern_speech_keeps_its_schema_when_the_voice_is_fixed() {
        let request = json!({
            "contents": [{"parts": [{"text": "Say in natural French: Bonjour !"}]}],
            "generationConfig": {"responseModalities": ["AUDIO"], "speechConfig": {
                "voiceConfig": {"voice": "Puck"}
            }}
        });
        let mut expected = request.clone();
        expected["generationConfig"]["speechConfig"]["voiceConfig"]["voice"] = json!("Kore");
        let actual: Value =
            serde_json::from_str(&fixed_voice(&request.to_string(), "Kore").unwrap()).unwrap();
        assert_eq!(
            actual, expected,
            "Modern speech did not keep its request shape with the fixed voice"
        );
    }

    #[test]
    fn preview_speech_keeps_its_schema_when_the_voice_is_fixed() {
        let request = json!({
            "contents": [{"parts": [{"text": "Say in natural Greek: Καλημέρα!"}]}],
            "generationConfig": {"responseModalities": ["AUDIO"], "speechConfig": {
                "voiceConfig": {"prebuiltVoiceConfig": {"voiceName": "Zephyr"}}
            }}
        });
        let mut expected = request.clone();
        expected["generationConfig"]["speechConfig"]["voiceConfig"]["prebuiltVoiceConfig"]["voiceName"] =
            json!("Kore");
        let actual: Value =
            serde_json::from_str(&fixed_voice(&request.to_string(), "Kore").unwrap()).unwrap();
        assert_eq!(
            actual, expected,
            "Preview speech did not keep its request shape with the fixed voice"
        );
    }

    #[test]
    fn an_interrupted_request_cannot_be_repeated_on_resume() {
        let directory = tempfile::tempdir().unwrap();
        let mut manifest = plan();
        manifest.samples[0].outcome = Outcome::Requested;
        assert!(
            validate(directory.path(), &manifest, &plan()).is_err(),
            "An uncertain paid request was allowed to run again"
        );
    }

    #[test]
    fn an_existing_audio_file_cannot_be_replaced_by_a_pending_sample() {
        let directory = tempfile::tempdir().unwrap();
        let manifest = plan();
        let path = directory.path().join(&manifest.samples[0].file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"existing audio").unwrap();
        assert!(
            validate(directory.path(), &manifest, &plan()).is_err(),
            "An existing sample was allowed to trigger another paid request"
        );
    }
}
