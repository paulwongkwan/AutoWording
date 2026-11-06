use super::*;
use crate::ai::{anthropic, lmstudio, models, ollama, openai};
use crate::config::{Provider, ProviderSettings};
use base64::Engine as _;
use serde_json::Value;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::thread::JoinHandle;
use std::time::Duration;

// ---------------------------------------------------------------- utilities

fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .no_proxy()
        .build()
        .expect("client")
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("autowording-ai-tests")
        .join(format!("{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn write_jpeg(path: &Path, width: u32, height: u32) {
    let image = image::RgbImage::from_pixel(width, height, image::Rgb([30, 60, 90]));
    image.save(path).expect("write jpeg");
}

struct MockServer {
    url: String,
    handle: JoinHandle<String>,
}

impl MockServer {
    fn url(&self) -> &str {
        &self.url
    }

    fn finish(self) -> String {
        self.handle.join().expect("mock server thread")
    }
}

fn find_headers_end(buffer: &[u8]) -> Option<usize> {
    buffer
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|position| position + 4)
}

fn read_request(stream: &mut TcpStream) -> String {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));

    let mut buffer = Vec::new();
    let mut header_end = None;
    while header_end.is_none() {
        let mut chunk = [0u8; 1024];
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => {
                buffer.extend_from_slice(&chunk[..read]);
                header_end = find_headers_end(&buffer);
            }
            Err(_) => break,
        }
    }

    if let Some(end) = header_end {
        let head = String::from_utf8_lossy(&buffer[..end]).to_ascii_lowercase();
        let content_length = head
            .lines()
            .find_map(|line| line.strip_prefix("content-length:"))
            .and_then(|value| value.trim().parse::<usize>().ok())
            .unwrap_or(0);
        while buffer.len() < end + content_length {
            let mut chunk = [0u8; 4096];
            match stream.read(&mut chunk) {
                Ok(0) => break,
                Ok(read) => buffer.extend_from_slice(&chunk[..read]),
                Err(_) => break,
            }
        }
    }

    String::from_utf8_lossy(&buffer).to_string()
}

/// Accept a sequence of requests, replying with the matching status/body each time.
fn mock_server_sequence(responses: Vec<(&'static str, &'static str)>) -> MockServer {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
    let port = listener.local_addr().expect("local addr").port();

    let handle = std::thread::spawn(move || {
        let mut requests = Vec::new();
        for (status, body) in responses {
            let (mut stream, _) = listener.accept().expect("accept request");
            let request = read_request(&mut stream);
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
            requests.push(request);
        }
        requests.join("\n---\n")
    });

    MockServer {
        url: format!("http://127.0.0.1:{port}"),
        handle,
    }
}

/// Accept one HTTP request, capture it and reply with a fixed status/body.
fn mock_server(status: &'static str, body: &'static str) -> MockServer {
    mock_server_sequence(vec![(status, body)])
}

fn request_body(request: &str) -> Value {
    let body = request
        .split("\r\n\r\n")
        .nth(1)
        .expect("request should contain a body");
    serde_json::from_str(body).expect("request body should be JSON")
}

// ------------------------------------------------------------ image prep

#[test]
fn prepare_image_downscales_to_max_dimension_and_keeps_aspect() {
    let dir = temp_dir("prepare-large");
    let path = dir.join("large.jpg");
    write_jpeg(&path, 2000, 1000);

    let encoded = prepare_image(&path).expect("encode image");
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .expect("valid base64");

    assert_eq!(&bytes[..3], &[0xFF, 0xD8, 0xFF], "output is a JPEG");
    let decoded = image::load_from_memory(&bytes).expect("decode jpeg");
    assert_eq!((decoded.width(), decoded.height()), (1600, 800));
}

#[test]
fn prepare_image_keeps_small_images_at_original_size() {
    let dir = temp_dir("prepare-small");
    let path = dir.join("small.jpg");
    write_jpeg(&path, 64, 48);

    let encoded = prepare_image(&path).expect("encode image");
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .expect("valid base64");
    let decoded = image::load_from_memory(&bytes).expect("decode jpeg");
    assert_eq!((decoded.width(), decoded.height()), (64, 48));
}

