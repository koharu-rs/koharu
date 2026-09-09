use std::{sync::Arc, time::Duration};

use crate::{Stage, resources::ResourceMonitor, stages::Stages};

pub(crate) struct AcceleratorGate {
    resources: Arc<ResourceMonitor>,
    lane: Option<Arc<tokio::sync::Semaphore>>,
}

pub(crate) struct AcceleratorPermit {
    _lane: Option<tokio::sync::OwnedSemaphorePermit>,
}

impl AcceleratorGate {
    pub(crate) fn new(device: &koharu_ml::Device, resources: Arc<ResourceMonitor>) -> Self {
        let accelerated = device.backend != koharu_ml::Backend::Cpu
            && device.device_type != koharu_ml::DeviceType::Cpu;
        Self {
            resources,
            // Heterogeneous CUDA model pairs took 2.5-4x longer together than
            // back-to-back on the target workload. Keep accelerator work
            // serialized while loaded weights remain resident between pages.
            lane: accelerated.then(|| Arc::new(tokio::sync::Semaphore::new(1))),
        }
    }

    pub(crate) async fn acquire(&self) -> AcceleratorPermit {
        let Some(lane) = self.lane.as_ref() else {
            return AcceleratorPermit::cpu();
        };
        AcceleratorPermit::accelerator(
            lane.clone()
                .acquire_owned()
                .await
                .expect("accelerator lane is never closed"),
        )
    }

    pub(crate) async fn recover(&self, stage: Stage, stages: &Stages) -> AcceleratorPermit {
        let permit = self.acquire().await;
        if self.lane.is_none() {
            return permit;
        }
        if unload_other_models(stage, stages) {
            let mut changed = self.resources.subscribe();
            let _ = tokio::time::timeout(Duration::from_millis(600), changed.changed()).await;
        }
        permit
    }
}

impl AcceleratorPermit {
    fn accelerator(lane: tokio::sync::OwnedSemaphorePermit) -> Self {
        Self { _lane: Some(lane) }
    }

    fn cpu() -> Self {
        Self { _lane: None }
    }
}

fn unload_other_models(requested: Stage, stages: &Stages) -> bool {
    let mut unloaded = false;
    for stage in Stage::ALL {
        if stage != requested && stages.unload(stage) {
            unloaded = true;
            tracing::info!(target: "koharu_metrics", metric = "model_unload", stage = %stage);
            tracing::debug!(%stage, "unloaded model while recovering from memory pressure");
        }
    }
    unloaded
}
