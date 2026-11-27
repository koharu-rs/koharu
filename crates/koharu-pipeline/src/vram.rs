//! App-global VRAM footprint measurements persisted at `~/.koharu/vram.json`.

use std::{
    collections::HashMap,
    ffi::OsString,
    io::Write as _,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result, bail};
use atomicwrites::{AtomicFile, OverwriteBehavior};
use fs_err::{File, OpenOptions};
use koharu_runtime::{Device, PinnedFile};
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use tokio::runtime::Handle;

use crate::Stage;

/// Headroom kept free when planning model loads and inferences (512 MiB).
pub const SAFETY_MARGIN: u64 = 512 * 1024 * 1024;

const FORMAT_VERSION: u32 = 1;
const MIN_CROP_MP: f64 = 0.25;

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct FootprintKey {
    pub device: String,
    pub model_key: String,
    pub revision: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct Footprint {
    pub load_bytes: u64,
    pub peak_working_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inpaint_bytes_per_mp: Option<u64>,
    pub samples: u32,
}

impl Footprint {
    /// Estimated total VRAM required for loading and running this model
    /// (`load_bytes + working_estimate(crop_mp)`).
    #[must_use]
    pub fn estimate(&self, crop_mp: Option<f64>) -> u64 {
        self.load_bytes
            .saturating_add(self.working_estimate(crop_mp))
    }

    /// Estimated working VRAM needed on top of already-resident weights.
    #[must_use]
    pub fn working_estimate(&self, crop_mp: Option<f64>) -> u64 {
        match (crop_mp, self.inpaint_bytes_per_mp) {
            (Some(mp), Some(per_mp)) if mp.is_finite() => {
                let scaled = ((per_mp as f64) * mp.max(MIN_CROP_MP)).ceil();
                if scaled.is_finite() && scaled >= 0.0 {
                    scaled as u64
                } else {
                    self.peak_working_bytes
                }
            }
            _ => self.peak_working_bytes,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct VramFile {
    version: u32,
    entries: Vec<VramEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct VramEntry {
    #[serde(flatten)]
    key: FootprintKey,
    #[serde(flatten)]
    footprint: Footprint,
}

/// Canonical device identifier used in `vram.json`.
#[must_use]
pub fn device_key(device: &Device) -> String {
    let backend: &'static str = device.backend.into();
    format!("{backend}:{}:{}", device.name, device.description)
}

/// Normalizes default local LLM references (`koharu/<id>`) to their default
/// quantization (`koharu/<id>:<default_quant>`) so both spellings share one
/// footprint entry.
#[must_use]
pub fn canonical_translate_reference(reference: &str, vision: bool) -> String {
    if let Some(rest) = reference.strip_prefix("koharu/")
        && !rest.contains(':')
        && let Some(local) = koharu_llm::local(reference, vision)
        && let Some(default_quant) = local.quantizations.first()
    {
        return format!("{reference}:{}", default_quant.id);
    }
    reference.to_owned()
}

/// Canonical model identifier used in `vram.json` across `gpu.rs` and
/// `list_model_catalog`.
#[must_use]
pub fn model_key(stage: Stage, id: &str, vision: bool) -> String {
    match stage {
        Stage::Detect | Stage::Recognize | Stage::Inpaint => format!("{stage}:{id}"),
        Stage::Translate => {
            let canonical = canonical_translate_reference(id, vision);
            if vision {
                format!("translate:{canonical}:vision")
            } else {
                format!("translate:{canonical}")
            }
        }
    }
}

/// Deterministic revision string for a model's pinned Hugging Face files.
/// Multi-repo models preserve unique commit hashes in order, joined by `+`.
#[must_use]
pub fn revision_for_files(files: &[PinnedFile]) -> String {
    let mut commits: Vec<&'static str> = Vec::new();
    for file in files {
        let commit = file.repo.commit();
        if !commits.contains(&commit) {
            commits.push(commit);
        }
    }
    commits.join("+")
}

/// Builds the canonical [`FootprintKey`] for a device, stage, model id/reference,
/// vision flag, and pinned file list.
#[must_use]
pub fn footprint_key(
    device: &Device,
    stage: Stage,
    id: &str,
    vision: bool,
    files: &[PinnedFile],
) -> FootprintKey {
    FootprintKey {
        device: device_key(device),
        model_key: model_key(stage, id, vision),
        revision: revision_for_files(files),
    }
}

/// Thread-safe, cross-process locked store for `~/.koharu/vram.json`.
#[derive(Clone)]
pub struct VramStore {
    inner: Arc<StoreInner>,
}

struct StoreInner {
    path: Option<PathBuf>,
    entries: RwLock<HashMap<FootprintKey, Footprint>>,
    io_lock: Mutex<()>,
}

impl VramStore {
    /// Default persistence path (`~/.koharu/vram.json`).
    #[must_use]
    pub fn default_path() -> Option<PathBuf> {
        dirs::home_dir().map(|home| home.join(".koharu").join("vram.json"))
    }

    /// Opens the default store at `~/.koharu/vram.json`, falling back to an
    /// in-memory store if the home directory is unavailable.
    pub fn open_default() -> Result<Self> {
        match Self::default_path() {
            Some(path) => Self::open(path),
            None => Ok(Self::in_memory()),
        }
    }

    /// Opens a store backed by `path`, reading any existing entries.
    /// Fails closed if `path` exists and contains invalid JSON or an
    /// unsupported format version.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let entries = read_entries_if_exists(&path)?;
        Ok(Self {
            inner: Arc::new(StoreInner {
                path: Some(path),
                entries: RwLock::new(entries),
                io_lock: Mutex::new(()),
            }),
        })
    }

    /// Creates an in-memory-only store (useful for tests or environments without
    /// a home directory).
    #[must_use]
    pub fn in_memory() -> Self {
        Self {
            inner: Arc::new(StoreInner {
                path: None,
                entries: RwLock::new(HashMap::new()),
                io_lock: Mutex::new(()),
            }),
        }
    }

    /// Looks up a recorded footprint for `key`.
    #[must_use]
    pub fn get(&self, key: &FootprintKey) -> Option<Footprint> {
        self.inner.entries.read().get(key).cloned()
    }

    /// Convenience helper returning estimated VRAM bytes for a catalog entry.
    #[must_use]
    pub fn measured_bytes(
        &self,
        device: &Device,
        stage: Stage,
        id: &str,
        vision: bool,
        files: &[PinnedFile],
    ) -> Option<u64> {
        self.get(&footprint_key(device, stage, id, vision, files))
            .map(|fp| fp.estimate(None))
    }

    /// Records a load VRAM delta (`load_bytes > 0`) and persists it.
    /// When `runtime` is `Some`, disk persistence runs off the calling thread
    /// via `spawn_blocking`.
    pub fn record_load(
        &self,
        key: FootprintKey,
        load_bytes: u64,
        allow_replace: bool,
        runtime: Option<&Handle>,
    ) {
        if load_bytes == 0 {
            return;
        }
        {
            let mut entries = self.inner.entries.write();
            let entry = entries.entry(key.clone()).or_default();
            entry.load_bytes = if allow_replace || entry.load_bytes == 0 {
                load_bytes
            } else {
                entry.load_bytes.max(load_bytes)
            };
            entry.samples = entry.samples.saturating_add(1);
        }
        self.persist(key, runtime);
    }

    /// Records an inference working-VRAM delta (`working_bytes > 0`), optionally
    /// updating `inpaint_bytes_per_mp` when `crop_mp` is given, and persists it.
    pub fn record_inference(
        &self,
        key: FootprintKey,
        working_bytes: u64,
        crop_mp: Option<f64>,
        runtime: Option<&Handle>,
    ) {
        if working_bytes == 0 {
            return;
        }
        {
            let mut entries = self.inner.entries.write();
            let entry = entries.entry(key.clone()).or_default();
            entry.peak_working_bytes = entry.peak_working_bytes.max(working_bytes);
            if let Some(mp) = crop_mp
                && mp.is_finite()
            {
                let per_mp = ((working_bytes as f64) / mp.max(MIN_CROP_MP)).ceil();
                if per_mp.is_finite() && per_mp >= 0.0 {
                    let per_mp = per_mp as u64;
                    entry.inpaint_bytes_per_mp =
                        Some(entry.inpaint_bytes_per_mp.map_or(per_mp, |p| p.max(per_mp)));
                }
            }
            entry.samples = entry.samples.saturating_add(1);
        }
        self.persist(key, runtime);
    }

    fn persist(&self, key: FootprintKey, runtime: Option<&Handle>) {
        if self.inner.path.is_none() {
            return;
        }
        let store = self.clone();
        let write = move || {
            if let Err(error) = store.persist_key_sync(&key) {
                tracing::warn!("failed to persist VRAM footprint: {error:#}");
            }
        };
        match runtime {
            Some(handle) => {
                handle.spawn_blocking(write);
            }
            None => write(),
        }
    }

    fn persist_key_sync(&self, key: &FootprintKey) -> Result<()> {
        let Some(path) = &self.inner.path else {
            return Ok(());
        };
        let _guard = self.inner.io_lock.lock();
        let Some(footprint) = self.inner.entries.read().get(key).cloned() else {
            return Ok(());
        };
        let _file_lock = lock(path)?;
        let mut disk = match read_entries_if_exists(path) {
            Ok(entries) => entries,
            Err(error) => {
                tracing::warn!(
                    "ignoring unreadable {} while updating VRAM footprint: {error:#}",
                    path.display()
                );
                HashMap::new()
            }
        };
        disk.insert(key.clone(), footprint);
        let mut entries: Vec<VramEntry> = disk
            .into_iter()
            .map(|(key, footprint)| VramEntry { key, footprint })
            .collect();
        entries.sort_by(|left, right| left.key.cmp(&right.key));
        let contents = serde_json::to_vec_pretty(&VramFile {
            version: FORMAT_VERSION,
            entries,
        })?;
        save(path, &contents)
    }
}

fn read_entries_if_exists(path: &Path) -> Result<HashMap<FootprintKey, Footprint>> {
    if !fs_err::exists(path)? {
        return Ok(HashMap::new());
    }
    let bytes = fs_err::read(path)?;
    let parsed: VramFile = serde_json::from_slice(&bytes)
        .with_context(|| format!("invalid VRAM cache at {}", path.display()))?;
    if parsed.version != FORMAT_VERSION {
        bail!(
            "unsupported VRAM cache version {} at {}",
            parsed.version,
            path.display()
        );
    }
    Ok(parsed
        .entries
        .into_iter()
        .map(|entry| (entry.key, entry.footprint))
        .collect())
}

/// Holds `path` against every other save of it, in this process or another,
/// until the returned file is dropped. Matches `koharu-config`'s lock scheme.
fn lock(path: &Path) -> Result<File> {
    if let Some(parent) = path.parent() {
        fs_err::create_dir_all(parent)?;
    }
    let mut name = OsString::from(".");
    name.push(path.file_name().unwrap_or_default());
    name.push(".lock");
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path.with_file_name(name))?;
    file.lock()?;
    Ok(file)
}

fn save(path: &Path, contents: &[u8]) -> Result<()> {
    AtomicFile::new(path, OverwriteBehavior::AllowOverwrite)
        .write(|file| file.write_all(contents))
        .with_context(|| format!("failed to write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use koharu_runtime::{Backend, Repository};

    use super::*;

    fn test_device() -> Device {
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
    fn footprint_estimate_scales_inpaint_working_bytes_with_floor() {
        let fp = Footprint {
            load_bytes: 1_000_000_000,
            peak_working_bytes: 500_000_000,
            inpaint_bytes_per_mp: Some(400_000_000),
            samples: 2,
        };
        // No crop_mp -> uses peak_working_bytes.
        assert_eq!(fp.working_estimate(None), 500_000_000);
        assert_eq!(fp.estimate(None), 1_500_000_000);
        // Very small crop -> clamped to 0.25 MP minimum.
        assert_eq!(fp.working_estimate(Some(0.05)), 100_000_000);
        assert_eq!(fp.estimate(Some(0.05)), 1_100_000_000);
        // Larger crop -> scales linearly.
        assert_eq!(fp.working_estimate(Some(1.5)), 600_000_000);
        assert_eq!(fp.estimate(Some(1.5)), 1_600_000_000);
    }

    #[test]
    fn model_key_canonicalizes_default_quant_and_separates_vision() {
        let default_implicit = model_key(Stage::Translate, "koharu/gemma4-e2b-it", false);
        let default_explicit = model_key(Stage::Translate, "koharu/gemma4-e2b-it:Q4_K_XL", false);
        let other_quant = model_key(Stage::Translate, "koharu/gemma4-e2b-it:Q2_K_XL", false);
        let with_vision = model_key(Stage::Translate, "koharu/gemma4-e2b-it", true);

        assert_eq!(default_implicit, default_explicit);
        assert_eq!(default_implicit, "translate:koharu/gemma4-e2b-it:Q4_K_XL");
        assert_ne!(default_implicit, other_quant);
        assert_eq!(with_vision, "translate:koharu/gemma4-e2b-it:Q4_K_XL:vision");
        assert_eq!(
            model_key(Stage::Inpaint, "flux3-klein", false),
            "inpaint:flux3-klein"
        );
    }

    #[test]
    fn revision_for_files_deduplicates_multi_repo_commits_preserving_order() {
        let files = koharu_ml::files_for(koharu_ml::Model::Flux3Klein);
        let rev = revision_for_files(&files);
        let parts: Vec<&str> = rev.split('+').collect();
        assert_eq!(parts.len(), 3);
        let c1 = "1111111111111111111111111111111111111111";
        let c2 = "2222222222222222222222222222222222222222";
        let single = revision_for_files(&[
            PinnedFile::new(Repository::model("a/b", c1), "f1", 10),
            PinnedFile::new(Repository::model("a/b", c1), "f2", 20),
            PinnedFile::new(Repository::model("c/d", c2), "f3", 30),
        ]);
        assert_eq!(single, format!("{c1}+{c2}"));
    }

    #[test]
    fn vram_store_persists_atomically_under_lock_and_merges_updates() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("vram.json");
        let dev = test_device();
        let key_a = FootprintKey {
            device: device_key(&dev),
            model_key: "detect:koharu-layout-highres".to_owned(),
            revision: "rev-a".to_owned(),
        };
        let key_b = FootprintKey {
            device: device_key(&dev),
            model_key: "inpaint:flux3-klein".to_owned(),
            revision: "rev-b".to_owned(),
        };

        let store1 = VramStore::open(&path)?;
        store1.record_load(key_a.clone(), 300_000_000, false, None);
        store1.record_inference(key_a.clone(), 150_000_000, None, None);

        // Concurrent handle writes a different key; merging under lock preserves key_a.
        let store2 = VramStore::open(&path)?;
        store2.record_load(key_b.clone(), 4_000_000_000, false, None);
        store2.record_inference(key_b.clone(), 1_000_000_000, Some(0.5), None);

        let reopened = VramStore::open(&path)?;
        let fp_a = reopened.get(&key_a).expect("key_a persisted");
        assert_eq!(fp_a.load_bytes, 300_000_000);
        assert_eq!(fp_a.peak_working_bytes, 150_000_000);
        assert_eq!(fp_a.samples, 2);

        let fp_b = reopened.get(&key_b).expect("key_b persisted");
        assert_eq!(fp_b.load_bytes, 4_000_000_000);
        assert_eq!(fp_b.inpaint_bytes_per_mp, Some(2_000_000_000));
        assert!(path.with_file_name(".vram.json.lock").exists());
        Ok(())
    }

    #[test]
    fn vram_store_fails_closed_on_corrupt_json_or_wrong_version() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("vram.json");
        fs_err::write(&path, b"{not json")?;
        assert!(VramStore::open(&path).is_err());

        fs_err::write(&path, br#"{"version": 99, "entries": []}"#)?;
        assert!(VramStore::open(&path).is_err());
        Ok(())
    }

    #[test]
    fn working_estimate_and_record_inference_ignore_non_finite_crop_mp() {
        let fp = Footprint {
            load_bytes: 1_000_000_000,
            peak_working_bytes: 500_000_000,
            inpaint_bytes_per_mp: Some(400_000_000),
            samples: 2,
        };
        assert_eq!(fp.working_estimate(Some(f64::NAN)), 500_000_000);
        assert_eq!(fp.working_estimate(Some(f64::INFINITY)), 500_000_000);
        assert_eq!(fp.working_estimate(Some(f64::NEG_INFINITY)), 500_000_000);

        let store = VramStore::in_memory();
        let key = FootprintKey {
            device: device_key(&test_device()),
            model_key: "inpaint:flux3-klein".to_owned(),
            revision: "rev-nan".to_owned(),
        };
        store.record_inference(key.clone(), 200_000_000, Some(f64::NAN), None);
        store.record_inference(key.clone(), 300_000_000, Some(f64::INFINITY), None);
        let recorded = store.get(&key).expect("entry recorded");
        assert_eq!(recorded.peak_working_bytes, 300_000_000);
        assert_eq!(recorded.inpaint_bytes_per_mp, None);
        assert_eq!(recorded.samples, 2);
    }

    #[test]
    fn vram_store_async_persist_converges_to_final_in_memory_state() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("vram.json");
        let key = FootprintKey {
            device: device_key(&test_device()),
            model_key: "inpaint:flux3-klein".to_owned(),
            revision: "rev-async".to_owned(),
        };
        let store = VramStore::open(&path)?;

        let rt = tokio::runtime::Runtime::new()?;
        for step in 1_u64..=25 {
            store.record_load(key.clone(), step * 100_000_000, false, Some(rt.handle()));
            store.record_inference(key.clone(), step * 25_000_000, Some(1.0), Some(rt.handle()));
        }
        rt.shutdown_timeout(std::time::Duration::from_secs(5));

        let mem = store.get(&key).expect("in-memory footprint present");
        let reopened = VramStore::open(&path)?;
        let disk = reopened.get(&key).expect("disk footprint present");
        assert_eq!(disk, mem);
        assert_eq!(disk.samples, 50);
        assert_eq!(disk.load_bytes, 2_500_000_000);
        assert_eq!(disk.peak_working_bytes, 625_000_000);
        Ok(())
    }
}