#[test]
fn prepare_image_rejects_non_images_and_missing_files() {
    let dir = temp_dir("prepare-invalid");
    let text_path = dir.join("not-an-image.jpg");
    std::fs::write(&text_path, b"this is not a jpeg").expect("write text file");

    let error = prepare_image(&text_path).unwrap_err().to_string();
    assert!(error.contains("Could not read image"), "got: {error}");

    let missing = dir.join("missing.jpg");
    let error = prepare_image(&missing).unwrap_err().to_string();
    assert!(error.contains("Could not read image"), "got: {error}");
}

// -------------------------------------------------------------- endpoints

#[test]
fn endpoint_appends_paths_and_respects_complete_urls() {
    assert_eq!(
        endpoint("http://localhost:11434", "/api/chat"),
        "http://localhost:11434/api/chat"
    );
    assert_eq!(
        endpoint("http://localhost:11434/", "/api/chat"),
        "http://localhost:11434/api/chat"
    );
    assert_eq!(
        endpoint("http://localhost:11434/api", "/api/chat"),
        "http://localhost:11434/api/chat"
    );
    assert_eq!(
        endpoint("http://localhost:11434/api/chat", "/api/chat"),
        "http://localhost:11434/api/chat"
    );
    assert_eq!(
        endpoint("https://api.openai.com/v1/", "/chat/completions"),
        "https://api.openai.com/v1/chat/completions"
    );
    assert_eq!(endpoint("", "/messages"), "/messages");
}

#[test]
fn lmstudio_endpoint_normalises_v1() {
    assert_eq!(
        lmstudio::endpoint_for("http://localhost:1234"),
        "http://localhost:1234/v1/chat/completions"
    );
    assert_eq!(
        lmstudio::endpoint_for("http://localhost:1234/v1"),
        "http://localhost:1234/v1/chat/completions"
    );
    assert_eq!(
        lmstudio::endpoint_for("http://localhost:1234/v1/"),
        "http://localhost:1234/v1/chat/completions"
    );
    assert_eq!(
        lmstudio::endpoint_for("http://localhost:1234/v1/chat/completions"),
        "http://localhost:1234/v1/chat/completions"
    );
    assert_eq!(
        ollama::endpoint_for("http://localhost:11434"),
        "http://localhost:11434/api/chat"
    );
}

#[test]
fn snippet_collapses_whitespace_and_truncates_long_text() {
    assert_eq!(snippet("a\n\t b   c"), "a b c");
    let long = "x".repeat(500);
    let short = snippet(&long);
    assert_eq!(short.chars().count(), 401);
    assert!(short.ends_with('…'));
    assert!(!snippet("short").ends_with('…'));
}

#[tokio::test]
async fn generate_retries_when_the_first_keyword_list_is_short() {
    let first = r#"{"choices":[{"message":{"content":"Title: Short Title\nEditorial Status: Commercial\nCaption: A factual caption that is long enough for this test.\nKeywords: apple"}}]}"#;
    let second = r#"{"choices":[{"message":{"content":"Title: Short Title\nEditorial Status: Commercial\nCaption: A factual caption that is long enough for this test.\nKeywords: apple, apples, banana, cherry"}}]}"#;
    let server = mock_server_sequence(vec![("200 OK", first), ("200 OK", second)]);

    let result = generate(AiRequest {
        provider: Provider::LmStudio,
        settings: ProviderSettings {
            base_url: server.url().to_string(),
            model: "test-model".to_string(),
            api_key: String::new(),
        },
        image_base64: "QUJD".to_string(),
        context: None,
        existing: crate::metadata::ExistingMetadata::default(),
        global_prompt: String::new(),
        date: "November 6, 2026".to_string(),
        keyword_target: 3,
        title_range: (3, 5),
        caption_range: (50, 180),
    })
    .await
    .expect("generate should recover from a short keyword list");

    assert_eq!(result.keywords, vec!["apple", "banana", "cherry"]);
    let requests = server.finish();
    assert_eq!(requests.matches("POST /v1/chat/completions").count(), 2);
    assert!(requests.contains("Only 1 usable keyword was returned"));
    assert!(requests.contains("title has 2 words"));
}

