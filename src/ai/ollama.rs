use super::{endpoint, snippet};
use anyhow::{anyhow, bail, Context, Result};
use reqwest::Client;
use serde_json::{json, Value};

pub(crate) fn endpoint_for(base_url: &str) -> String {
    endpoint(base_url, "/api/chat")
}

pub(crate) fn payload(
    model: &str,
    system: &str,
    user: &str,
    image_base64: Option<&str>,
    temperature: f64,
) -> Value {
    // A keywords-only top-up sends no image, so the `images` field is omitted.
    let mut message = json!({"role": "user", "content": user});
    if let Some(image_base64) = image_base64 {
        message["images"] = json!([image_base64]);
    }
    json!({
        "model": model,
        "stream": false,
        "messages": [{"role": "system", "content": system}, message],
        "options": {"temperature": temperature}
    })
}

pub async fn chat(
    client: &Client,
    base_url: &str,
    model: &str,
    system: &str,
    user: &str,
    image_base64: Option<&str>,
    temperature: f64,
) -> Result<String> {
    let url = endpoint_for(base_url);
    let body = payload(model, system, user, image_base64, temperature);

    let response = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .with_context(|| format!("Could not reach Ollama at {url}. Is the server running?"))?;

    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        bail!("Ollama returned {status}: {}", snippet(&text));
    }

    let value: Value = serde_json::from_str(&text)
        .with_context(|| format!("Unexpected Ollama response: {}", snippet(&text)))?;

    if let Some(error) = value.get("error").and_then(|value| value.as_str()) {
        bail!("Ollama error: {error}");
    }

    value
        .pointer("/message/content")
        .and_then(|content| content.as_str())
        .or_else(|| value.get("response").and_then(|content| content.as_str()))
        .map(str::to_string)
        .ok_or_else(|| anyhow!("Ollama returned no message content: {}", snippet(&text)))
}
