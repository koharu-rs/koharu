//! A run: what it works on, and its graph of jobs, a page's stage each, with
//! an edge for what each needs: detection before the page's other stages,
//! recognition before translation, and page turns in reading order. Multi-page
//! translation jobs are admitted together once the run's sources are ready.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use koharu_scene::{Content, LayerId, PageId, Project, ProjectId, Session};
use parking_lot::Mutex;
use petgraph::stable_graph::{NodeIndex, StableDiGraph};
use tokio_util::sync::CancellationToken;

use super::Mode;
use crate::{
    Error, PipelineConfig, Result, RunId, Stage, Target, resume, stages::inpaint::AreaWork,
};

pub(crate) struct Run {
    pub(super) id: RunId,
    pub(super) mode: Mode,
    pub(super) session: Arc<Mutex<Session>>,
    pub(super) project: ProjectId,
    pub(super) config: Arc<PipelineConfig>,
    pub(super) area: Option<Arc<AreaWork>>,
    pub(super) graph: StableDiGraph<Job, ()>,
    pub(super) cancel: CancellationToken,
    pub(super) resume: bool,
    pub(super) glossary_pages: Arc<BTreeSet<PageId>>,
}

pub(super) struct Job {
    pub(super) page: PageId,
    /// The page's place in the run.
    pub(super) index: usize,
    pub(super) stage: Stage,
    pub(super) blocks: Option<Arc<[LayerId]>>,
    pub(super) running: bool,
}

/// What a run works on: its pages in order, each with the blocks it's
/// limited to.
pub(crate) struct Plan {
    mode: Mode,
    pages: Vec<(PageId, Option<Arc<[LayerId]>>)>,
    area: Option<Arc<AreaWork>>,
    resume: bool,
    skip: BTreeSet<(PageId, Stage)>,
    skip_glossary: bool,
}

impl Plan {
    pub(crate) fn steps(&self, stages: &BTreeSet<Stage>) -> usize {
        let glossary = usize::from(stages.contains(&Stage::Glossary));
        self.pages.len() * (stages.len() - glossary) - self.skip.len()
            + if self.skip_glossary {
                0
            } else {
                glossary.min(self.pages.len())
            }
    }

    pub(crate) fn resume(
        &mut self,
        project: &Project,
        stages: &BTreeSet<Stage>,
        config: &PipelineConfig,
    ) -> Result<()> {
        if self.area.is_some() {
            return Ok(());
        }
        self.resume = true;
        let mut source_pending = false;
        for (id, selected) in &self.pages {
            let page = project.page(*id)?;
            let detect = stages.contains(&Stage::Detect) && !resume::detection_done(page);
            let ocr = stages.contains(&Stage::Recognize)
                && (detect
                    || !resume::pending_blocks(
                        project,
                        *id,
                        Stage::Recognize,
                        config,
                        selected.as_deref(),
                    )
                    .is_empty());
            source_pending |= detect || ocr;
            for &stage in stages {
                let done = match stage {
                    Stage::Detect => !detect,
                    Stage::Recognize => !ocr,
                    Stage::Glossary => continue,
                    Stage::Translate | Stage::Inpaint => {
                        !detect
                            && !ocr
                            && resume::pending_blocks(
                                project,
                                *id,
                                stage,
                                config,
                                selected.as_deref(),
                            )
                            .is_empty()
                    }
                };
                if done {
                    self.skip.insert((*id, stage));
                }
            }
        }
        if stages.contains(&Stage::Glossary) && !source_pending && !self.pages.is_empty() {
            let pages = self.pages.iter().map(|(page, _)| *page).collect();
            let lines = resume::glossary_lines(project, &pages)?;
            self.skip_glossary = lines.is_empty()
                || project.glossary_input()
                    == Some(resume::glossary_input(&lines, &config.translator.target));
        }
        Ok(())
    }

