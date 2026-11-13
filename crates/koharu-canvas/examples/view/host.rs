//! The host's part: it renders the page's frames and applies the canvas's
//! edits, as the desktop does.

use std::{sync::Arc, time::Instant};

use anyhow::Result;
use koharu_canvas::{Brush, Canvas, Effect};
use koharu_renderer::Renderer;
use koharu_scene::{Edit, Op, PageId, Ref, Rgba, Session};
use parking_lot::Mutex;

/// What the desktop does for the canvas: renders frames and applies edits.
pub struct Host {
    pub renderer: Renderer,
    pub session: Arc<Mutex<Session>>,
    pub page: PageId,
    pub canvas: Canvas,
    brush: Brush,
    /// How long each applied edit and each `set_frame` took, in
    /// milliseconds, for the script's report.
    pub applies: Vec<f64>,
    pub set_frames: Vec<f64>,
}

impl Host {
    pub fn new(renderer: Renderer, session: Arc<Mutex<Session>>, page: PageId) -> Result<Self> {
        let gpu = renderer.gpu()?;
        let canvas = Canvas::new(gpu, Arc::clone(&session));
        let brush = Brush {
            diameter: 24.0,
            color: Rgba([0.9, 0.1, 0.2, 1.0]),
        };
        let mut host = Self {
            renderer,
            session,
            page,
            canvas,
            brush,
            applies: Vec::new(),
            set_frames: Vec::new(),
        };
        host.canvas.set_brush(brush);
        host.refresh()?;
        Ok(host)
    }

    /// The page's frame again, after a change, on the canvas.
    pub fn refresh(&mut self) -> Result<()> {
        let frame = self.renderer.render(&self.session.lock(), self.page)?;
        let start = Instant::now();
        self.canvas.set_frame(frame);
        self.set_frames.push(milliseconds(start));
        Ok(())
    }

    pub fn handle(&mut self, effects: Vec<Effect>) -> Result<()> {
        for effect in effects {
            match effect {
                Effect::Edit(edit) => {
                    let start = Instant::now();
                    self.session.lock().apply(edit)?;
                    self.applies.push(milliseconds(start));
                    self.refresh()?;
                }
                Effect::Selection(layers) => tracing::info!("selected {layers:?}"),
                Effect::Camera(_) => {}
                Effect::Color(color) => {
                    self.brush.color = color;
                    self.canvas.set_brush(self.brush);
                }
                Effect::Inpaint { rect, .. } => {
                    tracing::info!("remove asked to inpaint {rect:?}; the example runs no model");
                }
                Effect::BrushDiameter(diameter) => {
                    self.brush.diameter = diameter;
                    self.canvas.set_brush(self.brush);
                }
                Effect::Notice(notice) => tracing::warn!("{notice:?}"),
            }
        }
        Ok(())
    }

    /// Undoes or redoes a step, as the app's Ctrl+Z does.
    pub fn history(&mut self, redo: bool) -> Result<()> {
        let mut session = self.session.lock();
        let change = match redo {
            true => session.redo()?,
            false => session.undo()?,
        };
        drop(session);
        if change.is_some() {
            self.refresh()?;
        }
        Ok(())
    }

    pub fn delete(&mut self) -> Result<()> {
        let ops: Vec<Op> = self
            .canvas
            .selection()
            .iter()
            .map(|&layer| Op::RemoveLayer {
                layer: Ref::from(layer),
            })
            .collect();
        if ops.is_empty() {
            return Ok(());
        }
        self.canvas.select(Vec::new());
        self.handle(vec![Effect::Edit(Edit {
            label: "delete".to_owned(),
            ops,
        })])
    }
}

pub fn milliseconds(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}
