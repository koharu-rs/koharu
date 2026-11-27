//! A run: what it works on, and its graph of jobs, a page's stage each, with
//! an edge for what each needs: detection before the page's other stages,
//! recognition before translation, and each page's translation before the
//! next page's. A job is ready when nothing points at it, and a finished job
//! leaves the graph.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use koharu_scene::{Content, LayerId, PageId, Project, ProjectId, Session};
use parking_lot::Mutex;
use petgraph::{
    Direction::{Incoming, Outgoing},
    stable_graph::{NodeIndex, StableDiGraph},
};
use tokio_util::sync::CancellationToken;

use super::Mode;
use crate::{
    Error, PipelineConfig, QueueScope, QueuedStep, Result, RunId, RunPriority, Stage, Target,
    stages::inpaint::AreaWork,
};

pub(crate) struct Run {
    pub(super) id: RunId,
    pub(super) mode: Mode,
    pub(super) priority: RunPriority,
    pub(super) scope: QueueScope,
    pub(super) stages: BTreeSet<Stage>,
    pub(super) session: Arc<Mutex<Session>>,
    pub(super) project: ProjectId,
    pub(super) config: Arc<PipelineConfig>,
    pub(super) area: Option<Arc<AreaWork>>,
    pub(super) graph: StableDiGraph<Job, ()>,
    pub(super) cancel: CancellationToken,
    pub(super) finished_steps: usize,
}

pub(super) struct Job {
    pub(super) page: Option<PageId>,
    /// The page's place in the run (or `pages.len()` for trailing metadata).
    pub(super) index: usize,
    pub(super) stage: Stage,
    pub(super) blocks: Option<Arc<[LayerId]>>,
    pub(super) running: bool,
    pub(super) cancelled: bool,
    pub(super) cancel: CancellationToken,
}

/// What a run works on: its pages in order, each with the blocks it's
/// limited to, plus whether to translate project metadata.
pub(crate) struct Plan {
    mode: Mode,
    pages: Vec<(PageId, Option<Arc<[LayerId]>>)>,
    area: Option<Arc<AreaWork>>,
    metadata: bool,
}

fn has_translatable_metadata(project: &Project) -> Result<bool> {
    let Some(value) = project.metadata(koharu_io::COMIC_INFO_KEY) else {
        return Ok(false);
    };
    let info: koharu_io::ComicInfo = serde_json::from_value(value.clone())
        .map_err(|err| Error::invalid(format!("stored ComicInfo metadata is invalid: {err}")))?;
    Ok(info.has_translatable_fields())
}

impl Plan {
    /// How many steps it runs.
    pub(crate) fn steps(&self, stages: usize) -> usize {
        self.pages.len() * stages + usize::from(self.metadata)
    }

    /// The planned steps in stage-batched execution order (all pages for
    /// `Detect`, then `Recognize`, then `Inpaint`, then `Translate`, followed
    /// by trailing metadata if scheduled).
    pub(crate) fn ordered_jobs(&self, stages: &BTreeSet<Stage>) -> Vec<QueuedStep> {
        let mut jobs = Vec::with_capacity(self.steps(stages.len()));
        for &stage in stages {
            for &(page, _) in &self.pages {
                jobs.push(QueuedStep {
                    page: Some(page),
                    stage,
                });
            }
            if stage == Stage::Translate && self.metadata {
                jobs.push(QueuedStep {
                    page: None,
                    stage: Stage::Translate,
                });
            }
        }
        jobs
    }

