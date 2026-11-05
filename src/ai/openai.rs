use super::snippet;
use anyhow::{anyhow, bail, Context, Result};
use reqwest::Client;
use serde_json::{json, Value};

pub(crate) fn payload(model: &str, system: &str, user: &str, image_base64: &str) -> Value {
    json!({
        "model": model,
        "temperature": 0.2,
        "max_tokens": 900,
        "messages": [
            {"role": "system", "content": system},
            {
                "role": "user",
                "content": [
                    {"type": "text", "text": user},
                    {
                        "type": "image_url",
                        "image_url": {"url": format!("data:image/jpeg;base64,{image_base64}")}
                    }
                ]
            }
        ]
    })
}

/// OpenAI-compatible chat completion with a base64 data URI image part.
pub async fn chat(
    client: &Client,
    url: &str,
    api_key: Option<&str>,
    model: &str,
    system: &str,
    user: &str,
    image_base64: &str,
) -> Result<String> {
    let body = payload(model, system, user, image_base64);

    let mut request = client.post(url).json(&body);
    if let Some(key) = api_key.filter(|key| !key.is_empty()) {
        request = request.bearer_auth(key);
    }

    let response = request
        .send()
        .await
        .with_context(|| format!("Could not reach OpenAI at {url}"))?;

    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        bail!("OpenAI returned {status}: {}", snippet(&text));
    }

    let value: Value = serde_json::from_str(&text)
        .with_context(|| format!("Unexpected OpenAI response: {}", snippet(&text)))?;

    content_to_text(value.pointer("/choices/0/message/content"))
        .ok_or_else(|| anyhow!("OpenAI returned no completion: {}", snippet(&text)))
}

fn content_to_text(content: Option<&Value>) -> Option<String> {
    match content? {
        Value::String(text) => Some(text.clone()),
        Value::Array(parts) => {
            let mut out = String::new();
            for part in parts {
                if let Some(text) = part.get("text").and_then(Value::as_str) {
                    if !out.is_empty() {
                        out.push('\n');
                    }
                    out.push_str(text);
                }
            }
            (!out.is_empty()).then_some(out)
        }
        _ => None,
    }
}
