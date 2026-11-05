use super::snippet;
use anyhow::{anyhow, bail, Context, Result};
use reqwest::Client;
use serde_json::{json, Value};

const ANTHROPIC_VERSION: &str = "2023-06-01";

pub(crate) fn payload(model: &str, system: &str, user: &str, image_base64: &str) -> Value {
    json!({
        "model": model,
        "max_tokens": 900,
        "temperature": 0.2,
        "system": system,
        "messages": [{
            "role": "user",
            "content": [
                {
                    "type": "image",
                    "source": {
                        "type": "base64",
                        "media_type": "image/jpeg",
                        "data": image_base64
                    }
                },
                {"type": "text", "text": user}
            ]
        }]
    })
}

pub async fn chat(
    client: &Client,
    url: &str,
    api_key: &str,
    model: &str,
    system: &str,
    user: &str,
    image_base64: &str,
) -> Result<String> {
    let body = payload(model, system, user, image_base64);

    let response = client
        .post(url)
        .header("x-api-key", api_key)
        .header("anthropic-version", ANTHROPIC_VERSION)
        .json(&body)
        .send()
        .await
        .with_context(|| format!("Could not reach Anthropic at {url}"))?;

    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        bail!("Anthropic returned {status}: {}", snippet(&text));
    }

    let value: Value = serde_json::from_str(&text)
        .with_context(|| format!("Unexpected Anthropic response: {}", snippet(&text)))?;

    let blocks = value
        .get("content")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("Anthropic returned no content: {}", snippet(&text)))?;

    let mut out = String::new();
    for block in blocks {
        if block.get("type").and_then(Value::as_str) == Some("text") {
            if let Some(text) = block.get("text").and_then(Value::as_str) {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(text);
            }
        }
    }

    if out.is_empty() {
        bail!("Anthropic returned no text blocks: {}", snippet(&text));
    }
    Ok(out)
}
