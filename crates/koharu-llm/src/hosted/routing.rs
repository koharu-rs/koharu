//! Which upstream host OpenRouter sends a request to: its `provider` request
//! object, from the rule `[providers.openrouter]` sets for all models and the
//! host it pins for one, and the hosts OpenRouter lists for a model.

use anyhow::{Context, Result, ensure};
use rig_core::{driver::DynModel, operation::Completion, providers::openrouter::PROVIDER_NAME};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{Providers, base_url, key, reqwest};

/// What OpenRouter orders a model's hosts by, as its `provider.sort` names it.
/// Unset, it balances price against uptime.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum Routing {
    Price,
    Throughput,
    Latency,
}

/// A host OpenRouter lists for a model.
#[derive(Clone, Debug, Serialize, specta::Type)]
pub struct Host {
    /// What `provider.only` takes: the host and its quantization, as
    /// OpenRouter tags the endpoint.
    pub tag: String,
    pub name: String,
    pub quantization: Option<String>,
}

/// The `provider` object of a request: only hosts that keep to its schema, and
/// then the pinned host with no fallback, or else hosts in the order `routing`
/// gives. A pinned host leaves nothing to sort.
fn provider(routing: Option<Routing>, pin: Option<&str>) -> Value {
    match (pin, routing) {
        (Some(tag), _) => {
            json!({"require_parameters": true, "only": [tag], "allow_fallbacks": false})
        }
        (None, Some(routing)) => json!({"require_parameters": true, "sort": routing}),
        (None, None) => json!({"require_parameters": true}),
    }
}

fn saved(id: &str) -> Result<(Option<Routing>, Option<String>)> {
    let providers: Providers = koharu_config::get()?;
    let endpoint = providers.0.get(PROVIDER_NAME);
    Ok((
        endpoint.and_then(|endpoint| endpoint.routing),
        endpoint.and_then(|endpoint| endpoint.pins.get(id).cloned()),
    ))
}

pub(crate) fn request(id: &str) -> Result<Value> {
    let (routing, pin) = saved(id)?;
    Ok(provider(routing, pin.as_deref()))
}

/// `error`, a failed request to `model`, with what a pinned host's refusal
/// means: OpenRouter answers 404 "No endpoints found" when the one host a
/// request is pinned to can't serve it.
pub fn explain(model: &DynModel<Completion>, error: anyhow::Error) -> anyhow::Error {
    let Some(spec) = model.spec() else {
        return error;
    };
    let pin = saved(&spec.id).ok().and_then(|(_, pin)| pin);
    explained(spec.provider.vendor(), pin.as_deref(), error)
}

fn explained(vendor: &str, pin: Option<&str>, error: anyhow::Error) -> anyhow::Error {
    match pin {
        Some(tag) if vendor == PROVIDER_NAME && format!("{error:#}").contains("No endpoints found") => {
            error.context(format!(
                "OpenRouter's provider `{tag}`, pinned for this model, can't serve this request. Choose Auto or another provider in Settings → Pipeline → Translator"
            ))
        }
        _ => error,
    }
}

/// The hosts OpenRouter lists for model `id`, `author/slug`, which its
/// endpoints list answers without a key, though it takes the user's.
pub async fn hosts(id: &str) -> Result<Vec<Host>> {
    let (author, slug) = id
        .split_once('/')
        .filter(|(author, slug)| !author.is_empty() && !slug.is_empty() && !slug.contains('/'))
        .with_context(|| format!("`{id}` is no OpenRouter model: expected `author/slug`"))?;
    let mut url = url::Url::parse(
        &base_url(PROVIDER_NAME)?.unwrap_or_else(|| "https://openrouter.ai/api/v1".to_owned()),
    )?;
    url.path_segments_mut()
        .ok()
        .context("OpenRouter's base URL can't take a path")?
        .pop_if_empty()
        .extend(["models", author, slug, "endpoints"]);
    let mut request = reqwest()?.get(url);
    if let Some(key) = key(PROVIDER_NAME)? {
        request = request.bearer_auth(key);
    }
    let response = request.send().await?;
    let status = response.status();
    ensure!(
        status.is_success(),
        "OpenRouter's host list answered {status}"
    );
    let reply: Value = response.json().await?;
    Ok(hosts_in(&reply))
}