#[tokio::test]
async fn generate_retries_out_of_range_title_and_caption() {
    let first = r#"{"choices":[{"message":{"content":"Title: Tiny Title\nEditorial Status: Commercial\nCaption: Too short.\nKeywords: apple, banana, cherry"}}]}"#;
    let second = r#"{"choices":[{"message":{"content":"Title: A Corrected Title\nEditorial Status: Commercial\nCaption: A corrected factual caption about the scene that is long enough to pass.\nKeywords: apple, banana, cherry"}}]}"#;
    let server = mock_server_sequence(vec![("200 OK", first), ("200 OK", second)]);

    let result = generate(AiRequest {
        provider: Provider::LmStudio,
        settings: ProviderSettings {
            base_url: server.url().to_string(),
            model: "test-model".to_string(),
            api_key: String::new(),
        },
        image_base64: "QUJD".to_string(),
        context: None,
        existing: crate::metadata::ExistingMetadata::default(),
        global_prompt: String::new(),
        date: "November 6, 2026".to_string(),
        keyword_target: 3,
        title_range: (3, 5),
        caption_range: (50, 180),
    })
    .await
    .expect("generate should retry an out-of-range title and caption");

    assert_eq!(result.title, "A Corrected Title");
    assert!(result.caption.starts_with("A corrected factual caption"));
    assert_eq!(result.keywords, vec!["apple", "banana", "cherry"]);
    let requests = server.finish();
    assert_eq!(requests.matches("POST /v1/chat/completions").count(), 2);
    assert!(requests.contains("Rewrite the title to 3–5 words"));
    assert!(requests.contains("Rewrite the caption to 50–180 characters"));
    assert!(requests.contains("Keep the keywords unchanged"));
}

// ----------------------------------------------------------------- Ollama

#[tokio::test]
async fn ollama_sends_image_and_parses_message_content() {
    let server = mock_server(
        "200 OK",
        r#"{"message":{"role":"assistant","content":"Title: Test Image"}}"#,
    );

    let reply = ollama::chat(
        &http_client(),
        server.url(),
        "llava",
        "system prompt",
        "user prompt",
        "QUJD",
    )
    .await
    .expect("ollama reply");
    assert_eq!(reply, "Title: Test Image");

    let request = server.finish();
    assert!(request.starts_with("POST /api/chat HTTP/1.1"));
    let body = request_body(&request);
    assert_eq!(body["model"], "llava");
    assert_eq!(body["stream"], false);
    assert_eq!(body["messages"][0]["role"], "system");
    assert_eq!(body["messages"][0]["content"], "system prompt");
    assert_eq!(body["messages"][1]["content"], "user prompt");
    assert_eq!(body["messages"][1]["images"][0], "QUJD");
}

#[tokio::test]
async fn ollama_surfaces_error_field_and_http_failures() {
    let server = mock_server("200 OK", r#"{"error":"model 'llava' not found"}"#);
    let error = ollama::chat(&http_client(), server.url(), "llava", "s", "u", "QUJD")
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("model 'llava' not found"), "got: {error}");
    server.finish();

    let server = mock_server("500 Internal Server Error", r#"{"error":"boom"}"#);
    let error = ollama::chat(&http_client(), server.url(), "llava", "s", "u", "QUJD")
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("500"), "got: {error}");
    assert!(error.contains("boom"), "got: {error}");
    server.finish();
}

#[tokio::test]
async fn ollama_rejects_malformed_responses() {
    let server = mock_server("200 OK", "not json at all");
    let error = ollama::chat(&http_client(), server.url(), "llava", "s", "u", "QUJD")
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("Unexpected Ollama response"), "got: {error}");
    server.finish();

    let server = mock_server("200 OK", "{}");
    let error = ollama::chat(&http_client(), server.url(), "llava", "s", "u", "QUJD")
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("no message content"), "got: {error}");
    server.finish();
}

