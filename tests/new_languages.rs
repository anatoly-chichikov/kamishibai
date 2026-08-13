//! Offline bidirectional smoke coverage for the ten languages added in 1.8.

use std::fs;
use std::io::{ErrorKind, Write};
use std::net::TcpListener;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

use assert_cmd::Command;
use kamishibai::languages::{ReportLabels, naming};
use kamishibai::session::{CardCell, LanguagePair};
use kamishibai::vocabulary::VocabularyDocument;
use serde_json::{Value, json};
use tempfile::TempDir;

#[derive(Debug, Eq, PartialEq)]
struct Smoke {
    status: Option<i32>,
    pair: (String, String),
    labels: (String, String, String, String),
    deck: String,
    cache_pair: String,
    cached: bool,
    external_calls: usize,
}

struct GeminiStub {
    calls: Arc<AtomicUsize>,
    stop: mpsc::Sender<()>,
    worker: Option<thread::JoinHandle<()>>,
    url: String,
}

impl GeminiStub {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("Gemini stub must bind");
        listener
            .set_nonblocking(true)
            .expect("Gemini stub must become nonblocking");
        let url = format!(
            "http://{}",
            listener
                .local_addr()
                .expect("Gemini stub must have an address")
        );
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = calls.clone();
        let (stop, stopped) = mpsc::channel();
        let worker = thread::spawn(move || {
            loop {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        observed.fetch_add(1, Ordering::SeqCst);
                        let _ = stream.write_all(
                        b"HTTP/1.1 503 Service Unavailable\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
                    );
                    }
                    Err(error) if error.kind() == ErrorKind::WouldBlock => {}
                    Err(_) => return,
                }
                if stopped.recv_timeout(Duration::from_millis(2)).is_ok() {
                    return;
                }
            }
        });
        Self {
            calls,
            stop,
            worker: Some(worker),
            url,
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl Drop for GeminiStub {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(worker) = self.worker.take() {
            worker.join().expect("Gemini stub must stop cleanly");
        }
    }
}