    /// What a run of `stages` on `target` at `priority` works on, or why it can't run.
    pub(crate) fn new(
        project: &Project,
        target: &Target,
        stages: &BTreeSet<Stage>,
        priority: RunPriority,
    ) -> Result<Self> {
        if stages.is_empty() {
            return Err(Error::invalid("a run needs a stage"));
        }
        let batch_mode = match priority {
            RunPriority::Normal => Mode::Batch,
            RunPriority::RerunNow => Mode::Interactive,
        };
        let pages = |pages: Vec<PageId>, metadata: bool| Self {
            mode: batch_mode,
            pages: pages.into_iter().map(|page| (page, None)).collect(),
            area: None,
            metadata,
        };
        Ok(match target {
            Target::Project => {
                let metadata =
                    stages.contains(&Stage::Translate) && has_translatable_metadata(project)?;
                pages(project.pages().map(|page| page.id).collect(), metadata)
            }
            Target::Pages(chosen) => {
                for page in chosen {
                    project.page(*page)?;
                }
                pages(
                    project
                        .pages()
                        .map(|page| page.id)
                        .filter(|page| chosen.contains(page))
                        .collect(),
                    false,
                )
            }
            Target::Blocks(blocks) => {
                if stages.contains(&Stage::Detect) {
                    return Err(Error::invalid("detection doesn't run on blocks"));
                }
                let mut held = BTreeMap::<PageId, Vec<LayerId>>::new();
                for &block in blocks {
                    let page = match project.layer(block).map(|layer| &layer.content) {
                        Ok(Content::Block(_)) => project.parent(block)?.0.id,
                        _ => return Err(Error::invalid(format!("layer {block} isn't a block"))),
                    };
                    held.entry(page).or_default().push(block);
                }
                Self {
                    mode: Mode::Interactive,
                    pages: project
                        .pages()
                        .filter_map(|page| Some((page.id, Some(held.remove(&page.id)?.into()))))
                        .collect(),
                    area: None,
                    metadata: false,
                }
            }
            Target::Area {
                page,
                area,
                prompt,
                layer,
            } => {
                if stages.iter().ne([Stage::Inpaint].iter()) {
                    return Err(Error::invalid("an area takes inpainting only"));
                }
                project.page(*page)?;
                if area.pixels.len() != area.rect.width as usize * area.rect.height as usize {
                    return Err(Error::invalid(format!(
                        "the area has {} pixels for a {}×{} rectangle",
                        area.pixels.len(),
                        area.rect.width,
                        area.rect.height
                    )));
                }
                let prompt = prompt
                    .as_deref()
                    .map(str::trim)
                    .filter(|prompt| !prompt.is_empty());
                Self {
                    mode: Mode::Interactive,
                    pages: vec![(*page, None)],
                    area: Some(Arc::new(AreaWork {
                        area: area.clone(),
                        prompt: prompt.map(str::to_owned),
                        layer: *layer,
                    })),
                    metadata: false,
                }
            }
            Target::Metadata => {
                if stages.iter().ne([Stage::Translate].iter()) {
                    return Err(Error::invalid("metadata takes translation only"));
                }
                Self {
                    mode: Mode::Interactive,
                    pages: Vec::new(),
                    area: None,
                    metadata: has_translatable_metadata(project)?,
                }
            }
        })
    }
}

/// Builds the stage dependency DAG for `pages` (each with its own stage set
/// and optional block filter) plus optional trailing metadata translation.
pub(super) fn build_graph(
    pages: &[(PageId, BTreeSet<Stage>, Option<Arc<[LayerId]>>)],
    metadata: bool,
    run_cancel: &CancellationToken,
) -> StableDiGraph<Job, ()> {
    let mut graph = StableDiGraph::new();
    let mut translated = None;
    let page_count = pages.len();
    for (index, (page, page_stages, blocks)) in pages.iter().enumerate() {
        let jobs: BTreeMap<Stage, NodeIndex> = page_stages
            .iter()
            .map(|&stage| {
                let job = Job {
                    page: Some(*page),
                    index,
                    stage,
                    blocks: blocks.clone(),
                    running: false,
                    cancelled: false,
                    cancel: run_cancel.child_token(),
                };
                (stage, graph.add_node(job))
            })
            .collect();
        if let Some(&detect) = jobs.get(&Stage::Detect) {
            for (_, &job) in jobs.range(Stage::Recognize..) {
                graph.add_edge(detect, job, ());
            }
        }
        if let Some(&translate) = jobs.get(&Stage::Translate) {
            if let Some(&recognize) = jobs.get(&Stage::Recognize) {
                graph.add_edge(recognize, translate, ());
            }
            if let Some(before) = translated.replace(translate) {
                graph.add_edge(before, translate, ());
            }
        }
    }
    if metadata {
        let metadata_node = graph.add_node(Job {
            page: None,
            index: page_count,
            stage: Stage::Translate,
            blocks: None,
            running: false,
            cancelled: false,
            cancel: run_cancel.child_token(),
        });
        if let Some(before) = translated {
            graph.add_edge(before, metadata_node, ());
        }
    }
    graph
}

