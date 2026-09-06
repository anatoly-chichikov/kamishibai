use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};

use super::*;

/// Read the complete local request before choosing the author or IPA response.
fn request(stream: &mut TcpStream) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("local request timeout must configure");
    let mut bytes = Vec::new();
    loop {
        let mut chunk = [0u8; 8192];
        let size = stream.read(&mut chunk).expect("local request must read");
        if size == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..size]);
        let text = String::from_utf8_lossy(&bytes);
        if let Some(end) = text.find("\r\n\r\n") {
            let length = text[..end]
                .lines()
                .find_map(|line| {
                    line.to_lowercase()
                        .strip_prefix("content-length: ")
                        .map(str::to_owned)
                })
                .and_then(|value| value.parse::<usize>().ok())
                .expect("local request must carry content length");
            if bytes.len() >= end + 4 + length {
                break;
            }
        }
    }
    String::from_utf8(bytes).expect("local request must be UTF-8")
}

/// Reply with valid authored metadata or a charged but invalid IPA document.
fn response(phonetics: bool) -> String {
    let text = if phonetics {
        serde_json::json!({"pronunciation": "kanaʁ", "transcription": "   "})
    } else {
        serde_json::json!({
            "pronunciation": "kanaʁ",
            "transcription": "lə kanaʁ naʒ",
            "meaning": "a duck",
            "importance": 5,
            "source_sentence": "The duck swims.",
            "source_highlight": "duck",
            "source_hint": "A water bird in motion.",
            "source_context": "A common water bird.",
            "target_sentence": "Le canard nage.",
            "labels": {"register": "neutral", "level": "b1", "type": "statement", "approx": []}
        })
    };
    let body = serde_json::json!({
        "candidates": [{"content": {"parts": [{"text": text.to_string()}]}}],
        "usageMetadata": {
            "promptTokenCount": if phonetics { 7 } else { 100 },
            "candidatesTokenCount": if phonetics { 3 } else { 20 },
            "thoughtsTokenCount": if phonetics { 11 } else { 30 },
            "totalTokenCount": if phonetics { 21 } else { 150 }
        }
    })
    .to_string();
    format!(
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    )
}

/// Capture every dependent byte that an unsuccessful refresh must retain.
fn snapshot(cell: &Cache, visual: &Cache) -> Vec<Option<Vec<u8>>> {
    [
        cell.filepath("meta.json").expect("meta path must resolve"),
        cell.filepath(VOICE_FILE).expect("voice path must resolve"),
        visual
            .filepath(SCENE_FILE)
            .expect("scene path must resolve"),
        visual
            .filepath(ILLUSTRATION_FILE)
            .expect("picture path must resolve"),
        visual
            .path()
            .join(IMAGE_ATTEMPTS_DIRECTORY)
            .join("attempt-0001.json"),
    ]
    .iter()
    .map(fs::read)
    .map(|result| result.ok())
    .collect()
}

#[test]
fn failed_phonetic_refresh_keeps_cached_artifacts_and_both_charges() {
    let Some(root) = std::env::var_os("KAMISHIBAI_PHONETIC_REFRESH_ROOT") else {
        let home = TempDir::new().expect("refresh tempdir must exist");
        let listener = TcpListener::bind("127.0.0.1:0").expect("local listener must bind");
        listener
            .set_nonblocking(true)
            .expect("listener must be nonblocking");
        let address = listener
            .local_addr()
            .expect("listener address must resolve");
        let stopped = Arc::new(AtomicBool::new(false));
        let stop = stopped.clone();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let observed = requests.clone();
        let server = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(15);
            while !stop.load(Ordering::SeqCst) && Instant::now() < deadline {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let body = request(&mut stream);
                        let phonetics = body.contains("Verify only the two IPA fields");
                        observed.lock().expect("requests must lock").push(phonetics);
                        stream
                            .write_all(response(phonetics).as_bytes())
                            .expect("local response must write");
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("local listener failed: {error}"),
                }
            }
        });
        let name = format!(
            "{}::failed_phonetic_refresh_keeps_cached_artifacts_and_both_charges",
            module_path!()
                .strip_prefix(concat!(env!("CARGO_CRATE_NAME"), "::"))
                .unwrap_or(module_path!())
        );
        let mut child = Command::new(std::env::current_exe().expect("test binary must resolve"))
            .args([name.as_str(), "--exact", "--nocapture"])
            .env("KAMISHIBAI_PHONETIC_REFRESH_ROOT", home.path())
            .env("GEMINI_API_KEY", "offline-phonetic-key")
            .env("KAMISHIBAI_GEMINI_URL", format!("http://{address}"))
            .stdout(Stdio::null())
            .spawn()
            .expect("refresh child must spawn");
        let deadline = Instant::now() + Duration::from_secs(10);
        let succeeded = loop {
            if let Some(status) = child.try_wait().expect("child state must resolve") {
                break status.success();
            }
            if Instant::now() >= deadline {
                child.kill().expect("timed out child must stop");
                child.wait().expect("timed out child must reap");
                break false;
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        stopped.store(true, Ordering::SeqCst);
        server.join().expect("local server must finish");
        assert_eq!(
            (
                succeeded,
                requests.lock().expect("requests must lock").clone()
            ),
            (true, vec![false, true]),
            "failed second-pass refresh did not preserve the cache and account for exactly two text calls"
        );
        return;
    };
    let root = PathBuf::from(root);
    let pair = LanguagePair::new("fr", "en");
    let old = labeled_meta(
        SentenceLevel::A2,
        SentenceKind::Statement,
        AxisSet::default(),
    );
    CardMetaCache::new(&root)
        .store("canard", "a duck", &pair, &old)
        .expect("old meta must store");
    let cell = CardCell::new(&root, &pair, "canard", "a duck").cache();
    let visual = cell.visual(visual_revision()).expect("visual must resolve");
    seed_refresh_files(&cell, &visual);
    fs::remove_file(
        cell.filepath(META_COST_FILE)
            .expect("cost path must resolve"),
    )
    .expect("unparsed cost fixture must clear");
    let before = snapshot(&cell, &visual);
    let production =
        MetadataProduction::new(root, GeminiAccess::console(), CostAccounting::new(None));
    let labels = SentenceLabelSelection::empty().choosing(SentenceAxis::Level, 2);
    let (result, cost) = production
        .generate("canard", "a duck", &pair, Some(&labels), None)
        .into_parts();
    let stored = load_cost_record(&cell, Artifact::Meta).expect("both costs must decode");
    assert_eq!(
        (
            result.is_err(),
            snapshot(&cell, &visual) == before,
            cost,
            stored
        ),
        (
            true,
            true,
            Some(GenerationCost::from_nanos(320_250)),
            Some(CostRecord::new(
                "gemini-3.8-flash",
                2,
                107,
                64,
                171,
                GenerationCost::from_nanos(320_250)
            ))
        ),
        "invalid IPA replaced an intact card, removed its dependents, or lost one of the two billed responses"
    );
}
