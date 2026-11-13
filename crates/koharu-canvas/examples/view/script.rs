//! The done check: a fixed set of gestures, offscreen, each phase timed:
//! the canvas's and the host's handling of an event, and the draw after it,
//! the GPU's work included.

use std::{path::Path, time::Instant};

use anyhow::{Context as _, Result};
use koharu_canvas::{Point, Rect, Tool, Vec2};
use koharu_renderer::Gpu;
use koharu_scene::{Edit, Op};
use serde_json::json;
use ui_events::{
    ScrollDelta,
    keyboard::Modifiers,
    pointer::{
        PointerButton, PointerButtonEvent, PointerEvent, PointerId, PointerInfo,
        PointerScrollEvent, PointerState, PointerType, PointerUpdate,
    },
};
use vello::wgpu;

use crate::host::{Host, milliseconds};

const WIDTH: f64 = 1280.0;
const HEIGHT: f64 = 800.0;
const SCALE: f64 = 1.5;

/// Runs the gestures and writes the canvas to `snapshot`; `dense` first
/// adds 150 text layers.
pub fn run(mut host: Host, snapshot: &Path, dense: bool) -> Result<()> {
    let gpu = host.renderer.gpu()?;
    if dense {
        add_dense(&mut host)?;
    }
    host.canvas
        .set_viewport(Rect::new(0.0, 0.0, WIDTH, HEIGHT), SCALE);
    host.canvas.fit();
    let center = Point::new(WIDTH * 0.5, HEIGHT * 0.5);
    let diagonal = |count: usize| -> Vec<Point> {
        (0..count)
            .map(|i| {
                Point::new(100.0, 100.0).lerp(Point::new(1180.0, 700.0), i as f64 / count as f64)
            })
            .collect()
    };
    let mut timer = Timer {
        gpu: &gpu,
        phases: Vec::new(),
    };

    // Warm-up: the first draw and some hover, unrecorded.
    host.canvas.draw()?;
    host.canvas.set_tool(Tool::Select);
    for at in diagonal(20) {
        timer.unrecorded(&mut host, through(&[at]))?;
    }

    timer.phase("pan and zoom");
    for _ in 0..3 {
        let zoom = wheel(
            center,
            ScrollDelta::LineDelta(0.0, -1.0),
            Modifiers::CONTROL,
        );
        timer.step(&mut host, zoom)?;
    }
    for step in 0..40 {
        let direction = if (step / 10) % 2 == 0 { 1.0 } else { -1.0 };
        let delta = ScrollDelta::PixelDelta((45.0 * direction, 60.0 * direction).into());
        timer.step(&mut host, wheel(center, delta, Modifiers::default()))?;
    }

    timer.phase("hover");
    for at in diagonal(60) {
        timer.step(&mut host, through(&[at]))?;
    }

    host.canvas.set_tool(Tool::Draw);
    timer.phase("brush ring");
    for at in diagonal(60) {
        timer.step(&mut host, through(&[at]))?;
    }

    host.canvas.set_tool(Tool::Select);
    timer.phase("marquee");
    let corner = Point::new(20.0, 20.0);
    timer.step(&mut host, press(corner))?;
    for step in 1..=30 {
        timer.step(
            &mut host,
            through(&[corner.lerp(center, f64::from(step) / 30.0)]),
        )?;
    }
    timer.step(&mut host, release(center))?;

    // A long stroke from near the top left to near the bottom right.
    host.canvas.set_tool(Tool::Draw);
    timer.phase("stroke");
    let (from, to) = (Point::new(240.0, 120.0), Point::new(1040.0, 680.0));
    let stroke: Vec<Point> = (0..=2000)
        .map(|i| {
            let along = from.lerp(to, f64::from(i) / 2000.0);
            Point::new(along.x, along.y + 40.0 * (f64::from(i) * 0.01).sin())
        })
        .collect();
    timer.step(&mut host, press(stroke[0]))?;
    for &at in &stroke[1..] {
        timer.step(&mut host, through(&[at]))?;
    }
    let camera = host.canvas.camera();
    let bounds = stroke
        .iter()
        .fold(Rect::from_points(stroke[0], stroke[0]), |bounds, &at| {
            bounds.union_pt(at)
        });
    let boxed = camera.inverse() * bounds;
    let applied = host.applies.len();
    let start = Instant::now();
    let effects = host.canvas.input(release(stroke[2000]))?;
    let canvas = milliseconds(start);
    host.handle(effects)?;
    let apply = host
        .applies
        .get(applied)
        .context("the stroke applied an edit")?;
    let finish = canvas + apply;
    let draw = timer.timed_draw(&mut host)?;
    timer.record(None, draw)?;

    // Drag the painted raster by (120, 60) page pixels.
    host.canvas.set_tool(Tool::Select);
    timer.phase("move drag");
    let grab = stroke[1000];
    let target = camera * (camera.inverse() * grab + Vec2::new(120.0, 60.0));
    timer.step(&mut host, press(grab))?;
    for step in 1..=30 {
        timer.step(
            &mut host,
            through(&[grab.lerp(target, f64::from(step) / 30.0)]),
        )?;
    }
    timer.step(&mut host, release(target))?;

    // Drag out a text box near the top.
    host.canvas.set_tool(Tool::Text);
    timer.phase("text");
    let corner = Point::new(center.x - 150.0, center.y - 250.0);
    let across = corner + Vec2::new(300.0, 120.0);
    timer.step(&mut host, press(corner))?;
    timer.step(&mut host, through(&[across]))?;
    timer.step(&mut host, release(across))?;
    host.canvas.draw()?;
    host.canvas.snapshot()?.save(snapshot)?;

    let info = gpu.adapter.get_info();
    let project = host.session.lock().project();
    let page = project.page(host.page)?;
    println!("device: {} ({:?})", info.name, info.backend);
    println!("viewport: {WIDTH}×{HEIGHT} at {SCALE}×");
    println!(
        "page: {}×{}, {} layers; stroke box {:.0}×{:.0} page pixels",
        page.size.width,
        page.size.height,
        page.layers.len(),
        boxed.width(),
        boxed.height()
    );
    for phase in &timer.phases {
        let (input, draw) = (spread(&phase.input), spread(&phase.draw));
        println!(
            "{}: {} events, input p50 {:.3} p95 {:.3} ms, draw p50 {:.2} p95 {:.2} ms",
            phase.name,
            phase.draw.len(),
            input.0,
            input.1,
            draw.0,
            draw.1
        );
    }
    println!("stroke finish: {finish:.2} ms");
    let set_frame = spread(&host.set_frames);
    println!(
        "set_frame: p50 {:.3} p95 {:.3} ms",
        set_frame.0, set_frame.1
    );
    for layer in &page.layers {
        let value = serde_json::to_value(layer)?;
        if value["kind"] != "text" || !dense {
            println!(
                "layer {}: offset {} transform {}",
                value["kind"], value["offset"], value["transform"]
            );
        }
    }
    let history = host.session.lock().history();
    println!("history: {:?}", history.steps);
    println!("snapshot: {}", snapshot.display());
    Ok(())
}

