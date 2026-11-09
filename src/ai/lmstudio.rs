use super::{endpoint, snippet};
use anyhow::{anyhow, bail, Context, Result};
use reqwest::Client;
use serde_json::{json, Value};

/// LM Studio exposes an OpenAI-compatible endpoint on `/v1`.
pub(crate) fn endpoint_for(base_url: &str) -> String {
    let base = base_url.trim().trim_end_matches('/');
    if base.ends_with("/chat/completions") {
        base.to_string()
    } else if base.ends_with("/v1") {
        endpoint(base, "/chat/completions")
    } else {
        endpoint(base, "/v1/chat/completions")
    }
}

pub(crate) fn payload(
    model: &str,
    system: &str,
    user: &str,
    image_base64: &str,
    temperature: f64,
) -> Value {
    json!({
        "model": model,
        "temperature": temperature,
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

pub async fn chat(
    client: &Client,
    base_url: &str,
    model: &str,
    system: &str,
    user: &str,
    image_base64: &str,
    temperature: f64,
) -> Result<String> {
    let url = endpoint_for(base_url);
    let body = payload(model, system, user, image_base64, temperature);

    let response = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .with_context(|| format!("Could not reach LM Studio at {url}. Is the server running?"))?;

    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        bail!("LM Studio returned {status}: {}", snippet(&text));
    }

    let value: Value = serde_json::from_str(&text)
        .with_context(|| format!("Unexpected LM Studio response: {}", snippet(&text)))?;

    value
        .pointer("/choices/0/message/content")
        .and_then(|content| content.as_str())
        .map(str::to_string)
        .ok_or_else(|| anyhow!("LM Studio returned no completion: {}", snippet(&text)))
}
