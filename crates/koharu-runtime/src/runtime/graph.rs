use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result, anyhow};
use petgraph::{
    algo::toposort,
    graph::{DiGraph, NodeIndex},
};

use crate::{
    Device, Hardware,
    runtime::{
        DiscoverablePackage, RuntimePackage,
        packages::{Cuda, Diffusion, Llama, Rocm, Torch},
    },
};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum Component {
    Cuda(Cuda),
    Rocm(Rocm),
    Torch(Torch),
    Llama(Llama),
    Diffusion(Diffusion),
}

impl Component {
    fn dependencies(self, hardware: &Hardware) -> Result<Vec<Self>> {
        match self {
            Self::Cuda(package) => package.dependencies(hardware),
            Self::Rocm(package) => package.dependencies(hardware),
            Self::Torch(package) => package.dependencies(hardware),
            Self::Llama(package) => package.dependencies(hardware),
            Self::Diffusion(package) => package.dependencies(hardware),
        }
    }

    async fn activate(self) -> Result<()> {
        match self {
            Self::Cuda(package) => activate(package).await,
            Self::Rocm(package) => activate(package).await,
            Self::Torch(package) => activate(package).await,
            Self::Llama(package) => activate(package).await,
            Self::Diffusion(package) => activate(package).await,
        }
    }

    fn label(self) -> String {
        match self {
            Self::Cuda(package) => package.label(),
            Self::Rocm(package) => package.label(),
            Self::Torch(package) => package.label(),
            Self::Llama(package) => package.label(),
            Self::Diffusion(package) => package.label(),
        }
    }
}

async fn activate<P: RuntimePackage>(package: P) -> Result<()> {
    package
        .activate()
        .await
        .with_context(|| format!("failed to activate {}", package.label()))
}

impl From<Cuda> for Component {
    fn from(value: Cuda) -> Self {
        Self::Cuda(value)
    }
}

impl From<Rocm> for Component {
    fn from(value: Rocm) -> Self {
        Self::Rocm(value)
    }
}

impl From<Torch> for Component {
    fn from(value: Torch) -> Self {
        Self::Torch(value)
    }
}

impl From<Llama> for Component {
    fn from(value: Llama) -> Self {
        Self::Llama(value)
    }
}

impl From<Diffusion> for Component {
    fn from(value: Diffusion) -> Self {
        Self::Diffusion(value)
    }
}

#[derive(Debug, Default)]
pub(crate) struct Plan {
    graph: DiGraph<Component, ()>,
    nodes: HashMap<Component, NodeIndex>,
    expanded: HashSet<Component>,
}

impl Plan {
    pub(crate) fn require<P>(&mut self, hardware: &Hardware) -> Result<Option<NodeIndex>>
    where
        P: DiscoverablePackage,
        Component: From<P>,
    {
        P::discover(hardware)
            .map(|package| self.insert(package.into(), hardware))
            .transpose()
    }

    fn insert(&mut self, component: Component, hardware: &Hardware) -> Result<NodeIndex> {
        let node = *self
            .nodes
            .entry(component)
            .or_insert_with(|| self.graph.add_node(component));
        if !self.expanded.insert(component) {
            return Ok(node);
        }

        let mut previous = None;
        for dependency in component.dependencies(hardware)? {
            let dependency = self.insert(dependency, hardware)?;
            self.graph.update_edge(dependency, node, ());
            if let Some(previous) = previous {
                self.graph.update_edge(previous, dependency, ());
            }
            previous = Some(dependency);
        }
        Ok(node)
    }

    pub(crate) fn sequence(&mut self, before: NodeIndex, after: NodeIndex) {
        self.graph.update_edge(before, after, ());
    }

    pub(crate) async fn initialize(&self, mut device: Device) -> Result<Device> {
        let order = toposort(&self.graph, None).map_err(|cycle| {
            anyhow!(
                "runtime dependency cycle contains {}",
                self.graph[cycle.node_id()].label()
            )
        })?;
        for node in order {
            let component = self.graph[node];
            component.activate().await?;
            if let Component::Rocm(rocm) = component {
                device.index = rocm.probe().await?;
                device.name = format!("ROCm{}", device.index);
            }
        }
        Ok(device)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_dependencies_are_deduplicated() {
        let hardware = Hardware {
            devices: vec![crate::Device {
                index: 0,
                name: "CUDA0".to_owned(),
                description: "CUDA0".to_owned(),
                backend: crate::Backend::Cuda,
                device_type: crate::DeviceType::Gpu,
                memory_total: 0,
                memory_free: 0,
                compute_capability: 80,
                target: None,
            }],
            candidates: vec![0],
            selected: Some(0),
        };
        let mut plan = Plan::default();
        let llama = plan.insert(Llama::WindowsCuda.into(), &hardware).unwrap();
        let diffusion = plan
            .insert(Diffusion::WindowsCuda.into(), &hardware)
            .unwrap();
        plan.sequence(llama, diffusion);

        assert_eq!(
            plan.graph
                .node_weights()
                .filter(|node| **node == Component::Cuda(Cuda::Runtime13))
                .count(),
            1
        );
    }
}