/// 150 box texts in a 10 × 15 grid over the page, each its cell's width
/// and a fifth of its height, in one edit.
fn add_dense(host: &mut Host) -> Result<()> {
    let size = host.session.lock().project().page(host.page)?.size;
    let (cell_width, cell_height) = (f64::from(size.width) / 10.0, f64::from(size.height) / 15.0);
    let ops = (0..150)
        .map(|index| {
            let (column, row) = (f64::from(index % 10), f64::from(index / 10));
            serde_json::from_value::<Op>(json!({
                "op": "add_layer", "page": host.page, "index": index,
                "layer": {
                    "kind": "text", "content": "Hello",
                    "frame": {"box": {"bounds": {
                        "x": 0.0, "y": 0.0, "width": cell_width, "height": cell_height / 5.0,
                    }}},
                    "transform": [[1.0, 0.0, column * cell_width], [0.0, 1.0, row * cell_height]],
                },
            }))
        })
        .collect::<Result<Vec<_>, _>>()?;
    host.handle(vec![koharu_canvas::Effect::Edit(Edit {
        label: "dense".to_owned(),
        ops,
    })])
}

struct Phase {
    name: &'static str,
    /// The events seen, the first among them unrecorded.
    seen: usize,
    input: Vec<f64>,
    draw: Vec<f64>,
}