// --------------------------------------------------------------- LM Studio

#[tokio::test]
async fn lmstudio_uses_v1_chat_endpoint_without_auth() {
    let server = mock_server(
        "200 OK",
        r#"{"choices":[{"message":{"content":"Title: Local Model"}}]}"#,
    );
    let base = format!("{}/v1", server.url());

    let reply = lmstudio::chat(&http_client(), &base, "local-model", "s", "u", "QUJD")
        .await
        .expect("lm studio reply");
    assert_eq!(reply, "Title: Local Model");

    let request = server.finish();
    let lower = request.to_ascii_lowercase();
    assert!(request.starts_with("POST /v1/chat/completions"));
    assert!(!lower.contains("authorization:"), "no auth header expected");
    let body = request_body(&request);
    assert_eq!(body["model"], "local-model");
    assert_eq!(
        body["messages"][1]["content"][1]["image_url"]["url"],
        "data:image/jpeg;base64,QUJD"
    );
}

#[tokio::test]
async fn lmstudio_reports_http_and_parse_errors() {
    let server = mock_server("400 Bad Request", r#"{"error":"bad request"}"#);
    let error = lmstudio::chat(&http_client(), server.url(), "m", "s", "u", "QUJD")
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("LM Studio returned 400"), "got: {error}");
    server.finish();

    let server = mock_server("200 OK", r#"{"choices":[]}"#);
    let error = lmstudio::chat(&http_client(), server.url(), "m", "s", "u", "QUJD")
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("no completion"), "got: {error}");
    server.finish();
}

// ----------------------------------------------------------------- OpenAI

#[tokio::test]
async fn openai_sends_bearer_token_and_data_uri() {
    let server = mock_server(
        "200 OK",
        r#"{"choices":[{"message":{"content":"Title: Cloud Model"}}]}"#,
    );
    let url = format!("{}/v1/chat/completions", server.url());

    let reply = openai::chat(
        &http_client(),
        &url,
        Some("test-key"),
        "gpt-4o-mini",
        "system",
        "user",
        "QUJD",
    )
    .await
    .expect("openai reply");
    assert_eq!(reply, "Title: Cloud Model");

    let request = server.finish();
    let lower = request.to_ascii_lowercase();
    assert!(request.starts_with("POST /v1/chat/completions"));
    assert!(lower.contains("authorization: bearer test-key"), "got: {lower}");
    let body = request_body(&request);
    assert_eq!(body["max_tokens"], 900);
    assert_eq!(
        body["messages"][1]["content"][1]["image_url"]["url"],
        "data:image/jpeg;base64,QUJD"
    );
}

#[tokio::test]
async fn openai_joins_array_content_blocks() {
    let server = mock_server(
        "200 OK",
        r#"{"choices":[{"message":{"content":[{"type":"text","text":"hello"},{"type":"text","text":"world"},{"type":"image"}]}}]}"#,
    );
    let url = format!("{}/v1/chat/completions", server.url());

    let reply = openai::chat(&http_client(), &url, None, "m", "s", "u", "QUJD")
        .await
        .expect("openai reply");
    assert_eq!(reply, "hello\nworld");
    server.finish();
}

#[tokio::test]
async fn openai_reports_http_and_parse_errors() {
    let server = mock_server("401 Unauthorized", r#"{"error":{"message":"bad key"}}"#);
    let url = format!("{}/v1/chat/completions", server.url());
    let error = openai::chat(&http_client(), &url, Some("bad"), "m", "s", "u", "QUJD")
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("OpenAI returned 401"), "got: {error}");
    assert!(error.contains("bad key"), "got: {error}");
    server.finish();

    let server = mock_server("200 OK", "<html>proxy error</html>");
    let url = format!("{}/v1/chat/completions", server.url());
    let error = openai::chat(&http_client(), &url, None, "m", "s", "u", "QUJD")
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("Unexpected OpenAI response"), "got: {error}");
    server.finish();
}

