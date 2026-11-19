//! Journaled, explicitly paid three-to-five-card explanation probe and deck export.
//!
//! Run `cargo run --example explanation_probe -- --paid INPUT.json OUTPUT_DIR`.
//! Input is an array of strict vocabulary entries plus reviewed explanation senses.
//! Only explanations are purchased; existing metadata supplies every other field.

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use kamishibai::anki::{CardModel, StableId, VocabularyDeck, VocabularyNote};
use kamishibai::explanation::{Card, Input, Language, Sense, Task, Writer, compose, write};
use kamishibai::report::{CardSheet, Thumbnail};
use kamishibai::vocabulary::{NonEmptyText, VocabularyEntry};
use reqwest::blocking::Client;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
struct Probe {
    entry: VocabularyEntry,
    senses: Vec<Sense>,
}

struct Journal {
    path: PathBuf,
    card: String,
}

impl Journal {
    fn new(path: PathBuf, card: String) -> Self {
        Self { path, card }
    }

    fn append(&self, event: &Value) -> Result<()> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        writeln!(file, "{event}")?;
        file.sync_all()?;
        Ok(())
    }

    fn balance(&self) -> Result<(usize, f64)> {
        let mut totals = BTreeMap::new();
        if self.path.exists() {
            for line in fs::read_to_string(&self.path)?.lines() {
                let event: Value = serde_json::from_str(line)?;
                totals.insert(
                    event["ticket"]
                        .as_u64()
                        .context("journal ticket is absent")?,
                    event["cost"].as_f64().context("journal cost is absent")?,
                );
            }
        }
        Ok((totals.len(), totals.values().sum()))
    }
}

struct ProbeWriter {
    client: Client,
    journal: Journal,
    prices: [f64; 2],
}

impl ProbeWriter {
    fn new(client: Client, journal: Journal, prices: [f64; 2]) -> Self {
        Self {
            client,
            journal,
            prices,
        }
    }
}

impl Writer for ProbeWriter {
    fn ask(&mut self, prompt: String) -> Result<String> {
        let schema: Value =
            serde_json::from_str(include_str!("../assets/explanation.schema.json"))?;
        let bytes = u32::try_from(prompt.len() + schema.to_string().len() + 10_000)?;
        let reserve = f64::from(bytes) * self.prices[0] + 3000.0 * self.prices[1];
        let (tickets, spent) = self.journal.balance()?;
        ensure!(
            spent + reserve <= 0.3,
            "probe budget would exceed $0.30 with ${spent:.6} spent and ${reserve:.6} reserved"
        );
        let ticket = tickets + 1;
        self.journal.append(
            &json!({"ticket":ticket,"card":self.journal.card,"state":"reserved","cost":reserve}),
        )?;
        let secret = std::env::var("OPENROUTER_API_KEY")
            .context("OPENROUTER_API_KEY is required for the paid probe")?;
        let response = self.client.post("https://openrouter.ai/api/v1/chat/completions").bearer_auth(secret).json(&json!({
            "model":"google/gemini-3.8-flash","messages":[{"role":"user","content":prompt}],
            "response_format":{"type":"json_schema","json_schema":{"name":"explanation","strict":true,"schema":schema}},
            "reasoning":{"effort":"low"},"max_tokens":3000,"usage":{"include":true}
        })).send()?;
        let status = response.status();
        let answer: Value = response.json()?;
        let cost = answer["usage"]["cost"].as_f64().unwrap_or(reserve);
        self.journal.append(&json!({"ticket":ticket,"card":self.journal.card,"state":if answer["usage"]["cost"].is_number(){"settled"}else{"unknown"},"cost":cost,"id":answer["id"],"http":status.as_u16(),"usage":answer["usage"]}))?;
        ensure!(
            status.is_success(),
            "explanation probe returned HTTP {status}"
        );
        ensure!(
            spent + cost <= 0.3,
            "probe budget was exceeded by the provider's reported price"
        );
        answer["choices"][0]["message"]["content"]
            .as_str()
            .map(str::to_owned)
            .context("explanation probe returned no text")
    }
}

