//! The models, each loaded the first time a step needs it and kept until the
//! settings switch away from it, `unload` is called, or the pipeline ends.
//! Nothing is unloaded to make room: a model that doesn't fit fails, naming
//! the memory free when it began to load. One thread owns them and runs one
//! call at a time, as koharu-llm's engine thread owns its model, so a model
//! is never shared and needs no lock.

use std::{
    collections::{HashMap, HashSet},
    sync::mpsc,
};

use bytesize::ByteSize;
use image::{DynamicImage, GrayImage, RgbImage, imageops};
use koharu_llm::{
    KOHARU, Size,
    rig::{driver::DynModel, operation::Completion},
};
use koharu_ml::{
    baberu_ocr::BaberuOcr,
    flux3_klein::{self, Flux3Klein},
    hayai_ocr::HayaiOcr,
    koharu_layout_highres::{Detected, KoharuLayoutHighres, Options},
    lama::Lama,
    manga_ocr_ar::MangaOcrAr,
    moss_image::{self, MossImage},
    rorem_mixed::{self, RoremMixed},
};
use koharu_runtime::Device;
use koharu_torch::Cuda;
use tokio::runtime::Handle;

use crate::{Detector, Error, Inpainter, PipelineConfig, Recognizer, Result};

/// A language model's room: one conversation at a time, of text unless
/// translation shows it pages.
pub(crate) const LANGUAGE: Size = Size {
    sequences: 1,
    context: 32_768,
    images: false,
};

pub(crate) trait Detect {
    fn detect(&self, image: &DynamicImage, options: &Options) -> anyhow::Result<Detected>;
}

pub(crate) trait Recognize {
    fn recognize(&self, images: &[DynamicImage]) -> anyhow::Result<Vec<String>>;
}

/// `image` redrawn where `mask` is set, at its own size. `prompt` is empty
/// for a model that takes none.
pub(crate) trait Inpaint {
    fn inpaint(&self, image: &RgbImage, mask: &GrayImage, prompt: &str)
    -> anyhow::Result<RgbImage>;
}

impl Detect for KoharuLayoutHighres {
    fn detect(&self, image: &DynamicImage, options: &Options) -> anyhow::Result<Detected> {
        self.inference(image, options)
    }
}

impl Recognize for MangaOcrAr {
    fn recognize(&self, images: &[DynamicImage]) -> anyhow::Result<Vec<String>> {
        self.inference(images)
    }
}

impl Recognize for BaberuOcr {
    fn recognize(&self, images: &[DynamicImage]) -> anyhow::Result<Vec<String>> {
        self.inference(images)
    }
}

impl Recognize for HayaiOcr {
    fn recognize(&self, images: &[DynamicImage]) -> anyhow::Result<Vec<String>> {
        self.inference(images)
    }
}

impl Inpaint for Lama {
    fn inpaint(&self, image: &RgbImage, mask: &GrayImage, _: &str) -> anyhow::Result<RgbImage> {
        self.inference(&DynamicImage::ImageRgb8(image.clone()), mask)
    }
}

impl Inpaint for RoremMixed {
    fn inpaint(&self, image: &RgbImage, mask: &GrayImage, _: &str) -> anyhow::Result<RgbImage> {
        let options = rorem_mixed::Options::default();
        let redrawn = self.inference(&DynamicImage::ImageRgb8(image.clone()), mask, &options)?;
        Ok(fitted(redrawn, image))
    }
}

impl Inpaint for Flux3Klein {
    fn inpaint(
        &self,
        image: &RgbImage,
        mask: &GrayImage,
        prompt: &str,
    ) -> anyhow::Result<RgbImage> {
        let options = flux3_klein::InpaintOptions::default();
        let redrawn = Flux3Klein::inpaint(
            self,
            prompt,
            &DynamicImage::ImageRgb8(image.clone()),
            None,
            &DynamicImage::ImageLuma8(mask.clone()),
            &options,
        )?;
        Ok(fitted(redrawn, image))
    }
}

