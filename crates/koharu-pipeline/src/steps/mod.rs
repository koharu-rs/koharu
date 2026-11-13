//! The steps, each in four stages, the way koharu-renderer's text goes:
//! `read` turns a page into the model's input on a blocking thread, `infer`
//! runs the model on the GPU thread, `finish` does the CPU work on its output
//! on a blocking thread, so the GPU thread can go on, and `write` turns that
//! and the project as it is then into ops, applied as one undo step per page.
//! The runner here owns the locking, Stop, the events, and the edit.

pub(crate) mod detect;
pub(crate) mod inpaint;
pub(crate) mod recognize;
pub(crate) mod translate;

use std::{sync::Arc, time::Instant};

use image::{DynamicImage, GrayImage, Luma};
use koharu_scene::{
    Block, Content, Edit, Frame, LayerId, Op, PageId, Project, Rect, Session, Text, Tile, Tiles,
};
use kurbo::{Affine, BezPath, Shape as _};
use parking_lot::Mutex;
use petgraph::stable_graph::NodeIndex;
use serde_json::{Value, json};
use tokio::{runtime::Handle, sync::oneshot};
use tokio_util::sync::CancellationToken;
use tracing::info_span;

use crate::{
    Error, Event, Result, RunId, Step,
    models::Models,
    scheduler::{Command, Done, Link, Priority},
};

/// A page of a run that a task works on.
pub(crate) struct Member {
    pub(crate) run: RunId,
    pub(crate) job: NodeIndex,
    pub(crate) page: PageId,
    /// The page's place in its run, from 0, which traces name it by.
    pub(crate) index: usize,
    /// The blocks a run on blocks works on; every block of the page when
    /// `None`.
    pub(crate) blocks: Option<Arc<[LayerId]>>,
    pub(crate) session: Arc<Mutex<Session>>,
    pub(crate) cancel: CancellationToken,
}

/// One or more pages a step runs on at once, recognition's on several.
pub(crate) struct Task {
    pub(crate) members: Vec<Member>,
    pub(crate) priority: Priority,
    /// Whether it counts against the pages in flight.
    pub(crate) counted: bool,
}

pub(crate) trait Stages: Send + 'static {
    type Input: Send + 'static;
    type Output: Send + 'static;
    type Ready: Send + 'static;

    const STEP: Step;

    /// Whether `infer` needs the GPU thread: a hosted translation doesn't.
    fn on_gpu(&self) -> bool {
        true
    }

    /// The model's input from a page, or `None` when the page has nothing
    /// for it, which isn't a failure.
    fn read(&self, member: &Member) -> anyhow::Result<Option<Self::Input>>;

    /// Every page's output, in the inputs' order.
    fn infer(
        &mut self,
        models: &mut Models,
        inputs: Vec<Self::Input>,
    ) -> anyhow::Result<Vec<Self::Output>>;

    /// One page's output made ready to write, off the GPU thread and without
    /// the project's lock.
    fn finish(&mut self, output: Self::Output) -> anyhow::Result<Self::Ready>;

    /// The ops for one page's ready output, on the project as it is now.
    /// Reading it then, a step writes nothing to a layer removed meanwhile.
    fn write(&self, project: &Project, ready: Self::Ready) -> anyhow::Result<Vec<Op>>;
}

/// Runs `task` through `stages` and tells the scheduler how it went.
pub(crate) async fn perform<S: Stages>(stages: S, task: Task, link: Link) {
    let members = task
        .members
        .iter()
        .map(|member| (member.run, member.job))
        .collect();
    let counted = task.counted;
    let page = task.members[0].page;
    let result = run(stages, task, &link).await.map_err(|error| {
        let error = anyhow::Error::from(error);
        (page, format!("{error:#}"))
    });
    link.send(Command::Done(Done {
        step: S::STEP,
        members,
        counted,
        result,
    }));
}

