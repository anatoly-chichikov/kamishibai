//! Exercise the built-in client's public picture customization.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use anyhow::Result;
use kamishibai::gemini::{GeminiClient, Transport, TransportResponse};
use kamishibai::generation::{SceneMode, SceneSource};
use kamishibai::models::{Look, MangaInk, Resend, SoftWorld};
use serde_json::{Value, json};

struct PaperWorld;

impl SoftWorld for PaperWorld {
    fn name(&self) -> &str {
        "paper-v1"
    }

    fn brief(&self) -> &str {
        "SOFT WORLD\nOnly folded paper animals appear."
    }

    fn admit(&self, _scene: &Value, _prompt: &str) -> Result<()> {
        Ok(())
    }
}

struct SceneTransport {
    requests: Rc<RefCell<Vec<String>>>,
}

impl Transport for SceneTransport {
    fn get(&self, _url: &str, _key: &str) -> Result<TransportResponse> {
        anyhow::bail!("unexpected credential request")
    }

    fn post(&self, _url: &str, _key: &str, body: &str) -> Result<TransportResponse> {
        let response = match self.requests.borrow().len() {
            0 => super::gemini::scene_features(),
            1 => super::gemini::dynamic_scene(),
            _ => anyhow::bail!("unexpected extra scene request"),
        };
        self.requests.borrow_mut().push(body.to_owned());
        Ok(TransportResponse {
            status: 200,
            body: json!({"candidates":[{"content":{"parts":[{"text":response.to_string()}]}}]})
                .to_string(),
        })
    }
}

fn requests(look: Option<Look>, mode: SceneMode) -> Vec<String> {
    let requests = Rc::new(RefCell::new(Vec::new()));
    let client = GeminiClient::new(
        "offline",
        SceneTransport {
            requests: requests.clone(),
        },
    );
    let client = match look {
        Some(look) => client.with_look(look),
        None => client,
    };
    SceneSource::scene(
        &client,
        "English",
        "cat",
        "The cat sleeps on the windowsill",
        "en",
        0,
        mode,
    )
    .unwrap();
    requests.take()
}

fn look() -> Look {
    Look::new(
        Arc::new(MangaInk),
        Some(Arc::new(PaperWorld)),
        Resend::Automatic,
    )
}

#[test]
fn plugged_look_cannot_lose_its_world_before_the_builtin_scene_request() {
    let suffix = serde_json::to_string(&format!("\n\n{}", PaperWorld.brief())).unwrap();
    let suffix = &suffix[1..suffix.len() - 1];
    let soft = requests(Some(look()), SceneMode::Soft);
    assert_eq!(
        (
            soft.iter()
                .map(|body| body.matches(suffix).count())
                .collect::<Vec<_>>(),
            soft.iter()
                .map(|body| body.replace(suffix, ""))
                .collect::<Vec<_>>()
        ),
        (vec![1, 1], requests(None, SceneMode::Stock)),
        "the public client lost its plugged world or changed the stock request bytes"
    );
}

#[test]
fn plugged_look_cannot_change_unrequested_stock_scene_bytes() {
    assert_eq!(
        requests(Some(look()), SceneMode::Stock),
        requests(None, SceneMode::Stock),
        "plugging a world changed stock scene requests"
    );
}

#[test]
fn publication_image_style_cannot_disappear_from_the_public_path() {
    assert_eq!(
        kamishibai::publishing::IMAGE_STYLE,
        "max-width: 100%; height: auto; border-radius: 10px",
        "the public publication style changed or disappeared"
    );
}

struct Watercolor;

impl kamishibai::models::PictureStyle for Watercolor {
    fn name(&self) -> &str {
        "watercolor-v1"
    }

    fn prompt(&self, _scene: &Value) -> Result<String> {
        Ok(String::from("Watercolor animals in one framed scene"))
    }
}

