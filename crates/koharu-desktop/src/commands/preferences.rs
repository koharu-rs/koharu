//! The preferences: `config.toml`'s sections, each handed to its owner when
//! saved, and each vendor's API key or sign-in.

use std::sync::Arc;

use futures::future::{AbortHandle, Abortable};
use koharu_agent::AgentConfig;
use koharu_config::secrets::SecretString;
use koharu_io::ExportConfig;
use koharu_llm::{KOHARU, Model, Providers, SignInPrompt, VENDORS};
use koharu_pipeline::{Detector, Inpainter, PipelineConfig, Recognizer, Stage, vram::VramStore};
use koharu_renderer::RendererConfig;
use koharu_runtime::{Device, Memory, PinnedFile};
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
    pub export: ExportConfig,
}

fn preferences() -> anyhow::Result<Preferences> {
    Ok(Preferences {
        pipeline: koharu_config::get()?,
        renderer: koharu_config::get()?,
        agent: koharu_config::get()?,
        providers: koharu_config::get()?,
        export: koharu_config::get()?,
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
            export,
        } = preferences;
        koharu_config::update(|section: &mut PipelineConfig| *section = pipeline)?;
        koharu_config::update(|section: &mut RendererConfig| *section = renderer)?;
        koharu_config::update(|section: &mut AgentConfig| *section = agent)?;
        koharu_config::update(|section: &mut Providers| *section = providers)?;
        koharu_config::update(|section: &mut ExportConfig| *section = export)?;
        let saved = self::preferences()?;
        app.pipeline.set_config(saved.pipeline.clone());
        // The preferences are saved by now, so a page that fails to redraw
        // with them doesn't fail the save.
        if let Err(error) = app.refresh() {
            tracing::error!("failed to show the project: {error:#}");
        }
        Ok(saved)
    })
    .await??)
}

#[derive(Clone, Debug, Serialize, Type)]
pub struct ModelCatalog {
    pub available_vram_bytes: Option<u64>,
    pub total_vram_bytes: Option<u64>,
    pub detect: Vec<StageModelEntry>,
    pub recognize: Vec<StageModelEntry>,
    pub inpaint: Vec<StageModelEntry>,
    pub translate: Vec<TranslateModelEntry>,
}

#[derive(Clone, Debug, Serialize, Type)]
pub struct StageModelEntry {
    pub id: String,
    pub name: String,
    pub stage: Stage,
    pub disk_bytes: u64,
    pub downloaded: bool,
    pub measured_vram_bytes: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Type)]
pub struct TranslateModelEntry {
    pub reference: String,
    pub name: String,
    pub vendor: String,
    pub images: bool,
    pub tools: bool,
    pub disk_bytes: Option<u64>,
    pub downloaded: bool,
    pub measured_vram_bytes: Option<u64>,
    pub variants: Vec<ModelVariant>,
}

#[derive(Clone, Debug, Serialize, Type)]
pub struct ModelVariant {
    pub id: String,
    pub reference: String,
    pub name: String,
    pub disk_bytes: u64,
    pub downloaded: bool,
    pub measured_vram_bytes: Option<u64>,
}

/// Every stage's models and each vendor's language models, together with disk
/// sizes, download status, and optional VRAM telemetry/footprints when a
/// discrete CUDA/ROCm runtime is already installed locally.
#[tauri::command]
#[specta::specta]
pub async fn list_model_catalog() -> Result<ModelCatalog> {
    let pipeline: PipelineConfig = koharu_config::get()?;
    let vision = pipeline.translator.vision;
    let device = koharu_runtime::try_device().await;
    let accounting_device = device.filter(|d| d.has_vram_accounting());
    let memory = accounting_device.map(Device::memory);
    let vram = if accounting_device.is_some() {
        tokio::task::spawn_blocking(|| {
            VramStore::open_default().unwrap_or_else(|error| {
                tracing::warn!("ignoring unreadable VRAM cache: {error:#}");
                VramStore::in_memory()
            })
        })
        .await
        .map_err(anyhow::Error::from)?
    } else {
        VramStore::in_memory()
    };
    let llm_models = koharu_llm::models().await;
    Ok(build_model_catalog(
        vision,
        accounting_device,
        memory,
        &vram,
        llm_models,
    )?)
}