    /// What a run of `stages` on `target` works on, or why it can't run.
    pub(crate) fn new(
        project: &Project,
        target: &Target,
        stages: &BTreeSet<Stage>,
    ) -> Result<Self> {
        if stages.is_empty() {
            return Err(Error::invalid("a run needs a stage"));
        }
        let pages = |pages: Vec<PageId>| Self {
            mode: Mode::Batch,
            pages: pages
                .into_iter()
                .filter(|&id| project.page(id).is_ok_and(|page| !page.excluded))
                .map(|page| (page, None))
                .collect(),
            area: None,
            resume: false,
            skip: BTreeSet::new(),
            skip_glossary: false,
        };
        Ok(match target {
            Target::Project => pages(project.pages().map(|page| page.id).collect()),
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
                )
            }
            Target::Blocks(blocks) => {
                if stages.contains(&Stage::Glossary) {
                    return Err(Error::invalid(
                        "terminology analysis runs on a project or pages",
                    ));
                }
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
                        .filter(|page| !page.excluded)
                        .filter_map(|page| Some((page.id, Some(held.remove(&page.id)?.into()))))
                        .collect(),
                    area: None,
                    resume: false,
                    skip: BTreeSet::new(),
                    skip_glossary: false,
                }
            }
            Target::Area { page, area, prompt } => {
                if stages.iter().ne([Stage::Inpaint].iter()) {
                    return Err(Error::invalid("an area takes inpainting only"));
                }
                if project.page(*page)?.excluded {
                    return Ok(pages(Vec::new()));
                }
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
                    resume: false,
                    skip: BTreeSet::new(),
                    skip_glossary: false,
                    area: Some(Arc::new(AreaWork {
                        area: area.clone(),
                        prompt: prompt.map(str::to_owned),
                    })),
                }
            }
        })
    }
}