impl Run {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        id: RunId,
        session: Arc<Mutex<Session>>,
        project: ProjectId,
        config: PipelineConfig,
        plan: Plan,
        stages: &BTreeSet<Stage>,
        priority: RunPriority,
        scope: QueueScope,
    ) -> Self {
        let pages: Vec<(PageId, BTreeSet<Stage>, Option<Arc<[LayerId]>>)> = plan
            .pages
            .into_iter()
            .map(|(page, blocks)| (page, stages.clone(), blocks))
            .collect();
        let cancel = CancellationToken::new();
        let graph = build_graph(&pages, plan.metadata, &cancel);
        Self {
            id,
            mode: plan.mode,
            priority,
            scope,
            stages: stages.clone(),
            session,
            project,
            config: Arc::new(config),
            area: plan.area,
            graph,
            cancel,
            finished_steps: 0,
        }
    }

    /// Remaining non-cancelled jobs in stage-batched execution order.
    pub(super) fn ordered_jobs(&self) -> Vec<QueuedStep> {
        let mut nodes: Vec<(Stage, usize, Option<PageId>)> = self
            .graph
            .node_weights()
            .filter(|job| !job.cancelled)
            .map(|job| (job.stage, job.index, job.page))
            .collect();
        nodes.sort_unstable_by_key(|&(stage, index, _)| (stage, index));
        nodes
            .into_iter()
            .map(|(stage, _, page)| QueuedStep { page, stage })
            .collect()
    }

    /// Removes `node` from the DAG, bridging incoming and outgoing
    /// `Stage::Translate` edges so remaining pages still translate in order.
    pub(super) fn remove_job_preserving_chain(&mut self, node: NodeIndex) -> Option<Job> {
        let stage = self.graph.node_weight(node)?.stage;
        if stage == Stage::Translate {
            let preds: Vec<NodeIndex> = self
                .graph
                .neighbors_directed(node, Incoming)
                .filter(|&n| self.graph[n].stage == Stage::Translate)
                .collect();
            let succs: Vec<NodeIndex> = self
                .graph
                .neighbors_directed(node, Outgoing)
                .filter(|&n| self.graph[n].stage == Stage::Translate)
                .collect();
            for pred in preds {
                for &succ in &succs {
                    if !self.graph.contains_edge(pred, succ) {
                        self.graph.add_edge(pred, succ, ());
                    }
                }
            }
        }
        self.graph.remove_node(node)
    }
}

#[cfg(test)]
mod tests {
    use koharu_scene::{Created, Edit, Rect};
    use petgraph::Direction::Incoming;
    use serde_json::json;

    use super::*;
    use crate::Area;

    /// A project of two pages, the second holding one block, in a folder
    /// removed with it.
    fn project() -> anyhow::Result<(tempfile::TempDir, Session, PageId, LayerId)> {
        let dir = tempfile::tempdir()?;
        let mut session =
            Session::create(dir.path().join("test.khr"), koharu_scene::Meta::default())?;
        let ops = [
            json!({"op": "add_page", "index": 0, "size": {"width": 100, "height": 100}}),
            json!({"op": "add_page", "id": "$p", "index": 1, "size": {"width": 100, "height": 100}}),
            json!({"op": "add_layer", "page": "$p", "index": 0, "layer": {"kind": "block", "source": {}}}),
        ];
        let change = session.apply(Edit {
            label: "test".to_owned(),
            ops: ops
                .into_iter()
                .map(serde_json::from_value)
                .collect::<Result<_, _>>()?,
        })?;
        let Some(Created::Layer { id: block, .. }) = change.created.get(2).cloned() else {
            anyhow::bail!("no block");
        };
        let page = session
            .project()
            .pages()
            .nth(1)
            .map(|page| page.id)
            .expect("a page");
        Ok((dir, session, page, block))
    }

    fn stages(stages: &[Stage]) -> BTreeSet<Stage> {
        stages.iter().copied().collect()
    }

    #[test]
    fn a_target_refuses_stages_that_dont_run_on_it() -> anyhow::Result<()> {
        let (_dir, session, page, block) = project()?;
        let project = session.project();
        let blocks = Target::Blocks(vec![block]);
        assert!(
            Plan::new(
                &project,
                &blocks,
                &stages(&[Stage::Detect]),
                RunPriority::Normal
            )
            .is_err()
        );
        let found = Plan::new(
            &project,
            &blocks,
            &stages(&[Stage::Recognize]),
            RunPriority::Normal,
        )?;
        assert_eq!(found.mode, Mode::Interactive);
        assert_eq!(found.pages.len(), 1);
        assert_eq!(found.pages[0].0, page);

        let proj_normal = Plan::new(
            &project,
            &Target::Project,
            &stages(&[Stage::Detect]),
            RunPriority::Normal,
        )?;
        assert_eq!(proj_normal.mode, Mode::Batch);
        let proj_rerun = Plan::new(
            &project,
            &Target::Project,
            &stages(&[Stage::Detect]),
            RunPriority::RerunNow,
        )?;
        assert_eq!(proj_rerun.mode, Mode::Interactive);

        let area = Target::Area {
            page,
            area: Area {
                rect: Rect {
                    x: 0,
                    y: 0,
                    width: 2,
                    height: 1,
                },
                pixels: vec![255, 0],
            },
            prompt: None,
            layer: None,
        };
        assert!(
            Plan::new(
                &project,
                &area,
                &stages(&[Stage::Translate]),
                RunPriority::Normal
            )
            .is_err()
        );
        assert!(
            Plan::new(
                &project,
                &area,
                &stages(&[Stage::Inpaint]),
                RunPriority::Normal
            )
            .is_ok()
        );
        Ok(())
    }