fn build_model_catalog(
    vision: bool,
    device: Option<&Device>,
    memory: Option<Memory>,
    vram: &VramStore,
    llm_models: Vec<Model>,
) -> anyhow::Result<ModelCatalog> {
    let stage_entry = |stage: Stage, id: &str, name: String, files: Vec<PinnedFile>| {
        let disk_bytes = files.iter().map(|file| file.bytes).sum();
        let downloaded = files.iter().all(PinnedFile::exists);
        let measured_vram_bytes =
            device.and_then(|dev| vram.measured_bytes(dev, stage, id, false, &files));
        StageModelEntry {
            id: id.to_owned(),
            name,
            stage,
            disk_bytes,
            downloaded,
            measured_vram_bytes,
        }
    };

    let detect = Detector::ALL
        .iter()
        .map(|model| stage_entry(Stage::Detect, model.id(), model.to_string(), model.files()))
        .collect();
    let recognize = Recognizer::ALL
        .iter()
        .map(|model| {
            stage_entry(
                Stage::Recognize,
                model.id(),
                model.to_string(),
                model.files(),
            )
        })
        .collect();
    let inpaint = Inpainter::ALL
        .iter()
        .map(|model| stage_entry(Stage::Inpaint, model.id(), model.to_string(), model.files()))
        .collect();

    let mut translate = Vec::with_capacity(llm_models.len());
    for model in llm_models {
        if model.vendor == KOHARU {
            let mut variants = Vec::new();
            if let Some(local) = koharu_llm::local(&model.reference, vision) {
                for quant in local.quantizations {
                    let reference = format!("{}:{}", model.reference, quant.id);
                    let files = koharu_llm::local_files(&reference, vision)?;
                    let measured_vram_bytes = device.and_then(|dev| {
                        vram.measured_bytes(dev, Stage::Translate, &reference, vision, &files)
                    });
                    variants.push(ModelVariant {
                        id: quant.id.to_owned(),
                        reference,
                        name: quant.name.to_owned(),
                        disk_bytes: quant.bytes,
                        downloaded: quant.downloaded,
                        measured_vram_bytes,
                    });
                }
            }
            let disk_bytes = variants.first().map(|variant| variant.disk_bytes);
            let downloaded = variants.first().is_some_and(|variant| variant.downloaded);
            let measured_vram_bytes = variants
                .first()
                .and_then(|variant| variant.measured_vram_bytes);
            translate.push(TranslateModelEntry {
                reference: model.reference,
                name: model.name,
                vendor: model.vendor,
                images: model.images,
                tools: model.tools,
                disk_bytes,
                downloaded,
                measured_vram_bytes,
                variants,
            });
        } else {
            translate.push(TranslateModelEntry {
                reference: model.reference,
                name: model.name,
                vendor: model.vendor,
                images: model.images,
                tools: model.tools,
                disk_bytes: None,
                downloaded: false,
                measured_vram_bytes: None,
                variants: Vec::new(),
            });
        }
    }

    Ok(ModelCatalog {
        available_vram_bytes: memory.map(|mem| mem.free),
        total_vram_bytes: memory.map(|mem| mem.total),
        detect,
        recognize,
        inpaint,
        translate,
    })
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

#[cfg(test)]
mod tests {
    use koharu_pipeline::vram::footprint_key;
    use koharu_runtime::Backend;

    use super::*;

    fn synthetic_cuda_device() -> Device {
        Device {
            name: "CUDA0".to_owned(),
            index: 0,
            description: "NVIDIA Test GPU".to_owned(),
            backend: Backend::Cuda {
                compute_capability: 89,
            },
            integrated: false,
        }
    }

    #[test]
    fn build_model_catalog_populates_all_stages_and_measured_vram() -> anyhow::Result<()> {
        let local_gemma = Model {
            reference: "koharu/gemma4-e2b-it".to_owned(),
            name: "Gemma 4 E2B IT".to_owned(),
            vendor: KOHARU.to_owned(),
            images: true,
            tools: true,
        };
        let hosted_gpt = Model {
            reference: "openai/gpt-5".to_owned(),
            name: "GPT-5".to_owned(),
            vendor: "openai".to_owned(),
            images: true,
            tools: true,
        };

        // Without accounting device: VRAM fields stay None.
        let unmeasured = build_model_catalog(
            false,
            None,
            None,
            &VramStore::in_memory(),
            vec![local_gemma.clone(), hosted_gpt.clone()],
        )?;
        assert_eq!(unmeasured.available_vram_bytes, None);
        assert_eq!(unmeasured.total_vram_bytes, None);
        assert_eq!(unmeasured.detect.len(), Detector::ALL.len());
        assert_eq!(unmeasured.recognize.len(), Recognizer::ALL.len());
        assert_eq!(unmeasured.inpaint.len(), Inpainter::ALL.len());
        assert!(unmeasured.inpaint.iter().all(|m| m.disk_bytes > 0));
        assert!(unmeasured.detect[0].measured_vram_bytes.is_none());

        let gemma = &unmeasured.translate[0];
        assert_eq!(gemma.variants.len(), 2);
        assert_eq!(gemma.disk_bytes, Some(gemma.variants[0].disk_bytes));
        assert!(gemma.disk_bytes.unwrap() > 0);
        let openai = &unmeasured.translate[1];
        assert_eq!(openai.disk_bytes, None);
        assert!(openai.variants.is_empty());

        // With accounting device & recorded footprint: measured_vram_bytes resolves.
        let device = synthetic_cuda_device();
        let vram = VramStore::in_memory();
        let flux_files = Inpainter::Flux3Klein.files();
        vram.record_load(
            footprint_key(&device, Stage::Inpaint, "flux3-klein", false, &flux_files),
            5_000_000_000,
            false,
            None,
        );
        let gemma_q2_files = koharu_llm::local_files("koharu/gemma4-e2b-it:Q2_K_XL", true)?;
        vram.record_load(
            footprint_key(
                &device,
                Stage::Translate,
                "koharu/gemma4-e2b-it:Q2_K_XL",
                true,
                &gemma_q2_files,
            ),
            2_500_000_000,
            false,
            None,
        );

        let measured = build_model_catalog(
            true,
            Some(&device),
            Some(Memory {
                free: 6_000_000_000,
                total: 8_000_000_000,
            }),
            &vram,
            vec![local_gemma, hosted_gpt],
        )?;
        assert_eq!(measured.available_vram_bytes, Some(6_000_000_000));
        assert_eq!(measured.total_vram_bytes, Some(8_000_000_000));
        let flux = measured
            .inpaint
            .iter()
            .find(|entry| entry.id == "flux3-klein")
            .unwrap();
        assert_eq!(flux.measured_vram_bytes, Some(5_000_000_000));
        let q2 = measured.translate[0]
            .variants
            .iter()
            .find(|variant| variant.id == "Q2_K_XL")
            .unwrap();
        assert_eq!(q2.measured_vram_bytes, Some(2_500_000_000));
        assert_eq!(measured.translate[0].measured_vram_bytes, None);
        assert_eq!(measured.translate[1].measured_vram_bytes, None);
        Ok(())
    }
}