fn exchange(mut stream: std::net::TcpStream, response: &str, status: u16) -> String {
    use std::io::{BufRead, Read, Write};
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(3)))
        .unwrap();
    stream
        .set_write_timeout(Some(std::time::Duration::from_secs(3)))
        .unwrap();
    let mut reader = std::io::BufReader::new(&mut stream);
    let mut length = None;
    loop {
        let mut line = String::new();
        let read = reader.read_line(&mut line).unwrap();
        if read == 0 {
            panic!("local request ended before its headers");
        }
        if line == "\r\n" {
            break;
        }
        if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            length = Some(value.trim().parse::<usize>().unwrap());
        }
    }
    let mut body = vec![0; length.expect("local request must carry its length")];
    reader.read_exact(&mut body).unwrap();
    write!(
        reader.into_inner(),
        "HTTP/1.1 {status} local\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{response}",
        response.len()
    )
    .unwrap();
    String::from_utf8(body).unwrap()
}

fn drawing(look: Option<Look>) -> (bool, Vec<Value>) {
    use kamishibai::application::CardProduction;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let endpoint = format!("http://{}/models", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut requests = Vec::new();
        while requests.len() < 3 {
            match listener.accept() {
                Ok((stream, _)) => {
                    let (status, response) = match requests.len() {
                        0 => (
                            200,
                            json!({"candidates":[{"content":{"parts":[{"text":super::gemini::scene_features().to_string()}]}}]}),
                        ),
                        1 => (
                            200,
                            json!({"candidates":[{"content":{"parts":[{"text":super::gemini::dynamic_scene().to_string()}]}}]}),
                        ),
                        _ => (
                            500,
                            json!({"error":{"code":500,"status":"INTERNAL","message":"local image probe ended"}}),
                        ),
                    };
                    requests.push(
                        serde_json::from_str::<Value>(&exchange(
                            stream,
                            &response.to_string(),
                            status,
                        ))
                        .unwrap(),
                    );
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if std::time::Instant::now() >= deadline {
                        panic!("local drawing requests exceeded their deadline");
                    }
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                Err(error) => panic!("local listener failed: {error}"),
            }
        }
        requests
    });
    let root = tempfile::tempdir().unwrap();
    let profile = kamishibai::gemini::GeminiProfile::from_models(
        &endpoint,
        kamishibai::gemini::StageModels::default(),
    )
    .unwrap();
    let client =
        GeminiClient::from_profile("offline", kamishibai::gemini::HttpTransport::new(), profile);
    let client = match look {
        Some(look) => client.with_look(look),
        None => client,
    };
    let workflow = kamishibai::gemini::workflow(
        client,
        root.path().join("cache"),
        root.path().join("output"),
        None,
    );
    let draft = kamishibai::session::CardDraft::new(
        "cat",
        "a household animal",
        kamishibai::session::LanguagePair::new("EN", "VI"),
    )
    .with_meta(
        kamishibai::session::CardMeta::new(
            "kæt",
            "kæt",
            "cat",
            5,
            "A cat sleeps",
            "cat",
            "a household animal",
            "an everyday noun",
            "The cat sleeps on the windowsill",
        ),
        None,
    );
    workflow.generate_scene_in(0, &draft).into_result().unwrap();
    let failed = workflow.generate_picture_in(0, &draft).error().is_some();
    (failed, server.join().unwrap())
}

#[test]
fn builtin_workflow_cannot_replace_a_clients_plugged_picture_style() {
    let (failed, mut plugged) = drawing(Some(Look::new(
        Arc::new(Watercolor),
        None,
        Resend::Automatic,
    )));
    let (default_failed, default) = drawing(None);
    let prompt = plugged[2]["contents"][0]["parts"][0]["text"].clone();
    plugged[2]["contents"][0]["parts"][0]["text"] =
        default[2]["contents"][0]["parts"][0]["text"].clone();
    assert_eq!(
        (failed, default_failed, prompt, plugged),
        (
            true,
            true,
            json!("Watercolor animals in one framed scene"),
            default
        ),
        "the built-in workflow lost the client's style or changed unrelated request fields"
    );
}