/// The hosts `reply`, an endpoints list, names, once each; a row with no tag
/// is no host a request can pin, and one that doesn't list
/// `structured_outputs` among its `supported_parameters` can't serve the
/// request's strict schema: OpenRouter answers 404 for it, as `provider`
/// requires every parameter.
fn hosts_in(reply: &Value) -> Vec<Host> {
    let mut hosts: Vec<Host> = Vec::new();
    for row in reply
        .pointer("/data/endpoints")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let text = |key| {
            row.get(key)
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
        };
        let Some(tag) = text("tag") else { continue };
        let structured = row
            .get("supported_parameters")
            .and_then(Value::as_array)
            .is_some_and(|parameters| {
                parameters
                    .iter()
                    .any(|parameter| parameter.as_str() == Some("structured_outputs"))
            });
        if !structured {
            continue;
        }
        if hosts.iter().any(|host| host.tag == tag) {
            continue;
        }
        hosts.push(Host {
            tag: tag.to_owned(),
            name: text("provider_name").unwrap_or(tag).to_owned(),
            quantization: text("quantization")
                .filter(|quantization| *quantization != "unknown")
                .map(str::to_owned),
        });
    }
    hosts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn balanced_routing_sorts_nothing() {
        assert_eq!(provider(None, None), json!({"require_parameters": true}));
    }

    #[test]
    fn a_routing_rule_sorts_by_its_name() {
        for (routing, sort) in [
            (Routing::Price, "price"),
            (Routing::Throughput, "throughput"),
            (Routing::Latency, "latency"),
        ] {
            assert_eq!(
                provider(Some(routing), None),
                json!({"require_parameters": true, "sort": sort})
            );
        }
    }

    /// A pinned host serves the request or it fails; a sort has nothing left
    /// to order.
    #[test]
    fn a_pinned_host_is_the_only_one_and_is_not_sorted() {
        assert_eq!(
            provider(Some(Routing::Price), Some("deepinfra/fp4")),
            json!({
                "require_parameters": true,
                "only": ["deepinfra/fp4"],
                "allow_fallbacks": false,
            })
        );
    }

    const NO_ENDPOINTS: &str = r#"ProviderResponseError: status 404 Not Found: {"error":{"message":"No endpoints found that can handle the requested parameters.","code":404}}"#;

    #[test]
    fn a_pinned_host_that_cannot_serve_the_request_is_named() {
        let error = explained(
            PROVIDER_NAME,
            Some("tenstorrent/fp8"),
            anyhow::anyhow!(NO_ENDPOINTS),
        );
        assert!(error.to_string().contains("tenstorrent/fp8"));
        assert_ne!(error.to_string(), NO_ENDPOINTS);
        assert_eq!(error.root_cause().to_string(), NO_ENDPOINTS);
    }

    #[test]
    fn an_error_is_left_as_it_is_unless_a_pin_caused_it() {
        let unchanged = |vendor, pin, text: &str| {
            let error = explained(vendor, pin, anyhow::anyhow!(text.to_owned()));
            assert_eq!(format!("{error:#}"), text);
        };
        unchanged(PROVIDER_NAME, None, NO_ENDPOINTS);
        unchanged(
            PROVIDER_NAME,
            Some("novita"),
            "status 429 Too Many Requests",
        );
        unchanged("openai", Some("novita"), NO_ENDPOINTS);
    }

    #[test]
    fn the_hosts_are_read_from_an_endpoints_list() {
        let reply = json!({"data": {"endpoints": [
            {"provider_name": "DeepInfra", "tag": "deepinfra/fp4", "quantization": "fp4",
                "supported_parameters": ["max_tokens", "structured_outputs"]},
            {"provider_name": "DeepInfra", "tag": "deepinfra/fp4", "quantization": "fp4",
                "supported_parameters": ["structured_outputs"]},
            {"provider_name": "Novita", "tag": "novita", "quantization": "unknown",
                "supported_parameters": ["structured_outputs"]},
            {"provider_name": "Nameless", "supported_parameters": ["structured_outputs"]},
            {"provider_name": "Tenstorrent", "tag": "tenstorrent/fp8", "quantization": "fp8",
                "supported_parameters": ["max_tokens", "response_format"]},
            {"provider_name": "Bare", "tag": "bare", "quantization": "fp8"},
        ]}});
        let hosts = hosts_in(&reply);
        assert_eq!(
            hosts
                .iter()
                .map(|host| (
                    host.tag.as_str(),
                    host.name.as_str(),
                    host.quantization.as_deref()
                ))
                .collect::<Vec<_>>(),
            [
                ("deepinfra/fp4", "DeepInfra", Some("fp4")),
                ("novita", "Novita", None),
            ]
        );
        assert!(hosts_in(&json!({})).is_empty());
    }
}