async fn run<S: Stages>(stages: S, task: Task, link: &Link) -> Result<()> {
    let failed = |source| Error::Step {
        step: S::STEP,
        source,
    };
    let Task {
        members, priority, ..
    } = task;
    let (stages, members, inputs) = blocking(move || {
        let inputs: anyhow::Result<Vec<_>> = members
            .iter()
            .map(|member| {
                let _span = info_span!("read", step = %S::STEP, page = member.index + 1).entered();
                stages.read(member)
            })
            .collect();
        (stages, members, inputs)
    })
    .await;
    let (present, inputs): (Vec<usize>, Vec<S::Input>) = inputs
        .map_err(failed)?
        .into_iter()
        .enumerate()
        .filter_map(|(at, input)| Some((at, input?)))
        .unzip();
    if inputs.is_empty() {
        return Ok(());
    }

    let started: Vec<Event> = present
        .iter()
        .map(|&at| Event::Started {
            run: members[at].run,
            page: members[at].page,
            step: S::STEP,
        })
        .collect();
    let cancels: Vec<CancellationToken> = present
        .iter()
        .map(|&at| members[at].cancel.clone())
        .collect();
    let on_gpu = stages.on_gpu();
    let events = link.events();
    let pages: Vec<usize> = present.iter().map(|&at| members[at].index + 1).collect();
    let queued = Instant::now();
    let (sender, receiver) = oneshot::channel();
    let work = move |models: &mut Models| {
        // A stopped run's work is dropped unrun.
        if cancels.iter().all(CancellationToken::is_cancelled) {
            return;
        }
        let waited = queued.elapsed();
        let _span = info_span!("infer", step = %S::STEP, ?pages, ?waited).entered();
        for event in started {
            // No one listening isn't a failure.
            let _ = events.send(event);
        }
        let mut stages = stages;
        // A panicking model fails its run rather than the thread that owns
        // every model.
        let outputs = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            stages.infer(models, inputs)
        }))
        .unwrap_or_else(|panic| {
            let message = panic
                .downcast_ref::<&str>()
                .map(|message| (*message).to_owned())
                .or_else(|| panic.downcast_ref::<String>().cloned())
                .unwrap_or_default();
            Err(anyhow::anyhow!("the model panicked: {message}"))
        });
        let _ = sender.send((stages, outputs));
    };
    if on_gpu {
        link.send(Command::Gpu {
            priority,
            work: Box::new(work),
        });
    } else {
        let runtime = Handle::current();
        tokio::task::spawn_blocking(move || work(&mut Models::new(runtime)));
    }
    let Ok((stages, outputs)) = receiver.await else {
        return Ok(());
    };
    let outputs = outputs.map_err(failed)?;
    if outputs.len() != present.len() {
        return Err(failed(anyhow::anyhow!(
            "{} results for {} pages",
            outputs.len(),
            present.len()
        )));
    }

    blocking(move || {
        let mut stages = stages;
        for (at, output) in present.into_iter().zip(outputs) {
            let member = &members[at];
            if member.cancel.is_cancelled() {
                continue;
            }
            let ready = {
                let _span =
                    info_span!("finish", step = %S::STEP, page = member.index + 1).entered();
                stages.finish(output).map_err(failed)?
            };
            let _span = info_span!("write", step = %S::STEP, page = member.index + 1).entered();
            let mut session = member.session.lock();
            if member.cancel.is_cancelled() {
                continue;
            }
            let ops = stages.write(&session.project(), ready).map_err(failed)?;
            if !ops.is_empty() {
                session.apply(Edit {
                    label: S::STEP.to_string(),
                    ops,
                })?;
            }
        }
        Ok(())
    })
    .await
}

/// Runs CPU work off the async threads, passing on a panic.
async fn blocking<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> T {
    match tokio::task::spawn_blocking(work).await {
        Ok(value) => value,
        Err(error) => std::panic::resume_unwind(error.into_panic()),
    }
}

/// The page's scan, decoded, and the project it was read from; `None` when
/// the page is gone or has no scan.
pub(crate) fn scan(member: &Member) -> anyhow::Result<Option<(Project, DynamicImage)>> {
    let (project, bytes) = {
        let session = member.session.lock();
        let project = session.project();
        let Some(background) = project
            .page(member.page)
            .ok()
            .and_then(|page| page.background)
        else {
            return Ok(None);
        };
        let bytes = session.file(&background.file)?;
        (project, bytes)
    };
    let _span = info_span!("decode").entered();
    Ok(Some((project, image::load_from_memory(&bytes)?)))
}

/// The page's blocks in reading order, or those of them in `only`.
pub(crate) fn blocks<'p>(
    project: &'p Project,
    page: PageId,
    only: Option<&[LayerId]>,
) -> Vec<(LayerId, &'p Block)> {
    let Ok(layers) = project.layers_of(page) else {
        return Vec::new();
    };
    layers
        .into_iter()
        .filter(|layer| only.is_none_or(|only| only.contains(&layer.id)))
        .filter_map(|layer| match &layer.content {
            Content::Block(block) => Some((layer.id, &**block)),
            _ => None,
        })
        .collect()
}

