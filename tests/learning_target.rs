//! Offline process tests for explicit and detected understanding targets.

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use kamishibai::session::MAX_INTAKE_WORDS;

use assert_cmd::Command;
use serde_json::{Value, json};
use tempfile::TempDir;

fn cli(data: &Path, cache: &Path, gemini: &str) -> Command {
    let mut command = Command::cargo_bin("kamishibai").expect("the binary must build");
    command
        .env("KAMISHIBAI_DATA", data)
        .env("KAMISHIBAI_CACHE", cache)
        .env("KAMISHIBAI_GEMINI_URL", gemini)
        .env("GEMINI_API_KEY", "offline-dummy-key");
    command
}

fn gemini(target: &str) -> (String, Arc<AtomicUsize>, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("the Gemini stub must bind");
    let port = listener
        .local_addr()
        .expect("the Gemini stub must have an address")
        .port();
    let calls = Arc::new(AtomicUsize::new(0));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let observed_calls = calls.clone();
    let observed_requests = requests.clone();
    let target = String::from(target);
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut stream = stream;
            observed_calls.fetch_add(1, Ordering::SeqCst);
            observed_requests
                .lock()
                .expect("request log must lock")
                .push(request(&mut stream));
            respond(&mut stream, target.as_str());
        }
    });
    (format!("http://127.0.0.1:{port}"), calls, requests)
}

fn request(stream: &mut TcpStream) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("request timeout must configure");
    let mut bytes = Vec::new();
    loop {
        let mut chunk = [0u8; 8192];
        let size = stream.read(&mut chunk).expect("request must read");
        if size == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..size]);
        if complete(&bytes) {
            break;
        }
    }
    String::from_utf8(bytes).expect("request must be UTF-8")
}

fn complete(bytes: &[u8]) -> bool {
    let text = String::from_utf8_lossy(bytes);
    let Some(header) = text.find("\r\n\r\n") else {
        return false;
    };
    let length = text[..header]
        .lines()
        .find_map(|line| {
            line.strip_prefix("content-length: ")
                .or_else(|| line.strip_prefix("Content-Length: "))
        })
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    bytes.len() >= header + 4 + length
}

fn respond(stream: &mut TcpStream, target: &str) {
    let intake = json!({
        "target_lang": target,
        "items": [{
            "term": "chat",
            "senses": [{"understanding": "Сущ. «кот», домашнее животное.", "tag": null}],
            "selected": 0,
            "ok": true
        }]
    });
    let body = json!({
        "candidates": [{
            "content": {"parts": [{"text": intake.to_string()}]}
        }]
    })
    .to_string();
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(response.as_bytes())
        .expect("response must write");
}

fn empty(path: &Path) -> bool {
    fs::read_dir(path)
        .expect("temporary root must be readable")
        .next()
        .is_none()
}

struct ScriptedGemini {
    address: String,
    requests: Arc<Mutex<Vec<String>>>,
    halted: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl ScriptedGemini {
    fn new(
        address: String,
        requests: Arc<Mutex<Vec<String>>>,
        halted: Arc<AtomicBool>,
        worker: Option<JoinHandle<()>>,
    ) -> Self {
        Self {
            address,
            requests,
            halted,
            worker,
        }
    }
}

impl Drop for ScriptedGemini {
    fn drop(&mut self) {
        self.halted.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            worker.join().expect("the Gemini stub must stop cleanly");
        }
    }
}

struct PendingRequest {
    stream: TcpStream,
    bytes: Vec<u8>,
    deadline: Instant,
}

impl PendingRequest {
    fn new(stream: TcpStream, bytes: Vec<u8>, deadline: Instant) -> Self {
        Self {
            stream,
            bytes,
            deadline,
        }
    }