impl Inpaint for MossImage {
    fn inpaint(
        &self,
        image: &RgbImage,
        mask: &GrayImage,
        prompt: &str,
    ) -> anyhow::Result<RgbImage> {
        let options = moss_image::InpaintOptions::default();
        let redrawn = MossImage::inpaint(
            self,
            prompt,
            &DynamicImage::ImageRgb8(image.clone()),
            &DynamicImage::ImageLuma8(mask.clone()),
            &options,
        )?;
        Ok(fitted(redrawn, image))
    }
}

/// `redrawn` back at `image`'s size: the diffusion models draw on their own
/// grid.
fn fitted(redrawn: RgbImage, image: &RgbImage) -> RgbImage {
    let (width, height) = image.dimensions();
    if redrawn.dimensions() == (width, height) {
        redrawn
    } else {
        imageops::resize(&redrawn, width, height, imageops::FilterType::Lanczos3)
    }
}

/// Which model, as the settings name it.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) enum Key {
    Detector(Detector),
    Recognizer(Recognizer),
    Inpainter(Inpainter),
    /// A local language model's `koharu/model` reference, and whether it
    /// loads to read images.
    Translator(String, bool),
}

impl Key {
    /// The local models `config` chooses, which a switch of settings keeps.
    pub(crate) fn chosen(config: &PipelineConfig) -> impl Iterator<Item = Self> {
        let translator = local(&config.translator.model)
            .then(|| Self::Translator(config.translator.model.clone(), config.translator.vision));
        [
            Self::Detector(config.detector.model),
            Self::Recognizer(config.recognizer.model),
            Self::Inpainter(config.inpainter.model),
        ]
        .into_iter()
        .chain(translator)
    }

    fn name(&self) -> String {
        match self {
            Self::Detector(model) => model.to_string(),
            Self::Recognizer(model) => model.to_string(),
            Self::Inpainter(model) => model.to_string(),
            Self::Translator(reference, _) => reference.clone(),
        }
    }

    /// Whether it runs on ggml, which fits itself to the device's free
    /// memory.
    fn ggml(&self) -> bool {
        match self {
            Self::Inpainter(model) => model.ggml(),
            Self::Translator(..) => true,
            Self::Detector(_) | Self::Recognizer(_) => false,
        }
    }
}

/// Whether a `vendor/model` reference runs in Koharu, so on the GPU thread.
pub(crate) fn local(reference: &str) -> bool {
    reference
        .split_once('/')
        .is_some_and(|(vendor, _)| vendor == KOHARU)
}

enum Loaded {
    Detector(Box<dyn Detect + Send>),
    Recognizer(Box<dyn Recognize + Send>),
    Inpainter(Box<dyn Inpaint + Send>),
    Translator(DynModel<Completion>),
}

/// The loaded models, which the GPU thread owns.
pub(crate) struct Models {
    runtime: Handle,
    loaded: HashMap<Key, Loaded>,
}

impl Models {
    pub(crate) fn new(runtime: Handle) -> Self {
        Self {
            runtime,
            loaded: HashMap::new(),
        }
    }

    /// The runtime a step's async work runs on from this thread.
    pub(crate) fn runtime(&self) -> &Handle {
        &self.runtime
    }

    pub(crate) fn detector(&mut self, model: Detector) -> Result<&dyn Detect> {
        match self.get(Key::Detector(model))? {
            Loaded::Detector(model) => Ok(&**model),
            _ => unreachable!("a detector's key loads a detector"),
        }
    }

    pub(crate) fn recognizer(&mut self, model: Recognizer) -> Result<&dyn Recognize> {
        match self.get(Key::Recognizer(model))? {
            Loaded::Recognizer(model) => Ok(&**model),
            _ => unreachable!("a recognizer's key loads a recognizer"),
        }
    }

    pub(crate) fn inpainter(&mut self, model: Inpainter) -> Result<&dyn Inpaint> {
        match self.get(Key::Inpainter(model))? {
            Loaded::Inpainter(model) => Ok(&**model),
            _ => unreachable!("an inpainter's key loads an inpainter"),
        }
    }

    /// A local language model, with its projector when it's to see images.
    pub(crate) fn translator(
        &mut self,
        reference: &str,
        images: bool,
    ) -> Result<DynModel<Completion>> {
        match self.get(Key::Translator(reference.to_owned(), images))? {
            Loaded::Translator(model) => Ok(model.clone()),
            _ => unreachable!("a translator's key loads a translator"),
        }
    }