/// Whether `layer` is still a block of the project.
pub(crate) fn is_block(project: &Project, layer: LayerId) -> bool {
    project
        .layer(layer)
        .is_ok_and(|layer| matches!(layer.content, Content::Block(_)))
}

/// Where a text's frame lies on the page, its transform applied; `None` for
/// point text.
pub(crate) fn bounds(text: &Text) -> Option<kurbo::Rect> {
    let rect = match &text.frame {
        Frame::Point => return None,
        Frame::Box { bounds, .. } => kurbo::Rect::new(
            bounds.x,
            bounds.y,
            bounds.x + bounds.width,
            bounds.y + bounds.height,
        ),
        Frame::Path { path, .. } => BezPath::from_svg(path.as_str()).ok()?.bounding_box(),
    };
    Some(Affine::from(text.transform).transform_rect_bbox(rect))
}

/// `rect` cut to a `width` by `height` page, or `None` when nothing is left.
pub(crate) fn clamp(rect: kurbo::Rect, width: u32, height: u32) -> Option<Rect> {
    let x0 = rect.x0.floor().max(0.0) as u32;
    let y0 = rect.y0.floor().max(0.0) as u32;
    let x1 = (rect.x1.ceil().max(0.0) as u32).min(width);
    let y1 = (rect.y1.ceil().max(0.0) as u32).min(height);
    (x1 > x0 && y1 > y0).then(|| Rect {
        x: x0 as i32,
        y: y0 as i32,
        width: x1 - x0,
        height: y1 - y0,
    })
}

/// A model's record of what it did, in C2PA's vocabulary.
pub(crate) fn action(action: &str, model: &str, parameters: Option<Value>) -> Value {
    let mut value = json!({
        "action": action,
        "softwareAgent": {"name": model, "version": env!("CARGO_PKG_VERSION")},
        "digitalSourceType": "http://cv.iptc.org/newscodes/digitalsourcetype/trainedAlgorithmicMedia",
    });
    if let Some(parameters) = parameters {
        value["parameters"] = parameters;
    }
    value
}

/// An op from its JSON form, the form the scene documents.
pub(crate) fn op(value: Value) -> anyhow::Result<Op> {
    Ok(serde_json::from_value(value)?)
}

/// A mask layer's tiles on a `width` by `height` page, and the rectangle it
/// covers there; `None` when it's empty.
pub(crate) fn on_page(tiles: &Tiles, width: u32, height: u32) -> Option<(Rect, GrayImage)> {
    let size = i64::from(Tile::SIZE);
    let (width, height) = (i64::from(width), i64::from(height));
    let mut on = Vec::new();
    for tile in &tiles.tiles {
        let left = i64::from(tile.x) * size + i64::from(tiles.offset.x);
        let top = i64::from(tile.y) * size + i64::from(tiles.offset.y);
        for (at, &value) in tile.pixels.iter().enumerate() {
            let (x, y) = (left + at as i64 % size, top + at as i64 / size);
            if value != 0 && (0..width).contains(&x) && (0..height).contains(&y) {
                on.push((x, y));
            }
        }
    }
    let x = on.iter().map(|&(x, _)| x).min()?;
    let y = on.iter().map(|&(_, y)| y).min()?;
    let right = on.iter().map(|&(x, _)| x).max()? + 1;
    let bottom = on.iter().map(|&(_, y)| y).max()? + 1;
    let rect = Rect {
        x: x as i32,
        y: y as i32,
        width: (right.min(width) - x) as u32,
        height: (bottom.min(height) - y) as u32,
    };
    let mut mask = GrayImage::new(rect.width, rect.height);
    for (px, py) in on {
        mask.put_pixel((px - x) as u32, (py - y) as u32, Luma([255]));
    }
    Some((rect, mask))
}

/// `mask` grown by `by` pixels every way, within its own size.
pub(crate) fn dilate(mask: &GrayImage, by: u32) -> GrayImage {
    let (width, height) = mask.dimensions();
    let spread = |length: u32, at: u32| at.saturating_sub(by)..=(at + by).min(length - 1);
    let rows = GrayImage::from_fn(width, height, |x, y| {
        Luma([spread(width, x)
            .map(|nx| mask.get_pixel(nx, y).0[0])
            .max()
            .unwrap_or(0)])
    });
    GrayImage::from_fn(width, height, |x, y| {
        Luma([spread(height, y)
            .map(|ny| rows.get_pixel(x, ny).0[0])
            .max()
            .unwrap_or(0)])
    })
}