    fn read(&mut self) -> RequestProgress {
        if Instant::now() >= self.deadline {
            return RequestProgress::Closed;
        }
        let mut chunk = [0u8; 8192];
        match self.stream.read(&mut chunk) {
            Ok(0) => RequestProgress::Closed,
            Ok(size) => {
                self.bytes.extend_from_slice(&chunk[..size]);
                if complete(&self.bytes) {
                    RequestProgress::Complete(
                        String::from_utf8(std::mem::take(&mut self.bytes))
                            .expect("the complete request must be UTF-8"),
                    )
                } else {
                    RequestProgress::Waiting
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                ) =>
            {
                RequestProgress::Waiting
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted
                ) =>
            {
                RequestProgress::Closed
            }
            Err(error) => panic!("the scripted request could not read: {error}"),
        }
    }
}

enum RequestProgress {
    Waiting,
    Closed,
    Complete(String),
}

fn scripted_gemini(replies: Vec<Value>) -> ScriptedGemini {
    let listener = TcpListener::bind("127.0.0.1:0").expect("the Gemini stub must bind");
    let address = format!(
        "http://{}",
        listener
            .local_addr()
            .expect("the stub must have an address")
    );
    listener
        .set_nonblocking(true)
        .expect("the stub must accept without blocking shutdown");
    let requests = Arc::new(Mutex::new(Vec::new()));
    let observed = requests.clone();
    let halted = Arc::new(AtomicBool::new(false));
    let stopping = halted.clone();
    let worker = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(60);
        let mut replies = replies.into_iter();
        let mut pending = Vec::new();
        while !stopping.load(Ordering::SeqCst) && Instant::now() < deadline {
            match listener.accept() {
                Ok((stream, _)) => {
                    stream
                        .set_nonblocking(true)
                        .expect("accepted requests must not block each other");
                    pending.push(PendingRequest::new(
                        stream,
                        Vec::new(),
                        Instant::now() + Duration::from_secs(20),
                    ));
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(error) => panic!("the Gemini stub could not accept: {error}"),
            }
            pending.retain_mut(|request| match request.read() {
                RequestProgress::Waiting => true,
                RequestProgress::Closed => false,
                RequestProgress::Complete(body) => {
                    observed
                        .lock()
                        .expect("the request log must lock")
                        .push(body);
                    request
                        .stream
                        .set_nonblocking(false)
                        .expect("the completed request must permit a bounded response");
                    scripted_response(
                        &mut request.stream,
                        replies.next().unwrap_or_else(|| json!({})),
                    );
                    false
                }
            });
            std::thread::sleep(Duration::from_millis(5));
        }
    });
    ScriptedGemini::new(address, requests, halted, Some(worker))
}

fn scripted_response(stream: &mut TcpStream, reply: Value) {
    let body = json!({
        "candidates": [{"content": {"parts": [{"text": reply.to_string()}]}}]
    })
    .to_string();
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .expect("the response timeout must configure");
    stream
        .write_all(response.as_bytes())
        .expect("the scripted response must write");
}

/// An idle TCP connection cannot occupy the scripted provider or consume its next reply.
#[test]
fn an_idle_connection_cannot_steal_the_scripted_response() {
    let mut gemini = scripted_gemini(vec![json!({"marker": "served"})]);
    let address = gemini
        .address
        .strip_prefix("http://")
        .expect("the stub address must be HTTP");
    let idle = TcpStream::connect(address).expect("the idle connection must connect");
    let mut active = TcpStream::connect(address).expect("the active connection must connect");
    active
        .set_read_timeout(Some(Duration::from_secs(3)))
        .expect("the active connection must have a bounded read");
    active
        .write_all(b"POST / HTTP/1.1\r\nHost: localhost\r\nContent-Length: 2\r\n\r\n{}")
        .expect("the active request must write");
    let mut reply = String::new();
    let completed = active.read_to_string(&mut reply).is_ok();
    drop(idle);
    drop(active);
    gemini.halted.store(true, Ordering::SeqCst);
    let stopped = gemini
        .worker
        .take()
        .expect("the scripted provider must own its worker")
        .join()
        .is_ok();
    assert_eq!(
        (completed, reply.contains("served"), stopped),
        (true, true, true),
        "an idle connection blocked the active request, consumed its reply, or killed the provider"
    );
}