fn prices(client: &Client) -> Result<[f64; 2]> {
    let catalog: Value = client
        .get("https://openrouter.ai/api/v1/models")
        .send()?
        .error_for_status()?
        .json()?;
    let model = catalog["data"]
        .as_array()
        .context("model catalog is absent")?
        .iter()
        .find(|model| model["id"] == "google/gemini-3.8-flash")
        .context("probe model is absent")?;
    let pricing = &model["pricing"];
    let rates = [
        pricing["prompt"]
            .as_str()
            .context("input price is absent")?
            .parse::<f64>()?,
        pricing["completion"]
            .as_str()
            .context("output price is absent")?
            .parse::<f64>()?,
    ];
    ensure!(
        rates.iter().all(|rate| rate.is_finite() && *rate >= 0.0)
            && pricing["request"].as_str().unwrap_or("0").parse::<f64>()? == 0.0,
        "probe pricing is unsupported"
    );
    Ok(rates)
}

fn run(input: &Path, output: &Path) -> Result<()> {
    let probes: Vec<Probe> = serde_json::from_str(&fs::read_to_string(input)?)?;
    ensure!(
        (3..=5).contains(&probes.len()),
        "a paid probe requires three to five words"
    );
    fs::create_dir_all(output)?;
    let client = Client::builder()
        .timeout(Duration::from_secs(120))
        .build()?;
    let rates = prices(&client)?;
    let mut entries = Vec::new();
    for probe in probes {
        let mut entry = probe.entry;
        let language =
            Language::from_codes(entry.source.lang.as_str(), entry.target.lang.as_str())?;
        let card = Card::new(
            entry.term.as_str().to_string(),
            entry.meaning.as_str().to_string(),
            entry.target.sentence.as_str().to_string(),
            probe.senses,
        );
        let task = Task::from_input(
            Input::new(
                card,
                entry.source.sentence.as_str().to_string(),
                entry.source.context.as_str().to_string(),
                String::new(),
            ),
            &language,
        )?;
        let mut writer = ProbeWriter::new(
            client.clone(),
            Journal::new(output.join("calls.jsonl"), entry.term.as_str().to_string()),
            rates,
        );
        let parts = write(&mut writer, &task, &language)?;
        fs::write(
            output.join(format!("parts-{}.json", entries.len())),
            serde_json::to_string_pretty(
                &json!({"term":entry.term,"task":task.stamp(),"accepted":parts.is_some(),"parts":parts}),
            )?,
        )?;
        if let Some(parts) = parts {
            entry.source.context = NonEmptyText::new(compose(&parts, &language))?;
            entry.source.context_format = kamishibai::vocabulary::ContextFormat::ExplanationV1;
        }
        entries.push(entry);
    }
    let mut deck = VocabularyDeck::new(
        StableId::new("Explanation probe").value(),
        "Explanation probe",
        VocabularyNote::new(CardModel::new().model()),
        Vec::<(PathBuf, String)>::new(),
    );
    let mut sheet = CardSheet::new();
    for entry in &entries {
        deck.add(entry, "", "");
        sheet.append(entry, None);
    }
    deck.save(output.join("probe.apkg"))?;
    sheet.save(output.join("probe.pdf"), &Thumbnail::new(256))?;
    fs::write(
        output.join("entries.json"),
        serde_json::to_string_pretty(&json!({"entries":entries}))?,
    )?;
    let (_, spent) = Journal::new(output.join("calls.jsonl"), String::new()).balance()?;
    println!(
        "{} cards exported, accounted spend ${spent:.6}",
        entries.len()
    );
    Ok(())
}

fn main() -> Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.len() != 3 || args[0] != "--paid" {
        bail!("usage: explanation_probe --paid INPUT.json OUTPUT_DIR");
    }
    run(Path::new(&args[1]), Path::new(&args[2]))
}
