//! Which models are loaded, and room for the next one. The lane owns it, so
//! a model in use is never unloaded.

use std::{
    cmp::Ordering,
    collections::BTreeMap,
    time::{Duration, Instant},
};

use anyhow::{Result, bail};
use image::{DynamicImage, GrayImage, Luma};
use koharu_ml::{
    aot_inpainting::{self, AotInpainting},
    baberu_ocr::BaberuOcr,
    flux3_klein::Flux3Klein,
    hayai_ocr::HayaiOcr,
    koharu_layout_highres::{self, KoharuLayoutHighres},
    lama::{self, Lama},
    manga_ocr::MangaOcr,
    paddle_ocr_vl_quantized::{self, PaddleOcrVlQuantized},
    rorem_mixed::RoremMixed,
};
use koharu_runtime::{Device, Devices};

/// A loaded model stays this long after its last use, Ollama's default.
pub(crate) const IDLE: Duration = Duration::from_secs(5 * 60);
/// Free memory a load leaves for everything else.
const RESERVE: u64 = 1 << 30;
/// After an unload, free memory is read this often, for this long, until
/// this share of the model's footprint shows, as Ollama waits.
const RECOVERY_POLL: Duration = Duration::from_millis(250);
const RECOVERY_WAIT: Duration = Duration::from_secs(5);
const RECOVERY_SHARE: f64 = 0.75;

/// Pages a local translation model decodes together, and the tokens each
/// holds: the instructions, a page's blocks, its scan, and the answer.
/// Fixed until the KV cache's cost per token is measured for the budget.
pub(crate) const TRANSLATION_SIZE: koharu_llm::Size = koharu_llm::Size {
    sequences: 4,
    context: 8192,
};
/// Crops PaddleOCR-VL reads together.
pub(crate) const OCR_SEQUENCES: u32 = 8;

/// A model the lane runs, with what it loads with.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Model {
    Layout,
    MangaOcr,
    BaberuOcr,
    HayaiOcr,
    PaddleOcrVl,
    /// A local language model; a hosted one runs off the lane.
    Translator {
        model: String,
        quantization: Option<String>,
    },
    Lama,
    Aot,
    Flux3Klein,
    RoremMixed,
}

impl Model {
    /// Only llama.cpp's and stable-diffusion.cpp's models give memory back
    /// when unloaded; libtorch keeps what its models freed for its own.
    fn evictable(&self) -> bool {
        matches!(
            self,
            Self::PaddleOcrVl | Self::Translator { .. } | Self::Flux3Klein | Self::RoremMixed
        )
    }

    /// How many jobs the model takes in one batch.
    pub(crate) fn batch(&self) -> usize {
        match self {
            Self::Translator { .. } => TRANSLATION_SIZE.sequences as usize,
            _ => usize::MAX,
        }
    }
}

#[expect(
    clippy::large_enum_variant,
    reason = "a model loads into its slot once and stays there"
)]
pub(crate) enum Loaded {
    Layout(KoharuLayoutHighres),
    MangaOcr(MangaOcr),
    BaberuOcr(BaberuOcr),
    HayaiOcr(HayaiOcr),
    PaddleOcrVl(PaddleOcrVlQuantized),
    Translator(koharu_llm::Model),
    Lama(Lama),
    Aot(AotInpainting),
    Flux3Klein(Flux3Klein),
    RoremMixed(RoremMixed),
}

impl Loaded {
    async fn load(model: &Model, devices: &'static Devices, budget: u64) -> Result<Self> {
        let torch = &devices.torch;
        Ok(match model {
            Model::Layout => Self::Layout(KoharuLayoutHighres::load(torch).await?),
            Model::MangaOcr => Self::MangaOcr(MangaOcr::load(torch).await?),
            Model::BaberuOcr => Self::BaberuOcr(BaberuOcr::load(torch).await?),
            Model::HayaiOcr => Self::HayaiOcr(HayaiOcr::load(torch).await?),
            Model::PaddleOcrVl => {
                Self::PaddleOcrVl(PaddleOcrVlQuantized::load(OCR_SEQUENCES).await?)
            }
            Model::Translator {
                model,
                quantization,
            } => {
                let selection = koharu_llm::Selection {
                    provider: koharu_llm::Provider::Local,
                    model: model.clone(),
                    quantization: quantization.clone(),
                };
                Self::Translator(koharu_llm::model(&selection, TRANSLATION_SIZE).await?)
            }
            Model::Lama => Self::Lama(Lama::load(torch).await?),
            Model::Aot => Self::Aot(AotInpainting::load(torch).await?),
            Model::Flux3Klein => Self::Flux3Klein(Flux3Klein::load(&devices.ggml, budget).await?),
            Model::RoremMixed => Self::RoremMixed(RoremMixed::load(&devices.ggml, budget).await?),
        })
    }