// -------------------------------------------------------------- Anthropic

#[tokio::test]
async fn anthropic_sends_key_version_and_base64_image() {
    let server = mock_server(
        "200 OK",
        r#"{"content":[{"type":"text","text":"Title: Claude Image"}]}"#,
    );
    let url = format!("{}/v1/messages", server.url());

    let reply = anthropic::chat(
        &http_client(),
        &url,
        "test-key",
        "claude-sonnet-4-20250514",
        "system",
        "user",
        "QUJD",
    )
    .await
    .expect("anthropic reply");
    assert_eq!(reply, "Title: Claude Image");

    let request = server.finish();
    let lower = request.to_ascii_lowercase();
    assert!(request.starts_with("POST /v1/messages"));
    assert!(lower.contains("x-api-key: test-key"), "got: {lower}");
    assert!(lower.contains("anthropic-version: 2023-06-01"), "got: {lower}");
    let body = request_body(&request);
    assert_eq!(body["system"], "system");
    assert_eq!(body["max_tokens"], 900);
    assert_eq!(body["messages"][0]["content"][0]["type"], "image");
    assert_eq!(
        body["messages"][0]["content"][0]["source"]["media_type"],
        "image/jpeg"
    );
    assert_eq!(body["messages"][0]["content"][0]["source"]["data"], "QUJD");
    assert_eq!(body["messages"][0]["content"][1]["text"], "user");
}

#[tokio::test]
async fn anthropic_skips_non_text_blocks_and_joins_text() {
    let server = mock_server(
        "200 OK",
        r#"{"content":[{"type":"text","text":"hello"},{"type":"tool_use","name":"x"},{"type":"text","text":"world"}]}"#,
    );
    let url = format!("{}/v1/messages", server.url());
    let reply = anthropic::chat(&http_client(), &url, "key", "m", "s", "u", "QUJD")
        .await
        .expect("anthropic reply");
    assert_eq!(reply, "hello\nworld");
    server.finish();
}

#[tokio::test]
async fn anthropic_reports_missing_content_and_http_errors() {
    let server = mock_server("200 OK", "{}");
    let url = format!("{}/v1/messages", server.url());
    let error = anthropic::chat(&http_client(), &url, "key", "m", "s", "u", "QUJD")
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("no content"), "got: {error}");
    server.finish();

    let server = mock_server("429 Too Many Requests", r#"{"error":"rate limited"}"#);
    let url = format!("{}/v1/messages", server.url());
    let error = anthropic::chat(&http_client(), &url, "key", "m", "s", "u", "QUJD")
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("Anthropic returned 429"), "got: {error}");
    assert!(error.contains("rate limited"), "got: {error}");
    server.finish();
}

// ----------------------------------------------------------- model lists

#[test]
fn model_list_endpoints_are_provider_specific() {
    assert_eq!(
        models::models_endpoint(Provider::Ollama, "http://localhost:11434"),
        "http://localhost:11434/api/tags"
    );
    assert_eq!(
        models::models_endpoint(Provider::Ollama, "http://localhost:11434/api"),
        "http://localhost:11434/api/tags"
    );
    assert_eq!(
        models::models_endpoint(Provider::Ollama, "http://localhost:11434/api/chat"),
        "http://localhost:11434/api/tags"
    );
    assert_eq!(
        models::models_endpoint(Provider::LmStudio, "http://localhost:1234"),
        "http://localhost:1234/v1/models"
    );
    assert_eq!(
        models::models_endpoint(Provider::LmStudio, "http://localhost:1234/v1"),
        "http://localhost:1234/v1/models"
    );
    assert_eq!(
        models::models_endpoint(Provider::LmStudio, "http://localhost:1234/v1/chat/completions"),
        "http://localhost:1234/v1/models"
    );
    assert_eq!(
        models::models_endpoint(Provider::OpenAi, "https://api.openai.com/v1"),
        "https://api.openai.com/v1/models"
    );
    assert_eq!(
        models::models_endpoint(Provider::Anthropic, "https://api.anthropic.com/v1"),
        "https://api.anthropic.com/v1/models"
    );
}