fn mixed_intake() -> Value {
    json!({
        "target_lang": "EN",
        "items": [
            {
                "term": "лук",
                "senses": [
                    {"translation": "onion", "understanding": "Овощ с острым вкусом.", "tag": null},
                    {"translation": "bow", "understanding": "Оружие для стрельбы стрелами.", "tag": null}
                ],
                "selected": 0,
                "ok": true
            },
            {
                "term": "serendipity",
                "senses": [{"understanding": "Счастливая случайность.", "tag": null}],
                "selected": 0,
                "ok": true
            }
        ]
    })
}

fn mixed_session(data: &Path, cache: &Path, out: &Path, gemini: &str, id: &str) -> Value {
    document(cli(data, cache, gemini).args([
        "new",
        "--word",
        "лук",
        "--word",
        "serendipity",
        "--known",
        "RU",
        "--learning",
        "EN",
        "--id",
        id,
        "--out",
        out.to_str().expect("the output path must be UTF-8"),
        "--json",
    ]))
}

fn detected_mixed_intake() -> Value {
    json!({
        "target_lang": "EN",
        "items": [
            {"term": "cat", "senses": [{"understanding": "Домашняя кошка.", "tag": null}], "selected": 0, "ok": true},
            {"term": "dog", "senses": [{"understanding": "Домашняя собака.", "tag": null}], "selected": 0, "ok": true},
            {"term": "war", "senses": [{"understanding": "Вооруженный конфликт.", "tag": null}], "selected": 0, "ok": true},
            {"term": "как-бы", "senses": [{"translation": "sort of", "understanding": "Разговорное смягчение утверждения.", "tag": null}], "selected": 0, "ok": true},
            {"term": "сумка", "senses": [{"translation": "bag", "understanding": "Вещь для переноски предметов.", "tag": null}], "selected": 0, "ok": true},
            {"term": "дорога", "senses": [
                {"translation": "road", "understanding": "Путь для движения транспорта.", "tag": null},
                {"translation": "journey", "understanding": "Поездка из одного места в другое.", "tag": null}
            ], "selected": 0, "ok": true}
        ]
    })
}

fn detected_mixed_session(data: &Path, cache: &Path, out: &Path, gemini: &str, id: &str) -> Value {
    document(cli(data, cache, gemini).args([
        "new",
        "--word",
        "cat",
        "--word",
        "dog",
        "--word",
        "war",
        "--word",
        "как-бы",
        "--word",
        "сумка",
        "--word",
        "дорога",
        "--known",
        "RU",
        "--id",
        id,
        "--out",
        out.to_str().expect("the output path must be UTF-8"),
        "--json",
    ]))
}

