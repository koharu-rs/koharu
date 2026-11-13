//! The preferences: `config.toml`'s sections, each handed to its owner when
//! saved.

use std::sync::Arc;

use koharu_agent::AgentConfig;
use koharu_config::secrets::SecretString;
use koharu_llm::Providers;
use koharu_pipeline::PipelineConfig;
use koharu_renderer::RendererConfig;
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::State;

use super::{Result, blocking};
use crate::app::App;

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct Preferences {
    pub pipeline: PipelineConfig,
    pub renderer: RendererConfig,
    pub agent: AgentConfig,
    pub providers: Providers,
}

fn preferences() -> anyhow::Result<Preferences> {
    Ok(Preferences {
        pipeline: koharu_config::get()?,
        renderer: koharu_config::get()?,
        agent: koharu_config::get()?,
        providers: koharu_config::get()?,
    })
}

#[tauri::command]
#[specta::specta]
pub fn get_preferences() -> Result<Preferences> {
    Ok(preferences()?)
}

/// Saves every section, hands each to its owner, and answers them as the
/// file holds them now.
#[tauri::command]
#[specta::specta]
pub async fn save_preferences(
    app: State<'_, Arc<App>>,
    preferences: Preferences,
) -> Result<Preferences> {
    let app = Arc::clone(&app);
    blocking(move || {
        let Preferences {
            pipeline,
            renderer,
            agent,
            providers,
        } = preferences;
        koharu_config::update(|section: &mut PipelineConfig| *section = pipeline)?;
        koharu_config::update(|section: &mut RendererConfig| *section = renderer)?;
        koharu_config::update(|section: &mut AgentConfig| *section = agent)?;
        koharu_config::update(|section: &mut Providers| *section = providers)?;
        let saved = self::preferences()?;
        app.pipeline.set_config(saved.pipeline.clone());
        app.renderer.set_config(saved.renderer.clone());
        // The renderer's fonts lay the page out again.
        app.changed()?;
        Ok(saved)
    })
    .await
}

/// A model the translator or the agent can use.
#[derive(Clone, Debug, Serialize, Type)]
pub struct ModelInfo {
    /// A `vendor/model` reference, as the preferences hold one.
    pub reference: String,
    pub name: String,
    pub vendor: String,
    /// Whether it reads images, as the translator's vision needs.
    pub images: bool,
    /// Whether it calls tools, as the agent needs.
    pub tools: bool,
}

/// Koharu's own models, and each vendor's that has a key or a sign-in.
#[tauri::command]
#[specta::specta]
pub async fn get_models() -> Vec<ModelInfo> {
    koharu_llm::models()
        .await
        .into_iter()
        .map(|spec| ModelInfo {
            reference: spec.reference(),
            name: spec.display_name.clone(),
            vendor: spec.provider.vendor().to_string(),
            images: spec.input.image,
            tools: spec.tools,
        })
        .collect()
}

/// A vendor Koharu lists models from, and whether its API key is stored.
#[derive(Clone, Debug, Serialize, Type)]
pub struct Vendor {
    pub name: String,
    pub key: bool,
}

/// The vendors that take an API key, by rig's names, each with whether one
/// is stored. ChatGPT signs in instead, and its sign-in has the slot a key
/// would take.
#[tauri::command]
#[specta::specta]
pub async fn get_vendors() -> Result<Vec<Vendor>> {
    blocking(|| {
        keyed()
            .map(|name| {
                Ok(Vendor {
                    name: name.to_owned(),
                    key: koharu_config::secrets::get(name)?.is_some(),
                })
            })
            .collect()
    })
    .await
}

/// Stores `vendor`'s API key, encrypted with the key the OS credential
/// store keeps, or forgets it for `None`.
#[tauri::command]
#[specta::specta]
pub async fn set_api_key(vendor: String, key: Option<String>) -> Result<()> {
    blocking(move || {
        anyhow::ensure!(
            keyed().any(|name| name == vendor),
            "{vendor} isn't a vendor that takes an API key"
        );
        match key {
            Some(key) => koharu_config::secrets::set(&vendor, &SecretString::from(key))?,
            None => koharu_config::secrets::delete(&vendor)?,
        }
        Ok(())
    })
    .await
}

fn keyed() -> impl Iterator<Item = &'static str> {
    koharu_llm::VENDORS
        .iter()
        .copied()
        .filter(|&vendor| vendor != koharu_llm::rig::providers::chatgpt::PROVIDER_NAME)
}