    /// The ready jobs' pages and stages, as the scheduler sees them.
    fn ready(run: &Run) -> Vec<(usize, Stage)> {
        let mut ready: Vec<(usize, Stage)> = run
            .graph
            .externals(Incoming)
            .map(|job| (run.graph[job].index, run.graph[job].stage))
            .collect();
        ready.sort();
        ready
    }

    fn finish(run: &mut Run, index: usize, stage: Stage) {
        let job = run
            .graph
            .node_indices()
            .find(|&job| run.graph[job].index == index && run.graph[job].stage == stage)
            .expect("the job");
        run.graph.remove_node(job);
    }

    #[test]
    fn a_page_waits_for_detection_and_translation_for_the_page_before() -> anyhow::Result<()> {
        let (_dir, session, ..) = project()?;
        let project = session.project();
        let stages = stages(&[Stage::Detect, Stage::Recognize, Stage::Translate]);
        let plan = Plan::new(&project, &Target::Project, &stages, RunPriority::Normal)?;
        let mut run = Run::new(
            RunId(0),
            Arc::new(Mutex::new(session)),
            project.id(),
            PipelineConfig::default(),
            plan,
            &stages,
            RunPriority::Normal,
            QueueScope::Project,
        );
        assert_eq!(ready(&run), [(0, Stage::Detect), (1, Stage::Detect)]);
        finish(&mut run, 1, Stage::Detect);
        finish(&mut run, 1, Stage::Recognize);
        assert_eq!(ready(&run), [(0, Stage::Detect)]);
        finish(&mut run, 0, Stage::Detect);
        finish(&mut run, 0, Stage::Recognize);
        assert_eq!(ready(&run), [(0, Stage::Translate)]);
        finish(&mut run, 0, Stage::Translate);
        assert_eq!(ready(&run), [(1, Stage::Translate)]);
        Ok(())
    }

    #[test]
    fn metadata_target_validates_stages_skips_empty_and_chains_after_project_translation()
    -> anyhow::Result<()> {
        let (_dir, mut session, ..) = project()?;
        assert!(
            Plan::new(
                &session.project(),
                &Target::Metadata,
                &stages(&[Stage::Detect]),
                RunPriority::Normal,
            )
            .is_err()
        );

        // Absent comic_info -> 0 steps (completes as Done without loading a model).
        let empty_plan = Plan::new(
            &session.project(),
            &Target::Metadata,
            &stages(&[Stage::Translate]),
            RunPriority::Normal,
        )?;
        assert_eq!(empty_plan.steps(1), 0);

        // Corrupt comic_info -> fails closed.
        session.apply(Edit {
            label: "corrupt".to_owned(),
            ops: vec![koharu_scene::Op::SetProjectMetadata {
                key: koharu_io::COMIC_INFO_KEY.to_owned(),
                value: Some(json!({"title": 123})),
            }],
        })?;
        assert!(
            Plan::new(
                &session.project(),
                &Target::Metadata,
                &stages(&[Stage::Translate]),
                RunPriority::Normal,
            )
            .is_err()
        );

        // Valid prose metadata -> 1 interactive step on Target::Metadata, and trailing
        // chained step on Target::Project + Stage::Translate.
        let info = koharu_io::ComicInfo {
            title: Some(koharu_io::LocalizedField::new("あつい夏")),
            ..Default::default()
        };
        session.apply(Edit {
            label: "meta".to_owned(),
            ops: vec![koharu_scene::Op::SetProjectMetadata {
                key: koharu_io::COMIC_INFO_KEY.to_owned(),
                value: Some(serde_json::to_value(info)?),
            }],
        })?;

        let meta_plan = Plan::new(
            &session.project(),
            &Target::Metadata,
            &stages(&[Stage::Translate]),
            RunPriority::Normal,
        )?;
        assert_eq!(meta_plan.steps(1), 1);

        let project = session.project();
        let stages = stages(&[Stage::Translate]);
        let batch_plan = Plan::new(&project, &Target::Project, &stages, RunPriority::Normal)?;
        assert_eq!(batch_plan.steps(1), 3);
        let mut run = Run::new(
            RunId(1),
            Arc::new(Mutex::new(session)),
            project.id(),
            PipelineConfig::default(),
            batch_plan,
            &stages,
            RunPriority::Normal,
            QueueScope::Project,
        );
        assert_eq!(ready(&run), [(0, Stage::Translate)]);
        finish(&mut run, 0, Stage::Translate);
        assert_eq!(ready(&run), [(1, Stage::Translate)]);
        finish(&mut run, 1, Stage::Translate);
        assert_eq!(ready(&run), [(2, Stage::Translate)]);
        let meta_node = run.graph.externals(Incoming).next().unwrap();
        assert_eq!(run.graph[meta_node].page, None);
        Ok(())
    }
}