fn document(command: &mut Command) -> Value {
    let output = command
        .timeout(Duration::from_secs(20))
        .output()
        .expect("the bounded console command must finish");
    if !output.status.success() {
        panic!(
            "the console command failed with {:?}: {} {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    serde_json::from_slice(&output.stdout).expect("the console document must decode")
}

fn stored_session(cache: &Path, id: &str) -> Value {
    serde_json::from_slice(
        &fs::read(cache.join("sessions").join(id).join("session.json"))
            .expect("the session record must persist"),
    )
    .expect("the persisted session must decode")
}

/// Mixed screenshot input detects English and preserves Russian translations across processes.
#[test]
fn mixed_input_cannot_require_an_explicit_target_when_english_is_detected() {
    let data = TempDir::new().expect("the data directory must exist");
    let cache = TempDir::new().expect("the cache directory must exist");
    let out = TempDir::new().expect("the output directory must exist");
    let gemini = scripted_gemini(vec![detected_mixed_intake()]);
    let initial = detected_mixed_session(
        data.path(),
        cache.path(),
        out.path(),
        &gemini.address,
        "detected-mixed",
    );
    let repeated = detected_mixed_session(
        data.path(),
        cache.path(),
        out.path(),
        &gemini.address,
        "detected-cached",
    );
    let stored = stored_session(cache.path(), "detected-cached");
    assert_eq!(
        json!({
            "pair": initial["pair"],
            "terms": initial["candidates"]["items"].as_array().expect("candidate items must be an array").iter().map(|item| &item["term"]).collect::<Vec<_>>(),
            "translations": initial["candidates"]["items"].as_array().expect("candidate items must be an array").iter().map(|item| &item["senses"][0]["translation"]).collect::<Vec<_>>(),
            "unchanged": initial["candidates"] == repeated["candidates"],
            "stored_alternative": stored["candidates"][5]["senses"][1]["translation"],
            "calls": gemini.requests.lock().expect("the request log must lock").len()
        }),
        json!({
            "pair": {"known": "RU", "learning": "EN"},
            "terms": ["cat", "dog", "war", "как-бы", "сумка", "дорога"],
            "translations": [null, null, null, "sort of", "bag", "road"],
            "unchanged": true,
            "stored_alternative": "journey",
            "calls": 1
        }),
        "mixed English and Russian input lost automatic target detection, translations, or cache reuse"
    );
}

/// A selected translation enters production after the same intake detects its target.
#[test]
fn an_autodetected_reverse_card_cannot_generate_from_the_original_russian_term() {
    let data = TempDir::new().expect("the data directory must exist");
    let cache = TempDir::new().expect("the cache directory must exist");
    let out = TempDir::new().expect("the output directory must exist");
    let gemini = scripted_gemini(vec![detected_mixed_intake()]);
    detected_mixed_session(
        data.path(),
        cache.path(),
        out.path(),
        &gemini.address,
        "detected-committed",
    );
    document(cli(data.path(), cache.path(), &gemini.address).args([
        "select",
        "detected-committed",
        "--card",
        "дорога",
        "--sense",
        "2",
        "--json",
    ]));
    for term in ["cat", "dog", "war", "как-бы", "сумка"] {
        document(cli(data.path(), cache.path(), &gemini.address).args([
            "exclude",
            "detected-committed",
            "--card",
            term,
            "--json",
        ]));
    }
    let output = cli(data.path(), cache.path(), &gemini.address)
        .args(["generate", "detected-committed", "--wait", "--json"])
        .timeout(Duration::from_secs(20))
        .output()
        .expect("generation against the failing stub must stop");
    let stored = stored_session(cache.path(), "detected-committed");
    let status = document(cli(data.path(), cache.path(), &gemini.address).args([
        "status",
        "detected-committed",
        "--json",
    ]));
    assert_eq!(
        (
            output.status.code().is_some(),
            stored["drafts"].as_array().map(Vec::len),
            stored["drafts"][0]["term"].as_str(),
            stored["drafts"][0]["understanding"].as_str(),
            status["phase"].as_str(),
            status["cards"]["items"][0]["term"].as_str()
        ),
        (
            true,
            Some(1),
            Some("journey"),
            Some("Поездка из одного места в другое."),
            Some("failed"),
            Some("journey")
        ),
        "automatic target detection lost the selected English expression before card production"
    );
}

/// Known-language-only input needs a target when the provider reports no language evidence.
#[test]
fn russian_only_input_cannot_silently_choose_its_learning_language() {
    let data = TempDir::new().expect("the data directory must exist");
    let cache = TempDir::new().expect("the cache directory must exist");
    let out = TempDir::new().expect("the output directory must exist");
    let gemini = scripted_gemini(vec![json!({
        "target_lang": "",
        "needs_learning_language": true,
        "items": []
    })]);
    let output = cli(data.path(), cache.path(), &gemini.address)
        .args([
            "new",
            "--word",
            "сумка",
            "--known",
            "RU",
            "--id",
            "invented-target",
            "--out",
            out.path().to_str().expect("the output path must be UTF-8"),
            "--json",
        ])
        .timeout(Duration::from_secs(20))
        .output()
        .expect("the bounded console command must finish");
    assert_eq!(
        (
            output.status.success(),
            cache
                .path()
                .join("sessions/invented-target/session.json")
                .exists(),
            cache.path().join("understanding/RU-EN").exists(),
            gemini
                .requests
                .lock()
                .expect("the request log must lock")
                .len()
        ),
        (false, false, false, 1),
        "known-language-only input ignored the missing target or persisted an ambiguous understanding"
    );
}

/// A chosen target makes a Russian-only batch sufficient for English card preparation.
#[test]
fn russian_only_input_cannot_require_an_english_word_when_the_target_is_explicit() {
    let data = TempDir::new().expect("the data directory must exist");
    let cache = TempDir::new().expect("the cache directory must exist");
    let out = TempDir::new().expect("the output directory must exist");
    let gemini = scripted_gemini(vec![json!({
        "target_lang": "EN",
        "items": [{
            "term": "сумка",
            "senses": [{"translation": "bag", "understanding": "Вещь для переноски предметов.", "tag": null}],
            "selected": 0,
            "ok": true
        }]
    })]);
    let created = document(cli(data.path(), cache.path(), &gemini.address).args([
        "new",
        "--word",
        "сумка",
        "--known",
        "RU",
        "--learning",
        "EN",
        "--id",
        "russian-explicit",
        "--out",
        out.path().to_str().expect("the output path must be UTF-8"),
        "--json",
    ]));
    assert_eq!(
        json!({
            "pair": created["pair"],
            "candidates": created["candidates"]["items"],
            "calls": gemini.requests.lock().expect("the request log must lock").len()
        }),
        json!({
            "pair": {"known": "RU", "learning": "EN"},
            "candidates": [{
                "term": "сумка",
                "included": true,
                "senses": [{"translation": "bag", "understanding": "Вещь для переноски предметов.", "selected": true}]
            }],
            "calls": 1
        }),
        "an explicit English target failed to translate a Russian-only batch"
    );
}

/// Forward and reverse inputs retain their meanings after a second process reuses intake.
#[test]
fn reverse_and_forward_inputs_cannot_lose_their_senses_when_the_cache_is_reused() {
    let data = TempDir::new().expect("the data directory must exist");
    let cache = TempDir::new().expect("the cache directory must exist");
    let out = TempDir::new().expect("the output directory must exist");
    let gemini = scripted_gemini(vec![mixed_intake()]);
    let initial = mixed_session(
        data.path(),
        cache.path(),
        out.path(),
        &gemini.address,
        "first",
    );
    let repeated = mixed_session(
        data.path(),
        cache.path(),
        out.path(),
        &gemini.address,
        "cached",
    );
    let stored = stored_session(cache.path(), "cached");
    assert_eq!(
        json!({
            "pair": initial["pair"],
            "candidates": repeated["candidates"]["items"],
            "unchanged": initial["candidates"] == repeated["candidates"],
            "stored_translation": stored["candidates"][0]["senses"][1]["translation"],
            "calls": gemini.requests.lock().expect("the request log must lock").len()
        }),
        json!({
            "pair": {"known": "RU", "learning": "EN"},
            "candidates": [
                {
                    "term": "лук",
                    "included": true,
                    "senses": [
                        {"translation": "onion", "understanding": "Овощ с острым вкусом.", "selected": true},
                        {"translation": "bow", "understanding": "Оружие для стрельбы стрелами.", "selected": false}
                    ]
                },
                {
                    "term": "serendipity",
                    "included": true,
                    "senses": [{"understanding": "Счастливая случайность.", "selected": true}]
                }
            ],
            "unchanged": true,
            "stored_translation": "bow",
            "calls": 1
        }),
        "reverse translation, default meaning, forward shape, or cache reuse was lost between processes"
    );
}

/// A reverse candidate remains addressable by what the learner originally typed.
#[test]
fn a_selected_reverse_meaning_cannot_revert_when_status_reloads_the_session() {
    let data = TempDir::new().expect("the data directory must exist");
    let cache = TempDir::new().expect("the cache directory must exist");
    let out = TempDir::new().expect("the output directory must exist");
    let gemini = scripted_gemini(vec![mixed_intake()]);
    mixed_session(
        data.path(),
        cache.path(),
        out.path(),
        &gemini.address,
        "selected",
    );
    document(cli(data.path(), cache.path(), &gemini.address).args([
        "select", "selected", "--card", "лук", "--sense", "2", "--json",
    ]));
    let status = document(
        cli(data.path(), cache.path(), &gemini.address).args(["status", "selected", "--json"]),
    );
    assert_eq!(
        (
            status["candidates"]["items"][0]["term"].as_str(),
            status["candidates"]["items"][0]["senses"][0]["selected"].as_bool(),
            status["candidates"]["items"][0]["senses"][1]["selected"].as_bool(),
            status["candidates"]["items"][0]["senses"][1]["translation"].as_str(),
            gemini
                .requests
                .lock()
                .expect("the request log must lock")
                .len()
        ),
        (Some("лук"), Some(false), Some(true), Some("bow"), 1),
        "selecting a reverse meaning by the original input lost its translation or selection on reload"
    );
}

/// Committing a reverse selection sends its English expression into card production.
#[test]
fn a_reverse_card_cannot_generate_from_the_original_known_language_term() {
    let data = TempDir::new().expect("the data directory must exist");
    let cache = TempDir::new().expect("the cache directory must exist");
    let out = TempDir::new().expect("the output directory must exist");
    let gemini = scripted_gemini(vec![mixed_intake()]);
    mixed_session(
        data.path(),
        cache.path(),
        out.path(),
        &gemini.address,
        "committed",
    );
    document(cli(data.path(), cache.path(), &gemini.address).args([
        "select",
        "committed",
        "--card",
        "лук",
        "--sense",
        "2",
        "--json",
    ]));
    document(cli(data.path(), cache.path(), &gemini.address).args([
        "exclude",
        "committed",
        "--card",
        "serendipity",
        "--json",
    ]));
    let output = cli(data.path(), cache.path(), &gemini.address)
        .args(["generate", "committed", "--wait", "--json"])
        .timeout(Duration::from_secs(20))
        .output()
        .expect("generation against the failing stub must stop");
    let stored = stored_session(cache.path(), "committed");
    let status = document(cli(data.path(), cache.path(), &gemini.address).args([
        "status",
        "committed",
        "--json",
    ]));
    assert_eq!(
        (
            output.status.code().is_some(),
            stored["drafts"].as_array().map(Vec::len),
            stored["drafts"][0]["term"].as_str(),
            stored["drafts"][0]["understanding"].as_str(),
            status["phase"].as_str(),
            status["cards"]["items"][0]["term"].as_str()
        ),
        (
            true,
            Some(1),
            Some("bow"),
            Some("Оружие для стрельбы стрелами."),
            Some("failed"),
            Some("bow")
        ),
        "the committed reverse card used the known-language input or lost its selected English meaning"
    );
}

/// Adding a meaning to a reverse candidate preserves its independently translated expression.
#[test]
fn correcting_a_reverse_candidate_cannot_discard_its_added_translation() {
    let data = TempDir::new().expect("the data directory must exist");
    let cache = TempDir::new().expect("the cache directory must exist");
    let out = TempDir::new().expect("the output directory must exist");
    let correction = json!({
        "senses": [{"translation": "look", "understanding": "Подобранный образ в одежде.", "tag": "мода"}],
        "message": null
    });
    let gemini = scripted_gemini(vec![mixed_intake(), correction]);
    mixed_session(
        data.path(),
        cache.path(),
        out.path(),
        &gemini.address,
        "corrected",
    );
    document(cli(data.path(), cache.path(), &gemini.address).args([
        "correct",
        "corrected",
        "--card",
        "лук",
        "--note",
        "Еще образ в одежде",
        "--json",
    ]));
    let status = document(cli(data.path(), cache.path(), &gemini.address).args([
        "status",
        "corrected",
        "--json",
    ]));
    let stored = stored_session(cache.path(), "corrected");
    assert_eq!(
        json!({
            "senses": status["candidates"]["items"][0]["senses"],
            "stored_translation": stored["candidates"][0]["senses"][2]["translation"],
            "calls": gemini.requests.lock().expect("the request log must lock").len()
        }),
        json!({
            "senses": [
                {"translation": "onion", "understanding": "Овощ с острым вкусом.", "selected": true},
                {"translation": "bow", "understanding": "Оружие для стрельбы стрелами.", "selected": false},
                {"translation": "look", "understanding": "Подобранный образ в одежде.", "tag": "мода", "selected": true}
            ],
            "stored_translation": "look",
            "calls": 2
        }),
        "correction discarded a reverse translation or failed to retain the existing and newly selected meanings"
    );
}

/// Invalid explicit languages fail before credentials, network, cache, or sessions.
#[test]
fn invalid_learning_fails_before_any_external_or_persistent_work() {
    let data = TempDir::new().expect("data tempdir must be created");
    let cache = TempDir::new().expect("cache tempdir must be created");
    let out = TempDir::new().expect("output tempdir must be created");
    let (gemini, calls, _) = gemini("FR");
    let output = cli(data.path(), cache.path(), gemini.as_str())
        .env_remove("GEMINI_API_KEY")
        .args([
            "new",
            "--word",
            "chat",
            "--known",
            "RU",
            "--learning",
            "ZZ",
            "--id",
            "rejected",
            "--out",
            out.path().to_str().expect("output path must be UTF-8"),
            "--json",
        ])
        .output()
        .expect("invalid new command must run");
    let document: Value =
        serde_json::from_slice(output.stdout.as_slice()).expect("stdout must be JSON");
    assert_eq!(
        (
            output.status.code(),
            document["error"]["code"].as_str(),
            document["error"]["exit"].as_u64(),
            document["error"]["hint"]
                .as_str()
                .is_some_and(|hint| hint.contains("EN, ZH, ES, JA, FR, DE, KO, RU, IT, PT, HI, AR, TR, PL, UK, ID, VI, TH, EL, HE, NL, CS")),
            calls.load(Ordering::SeqCst),
            empty(data.path()),
            empty(cache.path()),
        ),
        (Some(2), Some("usage"), Some(2), true, 0, true, true),
        "invalid --learning omitted supported codes or reached credentials, Gemini, cache, or session creation"
    );
}

/// Lowercase explicit input is canonicalised and sent as a mandatory target.
#[test]
fn lowercase_explicit_learning_controls_understanding_and_session_identity() {
    let data = TempDir::new().expect("data tempdir must be created");
    let cache = TempDir::new().expect("cache tempdir must be created");
    let out = TempDir::new().expect("output tempdir must be created");
    let (gemini, calls, requests) = gemini("FR");
    let output = cli(data.path(), cache.path(), gemini.as_str())
        .args([
            "new",
            "--word",
            "chat",
            "--known",
            "ru",
            "--learning",
            "fr",
            "--id",
            "lowercase",
            "--out",
            out.path().to_str().expect("output path must be UTF-8"),
            "--json",
        ])
        .output()
        .expect("explicit new command must run");
    let document: Value =
        serde_json::from_slice(output.stdout.as_slice()).expect("stdout must be JSON");
    let request = requests
        .lock()
        .expect("request log must lock")
        .first()
        .cloned()
        .expect("Gemini request must be recorded");
    assert_eq!(
        (
            output.status.success(),
            calls.load(Ordering::SeqCst),
            document["pair"]["known"].as_str(),
            document["pair"]["learning"].as_str(),
            request.contains("The required target language is FR (French)"),
            cache.path().join("understanding/RU-FR").is_dir(),
            cache
                .path()
                .join("sessions/lowercase/session.json")
                .is_file(),
        ),
        (true, 1, Some("RU"), Some("FR"), true, true, true),
        "lowercase explicit learning was not canonicalised through prompt, cache, and session"
    );
}

/// A provider target mismatch cannot leave a relabelled session or cache entry.
#[test]
fn provider_target_mismatch_leaves_no_session_or_understanding_entry() {
    let data = TempDir::new().expect("data tempdir must be created");
    let cache = TempDir::new().expect("cache tempdir must be created");
    let out = TempDir::new().expect("output tempdir must be created");
    let (gemini, calls, _) = gemini("EN");
    let output = cli(data.path(), cache.path(), gemini.as_str())
        .args([
            "new",
            "--word",
            "chat",
            "--known",
            "RU",
            "--learning",
            "FR",
            "--id",
            "mismatch",
            "--out",
            out.path().to_str().expect("output path must be UTF-8"),
            "--json",
        ])
        .output()
        .expect("mismatched new command must run");
    assert_eq!(
        (
            output.status.code(),
            calls.load(Ordering::SeqCst),
            cache.path().join("sessions/mismatch/session.json").exists(),
            cache.path().join("understanding/RU-FR").exists(),
        ),
        (Some(1), 1, false, false),
        "provider target mismatch created a relabelled session or understanding cache"
    );
}

/// Omitting the target keeps the existing model-driven language detection.
#[test]
fn omitted_learning_keeps_autodetection() {
    let data = TempDir::new().expect("data tempdir must be created");
    let cache = TempDir::new().expect("cache tempdir must be created");
    let out = TempDir::new().expect("output tempdir must be created");
    let (gemini, calls, requests) = gemini("EN");
    let output = cli(data.path(), cache.path(), gemini.as_str())
        .args([
            "new",
            "--word",
            "chat",
            "--known",
            "RU",
            "--id",
            "detected",
            "--out",
            out.path().to_str().expect("output path must be UTF-8"),
            "--json",
        ])
        .output()
        .expect("autodetected new command must run");
    let document: Value =
        serde_json::from_slice(output.stdout.as_slice()).expect("stdout must be JSON");
    let request = requests
        .lock()
        .expect("request log must lock")
        .first()
        .cloned()
        .expect("Gemini request must be recorded");
    assert_eq!(
        (
            output.status.success(),
            calls.load(Ordering::SeqCst),
            document["pair"]["learning"].as_str(),
            request.contains("Choose exactly one dominant target language"),
        ),
        (true, 1, Some("EN"), true),
        "omitting --learning no longer uses the existing autodetection contract"
    );
}

/// An oversized word list fails before credentials, network, cache, or sessions.
#[test]
fn an_oversized_word_list_fails_before_any_external_or_persistent_work() {
    let data = TempDir::new().expect("data tempdir must be created");
    let cache = TempDir::new().expect("cache tempdir must be created");
    let out = TempDir::new().expect("output tempdir must be created");
    let (gemini, calls, _) = gemini("FR");
    let mut command = cli(data.path(), cache.path(), gemini.as_str());
    command.env_remove("GEMINI_API_KEY").arg("new");
    for index in 0..=MAX_INTAKE_WORDS {
        command.args(["--word", &format!("word-{index:03}")]);
    }
    let output = command
        .args([
            "--known",
            "RU",
            "--out",
            out.path().to_str().expect("output path must be UTF-8"),
            "--json",
        ])
        .output()
        .expect("oversized new command must run");
    let document: Value =
        serde_json::from_slice(output.stdout.as_slice()).expect("stdout must be JSON");
    assert_eq!(
        (
            output.status.code(),
            document["error"]["code"].as_str(),
            document["error"]["exit"].as_u64(),
            document["error"]["hint"]
                .as_str()
                .is_some_and(|hint| hint.contains(&MAX_INTAKE_WORDS.to_string())),
            calls.load(Ordering::SeqCst),
            empty(data.path()),
            empty(cache.path()),
        ),
        (Some(2), Some("usage"), Some(2), true, 0, true, true),
        "an oversized word list reached credentials, Gemini, the cache, or session creation"
    );
}