    /// The model's first inference, at the largest input it takes, so its
    /// memory grows now rather than partway through a page.
    async fn warm_up(&self) -> Result<()> {
        let page = DynamicImage::new_rgb8(2048, 2048);
        let crop = DynamicImage::new_rgb8(256, 1024);
        let tile = DynamicImage::new_rgb8(768, 768);
        let mut mask = GrayImage::new(768, 768);
        mask.put_pixel(384, 384, Luma([255]));
        match self {
            Self::Layout(model) => {
                drop(model.inference(&page, &koharu_layout_highres::Options::default())?);
            }
            Self::MangaOcr(model) => drop(model.inference(&[crop])?),
            Self::BaberuOcr(model) => drop(model.inference(&[crop])?),
            Self::HayaiOcr(model) => drop(model.inference(&[crop])?),
            Self::PaddleOcrVl(model) => {
                drop(
                    model
                        .inference(&[crop], paddle_ocr_vl_quantized::Task::Ocr)
                        .await?,
                );
            }
            Self::Translator(_) => {}
            Self::Lama(model) => drop(model.inference(&tile, &mask, &lama::Options::default())?),
            Self::Aot(model) => {
                drop(model.inference(&tile, &mask, &aot_inpainting::Options::default())?);
            }
            // stable-diffusion.cpp frees a generation's compute buffers when it
            // ends, so a warm-up would cost a whole generation and leave the
            // footprint where loading left it.
            Self::Flux3Klein(_) | Self::RoremMixed(_) => {}
        }
        Ok(())
    }
}

struct Slot {
    model: Model,
    loaded: Loaded,
    last_used: Instant,
}

pub(crate) struct Residency {
    devices: &'static Devices,
    slots: Vec<Slot>,
    /// Each model's footprint, measured at its first load and kept until
    /// Koharu exits.
    footprints: BTreeMap<Model, u64>,
}

impl Residency {
    pub(crate) fn new(devices: &'static Devices) -> Self {
        Self {
            devices,
            slots: Vec::new(),
            footprints: BTreeMap::new(),
        }
    }

    /// The device whose memory models are measured and placed in: ggml's,
    /// the GPU, which libtorch shares unless it runs on the CPU, where its
    /// models take none of it.
    fn gpu(&self) -> &'static Device {
        &self.devices.ggml
    }

    pub(crate) fn is_loaded(&self, model: &Model) -> bool {
        self.slots.iter().any(|slot| slot.model == *model)
    }

    /// `model`, loaded first if it isn't, after making room for it. `next_use`
    /// orders the models a run still needs, sooner first; `None` is a model
    /// it doesn't need.
    pub(crate) async fn get<K: Ord>(
        &mut self,
        model: &Model,
        next_use: impl Fn(&Model) -> Option<K>,
    ) -> Result<&mut Loaded> {
        let now = Instant::now();
        if let Some(index) = self.slots.iter().position(|slot| slot.model == *model) {
            self.slots[index].last_used = now;
            return Ok(&mut self.slots[index].loaded);
        }
        self.make_room(model, &next_use).await?;
        let before = self.gpu().memory().free;
        let loaded = Loaded::load(model, self.devices, before.saturating_sub(RESERVE)).await?;
        loaded.warm_up().await?;
        let footprint = before.saturating_sub(self.gpu().memory().free);
        self.footprints.entry(model.clone()).or_insert(footprint);
        self.slots.push(Slot {
            model: model.clone(),
            loaded,
            last_used: Instant::now(),
        });
        Ok(&mut self.slots.last_mut().expect("just pushed").loaded)
    }

    /// Unloads idle models until free memory, less the reserve, holds
    /// `model`'s footprint. A model never loaded has no footprint yet, so
    /// every idle model that gives memory back makes way for it. With no
    /// room left to make, the load doesn't start.
    async fn make_room<K: Ord>(
        &mut self,
        model: &Model,
        next_use: &impl Fn(&Model) -> Option<K>,
    ) -> Result<()> {
        let footprint = self.footprints.get(model).copied();
        loop {
            let room = self.gpu().memory().free.saturating_sub(RESERVE);
            if footprint.is_some_and(|footprint| room >= footprint) {
                return Ok(());
            }
            let Some(victim) = self.victim(next_use) else {
                return match footprint {
                    None => Ok(()),
                    Some(footprint) => bail!(
                        "{model:?} needs {} MiB of device memory and only {} MiB is free past the reserve",
                        footprint >> 20,
                        room >> 20
                    ),
                };
            };
            self.unload(victim).await;
        }
    }

    /// The idle model needed furthest ahead, then the least recently used.
    fn victim<K: Ord>(&self, next_use: &impl Fn(&Model) -> Option<K>) -> Option<usize> {
        self.slots
            .iter()
            .enumerate()
            .filter(|(_, slot)| slot.model.evictable())
            .max_by(|(_, left), (_, right)| {
                let later = match (next_use(&left.model), next_use(&right.model)) {
                    (None, None) => Ordering::Equal,
                    (None, Some(_)) => Ordering::Greater,
                    (Some(_), None) => Ordering::Less,
                    (Some(left), Some(right)) => left.cmp(&right),
                };
                later.then(right.last_used.cmp(&left.last_used))
            })
            .map(|(index, _)| index)
    }

    /// Unloads the model at `index` and waits for its memory to show free.
    async fn unload(&mut self, index: usize) {
        let slot = self.slots.swap_remove(index);
        let before = self.gpu().memory().free;
        let expected = self.footprints.get(&slot.model).copied().unwrap_or(0);
        drop(slot);
        let deadline = Instant::now() + RECOVERY_WAIT;
        while Instant::now() < deadline
            && (self.gpu().memory().free.saturating_sub(before) as f64)
                < expected as f64 * RECOVERY_SHARE
        {
            tokio::time::sleep(RECOVERY_POLL).await;
        }
    }

    /// Unloads every model idle for `IDLE`, and says when the next one will
    /// be.
    pub(crate) fn expire(&mut self, now: Instant) -> Option<Instant> {
        self.slots.retain(|slot| now < slot.last_used + IDLE);
        self.slots.iter().map(|slot| slot.last_used + IDLE).min()
    }
}