/// Each phase's times, in milliseconds.
struct Timer<'a> {
    gpu: &'a Gpu,
    phases: Vec<Phase>,
}

impl Timer<'_> {
    fn phase(&mut self, name: &'static str) {
        self.phases.push(Phase {
            name,
            seen: 0,
            input: Vec::new(),
            draw: Vec::new(),
        });
    }

    /// Gives the canvas `event`, applies what it answers, and draws.
    fn step(&mut self, host: &mut Host, event: PointerEvent) -> Result<()> {
        let start = Instant::now();
        let effects = host.canvas.input(event)?;
        host.handle(effects)?;
        let input = milliseconds(start);
        let draw = self.timed_draw(host)?;
        self.record(Some(input), draw)
    }

    fn unrecorded(&mut self, host: &mut Host, event: PointerEvent) -> Result<()> {
        let effects = host.canvas.input(event)?;
        host.handle(effects)?;
        host.canvas.draw()?;
        self.wait()
    }

    /// An event's times, past its phase's first.
    fn record(&mut self, input: Option<f64>, draw: f64) -> Result<()> {
        let phase = self.phases.last_mut().context("a phase started")?;
        phase.seen += 1;
        if phase.seen > 1 {
            phase.input.extend(input);
            phase.draw.push(draw);
        }
        Ok(())
    }

    /// Draws, waiting for the GPU.
    fn timed_draw(&self, host: &mut Host) -> Result<f64> {
        let start = Instant::now();
        host.canvas.draw()?;
        self.wait()?;
        Ok(milliseconds(start))
    }

    fn wait(&self) -> Result<()> {
        self.gpu
            .device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            })
            .map_err(|error| anyhow::anyhow!("waiting for the GPU failed: {error:?}"))?;
        Ok(())
    }
}

/// The 50th and 95th percentiles of `times`.
fn spread(times: &[f64]) -> (f64, f64) {
    if times.is_empty() {
        return (0.0, 0.0);
    }
    let mut sorted = times.to_vec();
    sorted.sort_by(f64::total_cmp);
    let at = |share: f64| sorted[((sorted.len() - 1) as f64 * share).round() as usize];
    (at(0.5), at(0.95))
}

/// The script's mouse, the primary pointer.
fn mouse() -> PointerInfo {
    PointerInfo {
        pointer_id: Some(PointerId::PRIMARY),
        persistent_device_id: None,
        pointer_type: PointerType::Mouse,
    }
}

fn state(at: Point, modifiers: Modifiers) -> PointerState {
    PointerState {
        position: (at.x, at.y).into(),
        modifiers,
        ..PointerState::default()
    }
}

fn wheel(at: Point, delta: ScrollDelta, modifiers: Modifiers) -> PointerEvent {
    PointerEvent::Scroll(PointerScrollEvent {
        pointer: mouse(),
        delta,
        state: state(at, modifiers),
    })
}

fn press(at: Point) -> PointerEvent {
    PointerEvent::Down(PointerButtonEvent {
        button: Some(PointerButton::Primary),
        pointer: mouse(),
        state: state(at, Modifiers::default()),
    })
}

/// One move through `points`, the last its current place and the rest
/// coalesced before it, as a browser batches them.
fn through(points: &[Point]) -> PointerEvent {
    let mut states: Vec<PointerState> = points
        .iter()
        .map(|&at| state(at, Modifiers::default()))
        .collect();
    let current = states.pop().unwrap_or_default();
    PointerEvent::Move(PointerUpdate {
        pointer: mouse(),
        current,
        coalesced: states,
        predicted: Vec::new(),
    })
}

fn release(at: Point) -> PointerEvent {
    PointerEvent::Up(PointerButtonEvent {
        button: Some(PointerButton::Primary),
        pointer: mouse(),
        state: state(at, Modifiers::default()),
    })
}