#[tokio::test]
async fn ollama_model_list_uses_tags_and_sorts_names() {
    let server = mock_server(
        "200 OK",
        r#"{"models":[{"name":"qwen2-vl:latest"},{"name":"llava:7b"}]}"#,
    );
    let settings = ProviderSettings {
        base_url: format!("{}/api", server.url()),
        model: String::new(),
        api_key: String::new(),
    };

    let names = models::list_models(Provider::Ollama, &settings)
        .await
        .expect("ollama models");
    assert_eq!(names, vec!["llava:7b", "qwen2-vl:latest"]);

    let request = server.finish();
    assert!(
        request.starts_with("GET /api/tags HTTP/1.1"),
        "got: {request}"
    );
}

#[tokio::test]
async fn lmstudio_model_list_uses_v1_models_without_auth() {
    let server = mock_server(
        "200 OK",
        r#"{"data":[{"id":"qwen2-vl-7b-instruct"},{"id":"llava-v1.6"}]}"#,
    );
    let settings = ProviderSettings {
        base_url: server.url().to_string(),
        model: String::new(),
        api_key: String::new(),
    };

    let names = models::list_models(Provider::LmStudio, &settings)
        .await
        .expect("lm studio models");
    assert_eq!(names, vec!["llava-v1.6", "qwen2-vl-7b-instruct"]);

    let request = server.finish();
    let lower = request.to_ascii_lowercase();
    assert!(request.starts_with("GET /v1/models HTTP/1.1"), "got: {request}");
    assert!(!lower.contains("authorization:"), "no auth header expected");
}

#[tokio::test]
async fn openai_model_list_sends_bearer_and_parses_ids() {
    let server = mock_server(
        "200 OK",
        r#"{"data":[{"id":"gpt-4o-mini"},{"id":"gpt-4o"}]}"#,
    );
    let settings = ProviderSettings {
        base_url: format!("{}/v1", server.url()),
        model: String::new(),
        api_key: "sk-test".to_string(),
    };

    let names = models::list_models(Provider::OpenAi, &settings)
        .await
        .expect("openai models");
    assert_eq!(names, vec!["gpt-4o", "gpt-4o-mini"]);

    let request = server.finish();
    let lower = request.to_ascii_lowercase();
    assert!(request.starts_with("GET /v1/models HTTP/1.1"), "got: {request}");
    assert!(lower.contains("authorization: bearer sk-test"), "got: {lower}");
}

#[tokio::test]
async fn anthropic_model_list_sends_key_and_version_headers() {
    let server = mock_server(
        "200 OK",
        r#"{"data":[{"id":"claude-sonnet-4-20250514"}]}"#,
    );
    let settings = ProviderSettings {
        base_url: format!("{}/v1", server.url()),
        model: String::new(),
        api_key: "test-key".to_string(),
    };

    let names = models::list_models(Provider::Anthropic, &settings)
        .await
        .expect("anthropic models");
    assert_eq!(names, vec!["claude-sonnet-4-20250514"]);

    let request = server.finish();
    let lower = request.to_ascii_lowercase();
    assert!(request.starts_with("GET /v1/models HTTP/1.1"), "got: {request}");
    assert!(lower.contains("x-api-key: test-key"), "got: {lower}");
    assert!(lower.contains("anthropic-version: 2023-06-01"), "got: {lower}");
}

#[tokio::test]
async fn cloud_model_lists_require_an_api_key() {
    let settings = ProviderSettings {
        base_url: "http://127.0.0.1:9".to_string(),
        model: String::new(),
        api_key: String::new(),
    };

    for provider in [Provider::OpenAi, Provider::Anthropic] {
        let error = models::list_models(provider, &settings)
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("API key"), "got: {error}");
    }
}