impl Run {
    pub(crate) fn new(
        id: RunId,
        session: Arc<Mutex<Session>>,
        project: ProjectId,
        config: PipelineConfig,
        plan: Plan,
        stages: &BTreeSet<Stage>,
    ) -> Self {
        let mut graph = StableDiGraph::new();
        let mut translated = None;
        let glossary = plan.pages.first().and_then(|(page, _)| {
            (stages.contains(&Stage::Glossary) && !plan.skip_glossary).then(|| {
                graph.add_node(Job {
                    page: *page,
                    index: 0,
                    stage: Stage::Glossary,
                    blocks: None,
                    running: false,
                })
            })
        });
        let glossary_pages = if glossary.is_some() {
            plan.pages.iter().map(|(page, _)| *page).collect()
        } else {
            BTreeSet::new()
        };
        let keeps_originals = session
            .lock()
            .project()
            .effective_glossary(&config.glossary)
            .into_iter()
            .any(|entry| entry.kind == koharu_scene::GlossaryKind::PreserveBlock);
        for (index, (page, blocks)) in plan.pages.into_iter().enumerate() {
            let jobs: BTreeMap<Stage, NodeIndex> = stages
                .iter()
                .filter(|&&stage| stage != Stage::Glossary && !plan.skip.contains(&(page, stage)))
                .map(|&stage| {
                    let job = Job {
                        page,
                        index,
                        stage,
                        blocks: blocks.clone(),
                        running: false,
                    };
                    (stage, graph.add_node(job))
                })
                .collect();
            if let Some(glossary) = glossary {
                if let Some(&source) = jobs
                    .get(&Stage::Recognize)
                    .or_else(|| jobs.get(&Stage::Detect))
                {
                    graph.add_edge(source, glossary, ());
                }
                if let Some(&translate) = jobs.get(&Stage::Translate) {
                    graph.add_edge(glossary, translate, ());
                }
            }
            if let Some(&detect) = jobs.get(&Stage::Detect) {
                for (_, &job) in jobs.range(Stage::Recognize..) {
                    graph.add_edge(detect, job, ());
                }
            }
            if let Some(&recognize) = jobs.get(&Stage::Recognize) {
                if let Some(&translate) = jobs.get(&Stage::Translate) {
                    graph.add_edge(recognize, translate, ());
                }
                // The whole-block exclusions use the OCR text for that page.
                if let Some(&inpaint) = jobs.get(&Stage::Inpaint)
                    && keeps_originals
                {
                    graph.add_edge(recognize, inpaint, ());
                }
            }
            if !config.translator.batches_pages()
                && let Some(&translate) = jobs.get(&Stage::Translate)
            {
                if let Some(before) = translated.replace(translate) {
                    graph.add_edge(before, translate, ());
                }
            }
        }
        Self {
            id,
            mode: plan.mode,
            session,
            project,
            config: Arc::new(config),
            area: plan.area,
            graph,
            cancel: CancellationToken::new(),
            resume: plan.resume,
            glossary_pages: Arc::new(glossary_pages),
        }
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
        let mut session = Session::create(dir.path().join("test.khr"))?;
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
        assert!(Plan::new(&project, &blocks, &stages(&[Stage::Detect])).is_err());
        let found = Plan::new(&project, &blocks, &stages(&[Stage::Recognize]))?;
        assert_eq!(found.pages.len(), 1);
        assert_eq!(found.pages[0].0, page);

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
        };
        assert!(Plan::new(&project, &area, &stages(&[Stage::Translate])).is_err());
        assert!(Plan::new(&project, &area, &stages(&[Stage::Inpaint])).is_ok());
        Ok(())
    }

    #[test]
    fn resume_queues_only_pending_work_on_included_pages() -> anyhow::Result<()> {
        let (_dir, mut session, page, completed) = project()?;
        let first = session.project().pages().next().expect("first page").id;
        let ops = [
            json!({"op":"set_page_options","page":first,"excluded":true}),
            json!({"op":"set_text","layer":completed,"text":"Kept translation"}),
            json!({"op":"update_layer","layer":completed,"patch":{
                "source":{"content":[{"insert":"已有原文"}]},
                "translation":{"language":"en"}
            }}),
            json!({"op":"add_layer","id":"$pending","page":page,"index":1,"layer":{
                "kind":"block","source":{"content":[{"insert":"新原文"}]}
            }}),
        ];
        let change = session.apply(Edit {
            label: "test".into(),
            ops: ops
                .into_iter()
                .map(serde_json::from_value)
                .collect::<Result<_, _>>()?,
        })?;
        let Some(Created::Layer { id: pending, .. }) = change.created.first() else {
            anyhow::bail!("missing block");
        };
        let project = session.project();
        let config = PipelineConfig::default();
        let stages = stages(&[
            Stage::Detect,
            Stage::Recognize,
            Stage::Inpaint,
            Stage::Translate,
        ]);
        let mut plan = Plan::new(&project, &Target::Pages(vec![first, page]), &stages)?;
        plan.resume(&project, &stages, &config)?;
        assert_eq!(plan.pages.len(), 1);
        assert_eq!(plan.steps(&stages), 1);
        assert_eq!(
            resume::pending_blocks(&project, page, Stage::Translate, &config, None),
            [*pending]
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
        let plan = Plan::new(&project, &Target::Project, &stages)?;
        let mut run = Run::new(
            RunId(0),
            Arc::new(Mutex::new(session)),
            project.id(),
            PipelineConfig::default(),
            plan,
            &stages,
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
    fn glossary_runs_once_between_ocr_and_translation() -> anyhow::Result<()> {
        let (_dir, session, ..) = project()?;
        let project = session.project();
        let stages = stages(&[Stage::Recognize, Stage::Glossary, Stage::Translate]);
        let plan = Plan::new(&project, &Target::Project, &stages)?;
        assert_eq!(plan.steps(&stages), 5);
        let mut run = Run::new(
            RunId(0),
            Arc::new(Mutex::new(session)),
            project.id(),
            PipelineConfig::default(),
            plan,
            &stages,
        );
        assert_eq!(ready(&run), [(0, Stage::Recognize), (1, Stage::Recognize)]);
        finish(&mut run, 0, Stage::Recognize);
        assert_eq!(ready(&run), [(1, Stage::Recognize)]);
        finish(&mut run, 1, Stage::Recognize);
        assert_eq!(ready(&run), [(0, Stage::Glossary)]);
        finish(&mut run, 0, Stage::Glossary);
        assert_eq!(ready(&run), [(0, Stage::Translate)]);
        Ok(())
    }
}
