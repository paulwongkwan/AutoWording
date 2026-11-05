use super::{endpoint, snippet};
use crate::config::{Provider, ProviderSettings};
use anyhow::{bail, Context, Result};
use reqwest::Client;
use serde_json::Value;
use std::time::Duration;

/// Fetch the model IDs exposed by a provider's configured endpoint.
///
/// Local providers use their native model-list routes:
/// - Ollama: `GET /api/tags` (`models[].name`)
/// - LM Studio: `GET /v1/models` (`data[].id`)
///
/// Cloud providers use their OpenAI/Anthropic model-list routes and require
/// the configured API key:
/// - OpenAI: `GET /v1/models` with a Bearer token
/// - Anthropic: `GET /v1/models` with `x-api-key` and an API version header
pub async fn list_models(provider: Provider, settings: &ProviderSettings) -> Result<Vec<String>> {
    let base_url = settings.base_url.trim();
    let api_key = settings.api_key.trim();

    if base_url.is_empty() {
        bail!("No endpoint configured for {}.", provider.label());
    }
    if provider.requires_key() && api_key.is_empty() {
        bail!(
            "{} needs an API key before its model list can be loaded.",
            provider.label()
        );
    }

    let url = models_endpoint(provider, base_url);
    let client = Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .context("Could not build the HTTP client")?;

    let mut request = client.get(&url);
    match provider {
        Provider::OpenAi => request = request.bearer_auth(api_key),
        Provider::Anthropic => {
            request = request
                .header("x-api-key", api_key)
                .header("anthropic-version", "2023-06-01");
        }
        Provider::Ollama | Provider::LmStudio => {}
    }

    let response = request
        .send()
        .await
        .with_context(|| format!("Could not reach {} at {url}", provider.label()))?;

    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        bail!(
            "{} model list returned {status}: {}",
            provider.label(),
            snippet(&text)
        );
    }

    let value: Value = serde_json::from_str(&text)
        .with_context(|| format!("Unexpected {} model list: {}", provider.label(), snippet(&text)))?;

    let mut models = match provider {
        Provider::Ollama => named_models(&value, "models", &["name", "model"]),
        Provider::LmStudio | Provider::OpenAi | Provider::Anthropic => named_models(
            &value,
            "data",
            &["id"],
        ),
    };

    models.retain(|model| !model.trim().is_empty());
    for model in &mut models {
        *model = model.trim().to_string();
    }
    models.sort_by_key(|model| model.to_ascii_lowercase());
    models.dedup_by(|left, right| left.eq_ignore_ascii_case(right));

    if models.is_empty() {
        bail!("{} returned no models.", provider.label());
    }

    Ok(models)
}

/// Build the correct model-list URL for the provider and a user-supplied base.
pub fn models_endpoint(provider: Provider, base_url: &str) -> String {
    let base = base_url.trim().trim_end_matches('/');

    match provider {
        Provider::Ollama => {
            if base.ends_with("/api/tags") {
                base.to_string()
            } else if base.ends_with("/api/chat") {
                format!("{}/tags", base.trim_end_matches("/chat"))
            } else if base.ends_with("/api") {
                format!("{base}/tags")
            } else {
                endpoint(base, "/api/tags")
            }
        }
        Provider::LmStudio => {
            if base.ends_with("/v1/models") {
                base.to_string()
            } else if base.ends_with("/chat/completions") {
                format!("{}/models", base.trim_end_matches("/chat/completions"))
            } else if base.ends_with("/v1") {
                format!("{base}/models")
            } else {
                format!("{base}/v1/models")
            }
        }
        Provider::OpenAi => {
            if base.ends_with("/models") {
                base.to_string()
            } else if base.ends_with("/chat/completions") {
                format!("{}/models", base.trim_end_matches("/chat/completions"))
            } else {
                endpoint(base, "/models")
            }
        }
        Provider::Anthropic => {
            if base.ends_with("/models") {
                base.to_string()
            } else if base.ends_with("/messages") {
                format!("{}/models", base.trim_end_matches("/messages"))
            } else {
                endpoint(base, "/models")
            }
        }
    }
}

fn named_models(value: &Value, array_name: &str, fields: &[&str]) -> Vec<String> {
    value
        .get(array_name)
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| {
                    fields
                        .iter()
                        .find_map(|field| entry.get(*field).and_then(Value::as_str))
                })
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}
