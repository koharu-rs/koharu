//! The preferences: `config.toml`'s sections, each handed to its owner when
//! saved, and each vendor's API key or sign-in.

use std::sync::Arc;

use futures::future::{AbortHandle, Abortable};
use koharu_agent::AgentConfig;
use koharu_config::secrets::SecretString;
use koharu_llm::{Model, Providers, SignInPrompt, VENDORS};
use koharu_pipeline::PipelineConfig;
use koharu_renderer::RendererConfig;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{State, ipc::Channel};

use super::Result;
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
    Ok(tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
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
        // The preferences are saved by now, so a page that fails to redraw
        // with them doesn't fail the save.
        if let Err(error) = app.refresh() {
            tracing::error!("failed to show the project: {error:#}");
        }
        Ok(saved)
    })
    .await??)
}

/// Koharu's own models, and each vendor's that has a key or a sign-in.
#[tauri::command]
#[specta::specta]
pub async fn list_models() -> Vec<Model> {
    koharu_llm::models().await
}

/// A vendor, by rig's name, and whether its sign-in or key is stored. What
/// the user gives it, a key, a sign-in, a server, is the interface's to say.
#[derive(Clone, Debug, Serialize, Type)]
pub struct Vendor {
    pub name: String,
    pub connected: bool,
}

/// Every vendor Koharu lists models from, in its order.
#[tauri::command]
#[specta::specta]
pub async fn list_vendors() -> Result<Vec<Vendor>> {
    Ok(tokio::task::spawn_blocking(|| -> anyhow::Result<_> {
        VENDORS
            .iter()
            .map(|&name| {
                Ok(Vendor {
                    name: name.to_owned(),
                    connected: koharu_config::secrets::get(name)?.is_some(),
                })
            })
            .collect()
    })
    .await??)
}

/// Stores `vendor`'s API key, encrypted with the key the OS credential
/// store keeps, or forgets it for `None`.
#[tauri::command]
#[specta::specta]
pub async fn set_api_key(vendor: String, key: Option<String>) -> Result<()> {
    Ok(tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        match key {
            Some(key) => koharu_config::secrets::set(&vendor, &SecretString::from(key))?,
            None => koharu_config::secrets::delete(&vendor)?,
        }
        Ok(())
    })
    .await??)
}

/// The sign-in waiting for the user, which the next one or `stop_sign_in`
/// ends.
static SIGNING_IN: Mutex<Option<AbortHandle>> = Mutex::new(None);

/// Signs in to `vendor`, one that signs in, in place of the sign-in
/// waiting; `on_prompt` hears what the user must do. A stopped sign-in ends
/// without one.
#[tauri::command]
#[specta::specta]
pub async fn sign_in(vendor: String, on_prompt: Channel<SignInPrompt>) -> Result<()> {
    let (stop, stopped) = AbortHandle::new_pair();
    if let Some(waiting) = SIGNING_IN.lock().replace(stop) {
        waiting.abort();
    }
    let signed_in = koharu_llm::sign_in(&vendor, move |prompt| {
        let _ = on_prompt.send(prompt);
    });
    Ok(Abortable::new(signed_in, stopped).await.unwrap_or(Ok(()))?)
}

/// Stops the sign-in waiting for the user.
#[tauri::command]
#[specta::specta]
pub fn stop_sign_in() {
    if let Some(waiting) = SIGNING_IN.lock().take() {
        waiting.abort();
    }
}

/// Signs out of `vendor`: the vendor revokes the sign-in, and Koharu
/// deletes it.
#[tauri::command]
#[specta::specta]
pub async fn sign_out(vendor: String) -> Result<()> {
    Ok(koharu_llm::sign_out(&vendor).await?)
}