fn run(known: &str, learning: &str, source: &str, target: &str) -> Smoke {
    let data = TempDir::new().expect("data tempdir must exist");
    let cache = TempDir::new().expect("cache tempdir must exist");
    let output = TempDir::new().expect("output tempdir must exist");
    let stub = GeminiStub::new();
    let document = json!({
        "entries": [{
            "term": target,
            "meaning": "one precise meaning",
            "pronunciation": "irregular pronunciation",
            "transcription": "irregular sentence transcription",
            "importance": 7,
            "source": {
                "sentence": source,
                "lang": known,
                "highlight": source,
                "hint": "one concrete cue",
                "context": "one concrete context"
            },
            "target": {"sentence": target, "lang": learning}
        }]
    });
    let cards = data.path().join("cards.json");
    fs::write(
        cards.as_path(),
        serde_json::to_vec_pretty(&document).expect("cards JSON must serialize"),
    )
    .expect("cards JSON must write");
    let response = Command::cargo_bin("kamishibai")
        .expect("binary must build")
        .env("KAMISHIBAI_DATA", data.path())
        .env("KAMISHIBAI_CACHE", cache.path())
        .env("KAMISHIBAI_OUTPUT", output.path())
        .env("KAMISHIBAI_GEMINI_URL", stub.url.as_str())
        .env_remove("GEMINI_API_KEY")
        .args([
            "new",
            "--build",
            cards.to_str().expect("cards path must be UTF-8"),
            "--id",
            "role-smoke",
            "--json",
        ])
        .output()
        .expect("offline role smoke must run");
    let value = serde_json::from_slice::<Value>(&response.stdout)
        .expect("offline role smoke must emit JSON");
    let entry = VocabularyDocument::load(cards)
        .expect("cards document must load")
        .entries
        .into_iter()
        .next()
        .expect("cards document must contain one entry");
    let labels = ReportLabels::default().selected(&entry);
    let cell = CardCell::new(
        cache.path(),
        &LanguagePair::new(learning, known),
        target,
        "one precise meaning",
    )
    .cache();
    Smoke {
        status: response.status.code(),
        pair: (
            value["pair"]["known"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            value["pair"]["learning"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
        ),
        labels: (
            labels.sentence,
            labels.context,
            labels.hint,
            labels.importance,
        ),
        deck: naming(None, &[entry]).name,
        cache_pair: cell
            .path()
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_string(),
        cached: cell.path().join("meta.json").is_file(),
        external_calls: stub.calls(),
    }
}

fn both(code: &str, native: &str, target: &str) -> (Smoke, Smoke) {
    (
        run("EN", code, "An irregular source sentence.", target),
        run(code, "EN", native, "An irregular target sentence."),
    )
}

fn expected(code: &str, deck: &str, labels: (&str, &str, &str, &str)) -> (Smoke, Smoke) {
    (
        Smoke {
            status: Some(0),
            pair: (String::from("EN"), String::from(code)),
            labels: (
                String::from("Translation"),
                String::from("Context"),
                String::from("Hint"),
                String::from("Importance"),
            ),
            deck: String::from(deck),
            cache_pair: format!("EN-{code}"),
            cached: true,
            external_calls: 0,
        },
        Smoke {
            status: Some(0),
            pair: (String::from(code), String::from("EN")),
            labels: (
                String::from(labels.0),
                String::from(labels.1),
                String::from(labels.2),
                String::from(labels.3),
            ),
            deck: String::from("English Vocabulary"),
            cache_pair: format!("{code}-EN"),
            cached: true,
            external_calls: 0,
        },
    )
}

#[test]
fn korean_works_in_both_language_roles_offline() {
    assert_eq!(
        both("KO", "표현을 익혔다.", "표현을 익혔다."),
        expected(
            "KO",
            "Korean Vocabulary",
            ("번역", "문맥", "힌트", "중요도")
        ),
        "Korean did not preserve both offline language roles"
    );
}

#[test]
fn turkish_works_in_both_language_roles_offline() {
    assert_eq!(
        both("TR", "İfadeyi öğrendim.", "İfadeyi öğrendim."),
        expected(
            "TR",
            "Turkish Vocabulary",
            ("Çeviri", "Bağlam", "İpucu", "Önem")
        ),
        "Turkish did not preserve both offline language roles"
    );
}

#[test]
fn polish_works_in_both_language_roles_offline() {
    assert_eq!(
        both("PL", "Poznałem wyrażenie.", "Poznałem wyrażenie."),
        expected(
            "PL",
            "Polish Vocabulary",
            ("Tłumaczenie", "Kontekst", "Wskazówka", "Ważność")
        ),
        "Polish did not preserve both offline language roles"
    );
}

#[test]
fn ukrainian_works_in_both_language_roles_offline() {
    assert_eq!(
        both("UK", "Я вивчив вислів.", "Я вивчив вислів."),
        expected(
            "UK",
            "Ukrainian Vocabulary",
            ("Переклад", "Контекст", "Підказка", "Важливість")
        ),
        "Ukrainian did not preserve both offline language roles"
    );
}

#[test]
fn indonesian_works_in_both_language_roles_offline() {
    assert_eq!(
        both(
            "ID",
            "Saya mempelajari ungkapan itu.",
            "Saya mempelajari ungkapan itu."
        ),
        expected(
            "ID",
            "Indonesian Vocabulary",
            ("Terjemahan", "Konteks", "Petunjuk", "Tingkat Kepentingan")
        ),
        "Indonesian did not preserve both offline language roles"
    );
}

#[test]
fn hindi_works_in_both_language_roles_offline() {
    assert_eq!(
        both("HI", "मैंने यह अभिव्यक्ति सीखी।", "मैंने यह अभिव्यक्ति सीखी।"),
        expected("HI", "Hindi Vocabulary", ("अनुवाद", "संदर्भ", "संकेत", "महत्त्व")),
        "Hindi did not preserve both offline language roles"
    );
}

#[test]
fn arabic_works_in_both_language_roles_offline() {
    assert_eq!(
        both("AR", "تعلّمت هذا التعبير.", "تعلّمت هذا التعبير."),
        expected(
            "AR",
            "Arabic Vocabulary",
            ("الترجمة", "السياق", "تلميح", "الأهمية")
        ),
        "Arabic did not preserve both offline language roles"
    );
}

#[test]
fn thai_works_in_both_language_roles_offline() {
    assert_eq!(
        both("TH", "ฉันเรียนรู้สำนวนนี้แล้ว", "ฉันเรียนรู้สำนวนนี้แล้ว"),
        expected(
            "TH",
            "Thai Vocabulary",
            ("คำแปล", "บริบท", "คำใบ้", "ความสำคัญ")
        ),
        "Thai did not preserve both offline language roles"
    );
}

#[test]
fn hebrew_works_in_both_language_roles_offline() {
    assert_eq!(
        both("HE", "למדתי את הביטוי הזה.", "למדתי את הביטוי הזה."),
        expected(
            "HE",
            "Hebrew Vocabulary",
            ("תרגום", "הקשר", "רמז", "חשיבות")
        ),
        "Hebrew did not preserve both offline language roles"
    );
}

#[test]
fn vietnamese_works_in_both_language_roles_offline() {
    assert_eq!(
        both(
            "VI",
            "Tôi đã học cách diễn đạt này.",
            "Tôi đã học cách diễn đạt này."
        ),
        expected(
            "VI",
            "Vietnamese Vocabulary",
            ("Bản dịch", "Ngữ cảnh", "Gợi ý", "Mức độ quan trọng")
        ),
        "Vietnamese did not preserve both offline language roles"
    );
}