    /// Unloads every model `wanted` doesn't hold.
    pub(crate) fn retain(&mut self, wanted: &HashSet<Key>) {
        self.loaded.retain(|key, _| wanted.contains(key));
    }

    fn get(&mut self, key: Key) -> Result<&Loaded> {
        if !self.loaded.contains_key(&key) {
            let _span = tracing::info_span!("load", model = %key.name()).entered();
            let model = self.runtime.block_on(load(&key))?;
            self.loaded.insert(key.clone(), model);
        }
        Ok(&self.loaded[&key])
    }
}

async fn load(key: &Key) -> Result<Loaded> {
    let device = koharu_runtime::device()
        .await
        .map_err(|source| Error::Load {
            model: key.name(),
            device: "no device".to_owned(),
            free: ByteSize(0),
            reserved: ByteSize(0),
            active: ByteSize(0),
            source,
        })?;
    // libtorch's caching allocator keeps what its tensors freed, and the
    // device counts it as used, so ggml's fit wouldn't see it as free.
    let cuda = Cuda::device_count() > 0;
    if key.ggml() && cuda {
        Cuda::empty_cache().map_err(|source| failure(key, device, cuda, source.into()))?;
    }
    load_on(key, device)
        .await
        .map_err(|source| failure(key, device, cuda, source))
}

/// The load failure, with the memory free on `device` and libtorch's.
fn failure(key: &Key, device: &Device, cuda: bool, source: anyhow::Error) -> Error {
    let held = cuda
        .then(|| Cuda::memory(device.index as i64).ok())
        .flatten();
    Error::Load {
        model: key.name(),
        device: device.description.clone(),
        free: ByteSize(device.memory().free),
        reserved: ByteSize(held.map_or(0, |held| held.reserved)),
        active: ByteSize(held.map_or(0, |held| held.active)),
        source,
    }
}

async fn load_on(key: &Key, device: &Device) -> anyhow::Result<Loaded> {
    Ok(match key {
        Key::Detector(Detector::KoharuLayoutHighres) => {
            Loaded::Detector(Box::new(KoharuLayoutHighres::load(device).await?))
        }
        Key::Recognizer(Recognizer::MangaOcrAr) => {
            Loaded::Recognizer(Box::new(MangaOcrAr::load(device).await?))
        }
        Key::Recognizer(Recognizer::BaberuOcr) => {
            Loaded::Recognizer(Box::new(BaberuOcr::load(device).await?))
        }
        Key::Recognizer(Recognizer::HayaiOcr) => {
            Loaded::Recognizer(Box::new(HayaiOcr::load(device).await?))
        }
        Key::Inpainter(Inpainter::Lama) => Loaded::Inpainter(Box::new(Lama::load(device).await?)),
        Key::Inpainter(Inpainter::RoremMixed) => {
            Loaded::Inpainter(Box::new(RoremMixed::load(device).await?))
        }
        Key::Inpainter(Inpainter::Flux3Klein) => {
            Loaded::Inpainter(Box::new(Flux3Klein::load(device).await?))
        }
        Key::Inpainter(Inpainter::MossImage) => {
            Loaded::Inpainter(Box::new(MossImage::load(device).await?))
        }
        Key::Translator(reference, images) => {
            let size = Size {
                images: *images,
                ..LANGUAGE
            };
            Loaded::Translator(koharu_llm::connect(reference, size).await?)
        }
    })
}

/// Work for the GPU thread.
pub(crate) type Work = Box<dyn FnOnce(&mut Models) + Send>;

/// Starts the thread that owns the models. It ends, unloading them, when the
/// sender drops.
pub(crate) fn spawn(runtime: Handle) -> mpsc::Sender<Work> {
    let (sender, receiver) = mpsc::channel::<Work>();
    std::thread::Builder::new()
        .name("koharu-pipeline-gpu".to_owned())
        .spawn(move || {
            let mut models = Models::new(runtime);
            for work in receiver {
                work(&mut models);
            }
        })
        .expect("the OS starts a thread");
    sender
}
