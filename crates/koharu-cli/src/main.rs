use std::{
    collections::{BTreeSet, HashMap, HashSet},
    ffi::OsStr,
    io::{IsTerminal as _, Write as _},
    path::{Path, PathBuf},
    process::ExitCode,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context as _, Result, bail, ensure};
use clap::{Parser, error::ErrorKind};
use futures::StreamExt as _;
use icu_locale_core::LanguageIdentifier;
use koharu_io::{ExportConfig, ExportFormat, ImageCodec, ImageFormat};
use koharu_llm::rig::completion::Reasoning;
use koharu_pipeline::{
    Detector, Event, Inpainter, Outcome, Pipeline, PipelineConfig, Recognizer, Request,
    RunPriority, Stage, Target, TranslatorConfig,
};
use koharu_psd::Labels;
use koharu_renderer::{Renderer, RendererConfig};
use koharu_runtime::{AcceleratorLock, DownloadEntry, DownloadState};
use koharu_scene::{EXTENSION as KHR_EXTENSION, Meta, PageId, Session};
use parking_lot::Mutex;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::sync::broadcast::error::RecvError;
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(
    name = "koharu-cli",
    version,
    about = "Headless batch runner for Koharu"
)]
struct Cli {
    /// Existing .khr project OR images/folders/archives/PDFs.
    #[arg(required = true, value_name = "INPUT")]
    inputs: Vec<PathBuf>,

    /// Project .khr path to open or create.
    #[arg(long, value_name = "PATH")]
    project: Option<PathBuf>,

    /// Stages to run: detect,recognize,inpaint,translate.
    #[arg(
        long,
        value_delimiter = ',',
        value_parser = parse_serde::<Stage>,
        default_value = "detect,recognize,inpaint,translate"
    )]
    stages: Vec<Stage>,

    /// Override detection model ID.
    #[arg(long, value_parser = parse_serde::<Detector>, value_name = "ID")]
    detector: Option<Detector>,

    /// Override recognition model ID.
    #[arg(long, value_parser = parse_serde::<Recognizer>, value_name = "ID")]
    recognizer: Option<Recognizer>,

    /// Override inpainting model ID.
    #[arg(long, value_parser = parse_serde::<Inpainter>, value_name = "ID")]
    inpainter: Option<Inpainter>,

    /// Override translation model reference (`vendor/model`).
    #[arg(long, value_name = "MODEL")]
    translator: Option<String>,

    /// Override target translation language (BCP-47 tag supported by CLDR).
    #[arg(long, value_parser = parse_target, value_name = "LANG")]
    target: Option<LanguageIdentifier>,

    /// Enable vision input for translation.
    #[arg(long)]
    vision: bool,

    /// Override translation reasoning level (off, minimal, low, medium, high).
    #[arg(long, value_parser = parse_reasoning, value_name = "LEVEL")]
    reasoning: Option<Reasoning>,

    /// Export format: png | jpeg | webp | jxl | psd | cbz | pdf.
    #[arg(long, value_parser = parse_serde::<CliExportFormat>, value_name = "FMT")]
    export_format: Option<CliExportFormat>,

    /// Destination file (cbz/pdf) or folder (png/jpeg/webp/jxl/psd).
    #[arg(long, value_name = "PATH")]
    export_out: Option<PathBuf>,

    /// CBZ page image codec: png | jpeg | webp | jxl.
    #[arg(long, value_parser = parse_serde::<ImageCodec>, value_name = "CODEC")]
    cbz_codec: Option<ImageCodec>,

    /// Lossy encoding quality (1-100) for JPEG, WebP, and JXL.
    #[arg(long, value_parser = clap::value_parser!(u8).range(1..=100), value_name = "1-100")]
    quality: Option<u8>,

    /// Force lossless encoding for WebP and JXL.
    #[arg(long)]
    lossless: bool,

    /// Emit JSONL progress and final summary on stdout.
    #[arg(long)]
    json: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum CliExportFormat {
    Png,
    Jpeg,
    Webp,
    Jxl,
    Psd,
    Cbz,
    Pdf,
}

fn parse_serde<T: DeserializeOwned>(raw: &str) -> Result<T, String> {
    serde_json::from_value(Value::String(raw.to_owned())).map_err(|err| err.to_string())
}

fn parse_target(raw: &str) -> Result<LanguageIdentifier, String> {
    let config: TranslatorConfig =
        serde_json::from_value(json!({ "target": raw })).map_err(|err| err.to_string())?;
    Ok(config.target)
}

fn parse_reasoning(raw: &str) -> Result<Reasoning, String> {
    #[derive(serde::Deserialize)]
    struct Wrapper(#[serde(with = "koharu_llm::reasoning")] Option<Reasoning>);

    serde_json::from_value::<Wrapper>(Value::String(raw.to_owned()))
        .map_err(|err| err.to_string())?
        .0
        .ok_or_else(|| "reasoning value cannot be null".to_owned())
}

fn is_khr(path: &Path) -> bool {
    path.extension()
        .and_then(OsStr::to_str)
        .is_some_and(|ext| ext.eq_ignore_ascii_case(KHR_EXTENSION))
}

fn export_format_name(format: CliExportFormat) -> &'static str {
    match format {
        CliExportFormat::Png => "png",
        CliExportFormat::Jpeg => "jpeg",
        CliExportFormat::Webp => "webp",
        CliExportFormat::Jxl => "jxl",
        CliExportFormat::Psd => "psd",
        CliExportFormat::Cbz => "cbz",
        CliExportFormat::Pdf => "pdf",
    }
}

fn infer_export_format(path: &Path) -> Option<CliExportFormat> {
    let ext = path.extension().and_then(OsStr::to_str)?;
    if ext.eq_ignore_ascii_case("cbz") {
        Some(CliExportFormat::Cbz)
    } else if ext.eq_ignore_ascii_case("pdf") {
        Some(CliExportFormat::Pdf)
    } else {
        None
    }
}

fn resolve_export_format(format: CliExportFormat, config: &ExportConfig) -> ExportFormat {
    match format {
        CliExportFormat::Png => ExportFormat::Images(ImageFormat::Png),
        CliExportFormat::Jpeg => ExportFormat::Images(config.image_format(ImageCodec::Jpeg)),
        CliExportFormat::Webp => ExportFormat::Images(config.image_format(ImageCodec::Webp)),
        CliExportFormat::Jxl => ExportFormat::Images(config.image_format(ImageCodec::Jxl)),
        CliExportFormat::Psd => ExportFormat::Psd(Labels {
            background: "Background".to_owned(),
            cleanup: "Cleanup".to_owned(),
            outline: "Outline".to_owned(),
        }),
        CliExportFormat::Cbz => ExportFormat::Cbz(config.image_format(config.cbz_codec)),
        CliExportFormat::Pdf => ExportFormat::Pdf,
    }
}

struct ValidatedPlan {
    stages: BTreeSet<Stage>,
    export: Option<(CliExportFormat, PathBuf)>,
    input_is_khr: bool,
}

fn validate_cli(cli: &Cli) -> Result<ValidatedPlan> {
    let stages: BTreeSet<Stage> = cli.stages.iter().copied().collect();
    ensure!(!stages.is_empty(), "at least one stage is required");

    for input in &cli.inputs {
        fs_err::metadata(input)
            .with_context(|| format!("input path is not accessible: {}", input.display()))?;
    }

    let any_khr = cli.inputs.iter().any(|path| is_khr(path));
    if any_khr {
        ensure!(
            cli.inputs.len() == 1,
            "cannot pass a .{KHR_EXTENSION} project together with other inputs"
        );
        if let Some(project) = &cli.project {
            ensure!(
                project == &cli.inputs[0],
                "cannot pass a .{KHR_EXTENSION} input together with a different --project path"
            );
        }
    }

    if let Some(project) = &cli.project {
        ensure!(
            is_khr(project),
            "--project path must have a .{KHR_EXTENSION} extension: {}",
            project.display()
        );
    }

    if cli.export_format.is_some() && cli.export_out.is_none() {
        bail!("--export-format requires --export-out");
    }

    let export = if let Some(out) = &cli.export_out {
        let format = cli
            .export_format
            .or_else(|| infer_export_format(out))
            .context("--export-format is required for this --export-out path")?;
        match format {
            CliExportFormat::Png
            | CliExportFormat::Jpeg
            | CliExportFormat::Webp
            | CliExportFormat::Jxl
            | CliExportFormat::Psd => {
                ensure!(
                    !out.exists() || out.is_dir(),
                    "export destination {} exists and is not a directory",
                    out.display()
                );
            }
            CliExportFormat::Cbz | CliExportFormat::Pdf => {
                ensure!(
                    !out.is_dir(),
                    "export destination {} must be a file, not a directory",
                    out.display()
                );
            }
        }
        Some((format, out.clone()))
    } else {
        None
    };

    Ok(ValidatedPlan {
        stages,
        export,
        input_is_khr: any_khr,
    })
}

fn preferred_meta() -> Result<Meta> {
    let config: RendererConfig = koharu_config::get()?;
    let mut meta = Meta::default();
    for (kernel, faces) in config.styles {
        meta.styles.insert(kernel, faces.into_iter().collect());
    }
    Ok(meta)
}

fn open_session(path: &Path) -> Result<Session> {
    Session::open(path, preferred_meta()?).map_err(|err| match err {
        koharu_scene::Error::ProjectLocked(locked_path) => anyhow::anyhow!(
            "Project {} is currently open in Koharu",
            locked_path.display()
        ),
        other => anyhow::Error::new(other),
    })
}

async fn open_or_create_project(
    cli: &Cli,
    input_is_khr: bool,
) -> Result<(Arc<Mutex<Session>>, Option<tempfile::TempDir>)> {
    if input_is_khr {
        let path = &cli.inputs[0];
        let session = open_session(path)?;
        return Ok((Arc::new(Mutex::new(session)), None));
    }

    let (project_path, temp_holder, created_new) = match &cli.project {
        Some(path) if path.exists() => (path.clone(), None, false),
        Some(path) => {
            if let Some(parent) = path.parent()
                && !parent.as_os_str().is_empty()
            {
                fs_err::create_dir_all(parent)?;
            }
            (path.clone(), None, true)
        }
        None => {
            let temp = tempfile::tempdir()?;
            let path = temp.path().join(format!("project.{KHR_EXTENSION}"));
            (path, Some(temp), true)
        }
    };

    let session = if created_new {
        Session::create(&project_path, preferred_meta()?)?
    } else {
        open_session(&project_path)?
    };
    let session = Arc::new(Mutex::new(session));

    let inputs = cli.inputs.clone();
    let import_session = Arc::clone(&session);
    let report = tokio::task::spawn_blocking(move || {
        koharu_io::import_paths(&import_session, &inputs, None)
    })
    .await?;

    match report {
        Ok(report) => {
            if !report.skipped_files.is_empty() {
                tracing::warn!(
                    skipped = report.skipped_files.len(),
                    files = ?report.skipped_files,
                    "skipped unsupported files during import"
                );
            }
            Ok((session, temp_holder))
        }
        Err(err) => {
            drop(session);
            if created_new && temp_holder.is_none() {
                let _ = Session::remove(&project_path);
            }
            Err(err)
        }
    }
}

fn apply_pipeline_overrides(config: &mut PipelineConfig, cli: &Cli) {
    if let Some(detector) = cli.detector {
        config.detector.model = detector;
    }
    if let Some(recognizer) = cli.recognizer {
        config.recognizer.model = recognizer;
    }
    if let Some(inpainter) = cli.inpainter {
        config.inpainter.model = inpainter;
    }
    if let Some(translator) = &cli.translator {
        config.translator.model = translator.clone();
    }
    if let Some(target) = &cli.target {
        config.translator.target = target.clone();
    }
    if cli.vision {
        config.translator.vision = true;
    }
    if let Some(reasoning) = cli.reasoning {
        config.translator.reasoning = Some(reasoning);
    }
}

fn apply_export_overrides(config: &mut ExportConfig, cli: &Cli) -> Result<()> {
    if let Some(codec) = cli.cbz_codec {
        config.cbz_codec = codec;
    }
    if let Some(quality) = cli.quality {
        config.quality = quality;
    }
    if cli.lossless {
        config.lossless = true;
    }
    ensure!(
        (1..=100).contains(&config.quality),
        "export quality {} must be between 1 and 100",
        config.quality
    );
    Ok(())
}

async fn preflight_translate(config: &PipelineConfig) -> Result<()> {
    let reference = &config.translator.model;
    let (vendor, _) = reference
        .split_once('/')
        .with_context(|| format!("`{reference}` names no model: expected `vendor/model`"))?;
    if vendor == koharu_llm::KOHARU {
        koharu_llm::local_files(reference, config.translator.vision)?;
    } else {
        let _ = koharu_llm::connect_tracked(reference, koharu_llm::Size::AGENT).await?;
    }
    Ok(())
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum OutputEvent<'a> {
    Download(&'a DownloadEntry),
    Queued {
        run: u64,
        steps: usize,
        stages: &'a BTreeSet<Stage>,
    },
    Started {
        run: u64,
        page: Option<usize>,
        page_id: Option<PageId>,
        stage: Stage,
        model: Option<&'a str>,
    },
    Finished {
        run: u64,
        page: Option<usize>,
        page_id: Option<PageId>,
        stage: Stage,
    },
    Failed {
        run: u64,
        page: Option<usize>,
        page_id: Option<PageId>,
        stage: Stage,
        reason: &'a str,
    },
    Export {
        format: &'a str,
        path: &'a Path,
        pages: usize,
    },
    Summary {
        pages_total: usize,
        pages_succeeded: usize,
        pages_failed: usize,
        exported_path: Option<&'a Path>,
        duration_ms: u64,
    },
}

struct Emitter {
    json: bool,
    last_download_tty: HashMap<u64, Instant>,
}

impl Emitter {
    fn new(json: bool) -> Self {
        Self {
            json,
            last_download_tty: HashMap::new(),
        }
    }

    fn emit(&mut self, event: OutputEvent<'_>) {
        if self.json {
            let line = serde_json::to_string(&event).expect("OutputEvent serializes");
            let mut out = std::io::stdout().lock();
            let _ = writeln!(out, "{line}");
            let _ = out.flush();
            return;
        }

        let mut out = std::io::stdout().lock();
        match event {
            OutputEvent::Download(entry) => {
                let is_terminal_state = !matches!(entry.state, DownloadState::Running);
                let now = Instant::now();
                let should_print = is_terminal_state
                    || self
                        .last_download_tty
                        .get(&entry.id)
                        .is_none_or(|prev| now.duration_since(*prev) >= Duration::from_millis(250));
                if should_print {
                    self.last_download_tty.insert(entry.id, now);
                    let pct = entry
                        .completed
                        .saturating_mul(100)
                        .checked_div(entry.total)
                        .unwrap_or(0);
                    let _ = writeln!(
                        out,
                        "download {}: {} ({pct}%, {}/{} files, {:?})",
                        entry.model_id,
                        entry.model_name,
                        entry.files_done,
                        entry.files_total,
                        entry.state
                    );
                }
            }
            OutputEvent::Queued { run, steps, .. } => {
                let _ = writeln!(out, "queued run {run} ({steps} steps)");
            }
            OutputEvent::Started {
                page, stage, model, ..
            } => {
                let target = page_label(page);
                match model {
                    Some(model) => {
                        let _ = writeln!(out, "{target} {stage} started ({model})");
                    }
                    None => {
                        let _ = writeln!(out, "{target} {stage} started");
                    }
                }
            }
            OutputEvent::Finished { page, stage, .. } => {
                let _ = writeln!(out, "{} {stage} finished", page_label(page));
            }
            OutputEvent::Failed {
                page,
                stage,
                reason,
                ..
            } => {
                let _ = writeln!(out, "{} {stage} failed: {reason}", page_label(page));
            }
            OutputEvent::Export {
                format,
                path,
                pages,
            } => {
                let _ = writeln!(
                    out,
                    "exported {pages} page(s) as {format} to {}",
                    path.display()
                );
            }
            OutputEvent::Summary {
                pages_total,
                pages_succeeded,
                pages_failed,
                exported_path,
                duration_ms,
            } => match exported_path {
                Some(path) => {
                    let _ = writeln!(
                        out,
                        "done in {duration_ms}ms: {pages_succeeded}/{pages_total} succeeded, {pages_failed} failed, exported to {}",
                        path.display()
                    );
                }
                None => {
                    let _ = writeln!(
                        out,
                        "done in {duration_ms}ms: {pages_succeeded}/{pages_total} succeeded, {pages_failed} failed"
                    );
                }
            },
        }
        let _ = out.flush();
    }
}

fn page_label(page: Option<usize>) -> String {
    match page {
        Some(num) => format!("page {num}"),
        None => "metadata".to_owned(),
    }
}

fn page_number(pages_order: &[PageId], page: Option<PageId>) -> Option<usize> {
    page.and_then(|id| pages_order.iter().position(|p| *p == id).map(|idx| idx + 1))
}

struct Signals {
    #[cfg(unix)]
    sigint: Option<tokio::signal::unix::Signal>,
    #[cfg(unix)]
    sigterm: Option<tokio::signal::unix::Signal>,
}

impl Signals {
    fn new() -> Self {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{SignalKind, signal};
            Self {
                sigint: signal(SignalKind::interrupt()).ok(),
                sigterm: signal(SignalKind::terminate()).ok(),
            }
        }
        #[cfg(not(unix))]
        {
            Self {}
        }
    }

    async fn recv(&mut self) {
        #[cfg(unix)]
        {
            tokio::select! {
                _ = async {
                    if let Some(ref mut int) = self.sigint {
                        let _ = int.recv().await;
                    } else {
                        let _ = tokio::signal::ctrl_c().await;
                    }
                } => {}
                _ = async {
                    if let Some(ref mut term) = self.sigterm {
                        let _ = term.recv().await;
                    } else {
                        std::future::pending::<()>().await;
                    }
                } => {}
            }
        }
        #[cfg(not(unix))]
        {
            let _ = tokio::signal::ctrl_c().await;
        }
    }
}

fn exit_code_for(
    interrupted: bool,
    outcome: Option<&Outcome>,
    pages_failed: usize,
    metadata_failed: bool,
    export_failed: bool,
) -> u8 {
    if interrupted || matches!(outcome, Some(Outcome::Stopped)) {
        130
    } else if export_failed || matches!(outcome, Some(Outcome::Failed { .. }) | None) {
        1
    } else if pages_failed > 0 || metadata_failed {
        2
    } else {
        0
    }
}

async fn run_cli(cli: Cli, json_mode: bool) -> Result<u8> {
    let started = Instant::now();
    let plan = validate_cli(&cli)?;

    let accel_lock = Arc::new(AcceleratorLock::acquire()?);
    let (session, _holder) = open_or_create_project(&cli, plan.input_is_khr).await?;

    let mut pipeline_config = koharu_config::get::<PipelineConfig>()?;
    apply_pipeline_overrides(&mut pipeline_config, &cli);

    let mut export_config = koharu_config::get::<ExportConfig>()?;
    apply_export_overrides(&mut export_config, &cli)?;

    if plan.stages.contains(&Stage::Translate) {
        preflight_translate(&pipeline_config).await?;
    }

    let pages_order: Vec<PageId> = session.lock().project().pages().map(|p| p.id).collect();
    let mut download_stream = koharu_runtime::downloads();
    let pipeline =
        Pipeline::with_accelerator_lock(pipeline_config.clone(), Arc::clone(&accel_lock));
    let mut pipeline_events = pipeline.events();

    let run = pipeline.run(
        Arc::clone(&session),
        Request {
            target: Target::Project,
            stages: plan.stages.clone(),
            config: None,
            priority: RunPriority::Normal,
        },
    )?;

    let mut emitter = Emitter::new(json_mode);
    let mut finished_counts: HashMap<PageId, usize> = HashMap::new();
    let mut failed_pages: HashSet<PageId> = HashSet::new();
    let mut metadata_failed = false;
    let mut outcome: Option<Outcome> = None;
    let mut interrupted = false;
    let mut signals = Signals::new();

    loop {
        tokio::select! {
            maybe_dl = download_stream.next() => {
                if let Some(entry) = maybe_dl {
                    emitter.emit(OutputEvent::Download(&entry));
                }
            }
            ev = pipeline_events.recv() => {
                match ev {
                    Ok(Event::Queued { run: r, steps, stages, .. }) if r == run => {
                        emitter.emit(OutputEvent::Queued {
                            run: r.0,
                            steps,
                            stages: &stages,
                        });
                    }
                    Ok(Event::Started { run: r, page, stage, model }) if r == run => {
                        emitter.emit(OutputEvent::Started {
                            run: r.0,
                            page: page_number(&pages_order, page),
                            page_id: page,
                            stage,
                            model: model.as_deref(),
                        });
                    }
                    Ok(Event::Finished { run: r, page, stage }) if r == run => {
                        if let Some(page_id) = page {
                            *finished_counts.entry(page_id).or_default() += 1;
                        }
                        emitter.emit(OutputEvent::Finished {
                            run: r.0,
                            page: page_number(&pages_order, page),
                            page_id: page,
                            stage,
                        });
                    }
                    Ok(Event::Failed { run: r, page, stage, reason }) if r == run => {
                        match page {
                            Some(page_id) => {
                                failed_pages.insert(page_id);
                            }
                            None => {
                                metadata_failed = true;
                            }
                        }
                        emitter.emit(OutputEvent::Failed {
                            run: r.0,
                            page: page_number(&pages_order, page),
                            page_id: page,
                            stage,
                            reason: &reason,
                        });
                    }
                    Ok(Event::Ended { run: r, outcome: ended }) if r == run => {
                        if let Outcome::Failed { page, stage, ref reason } = ended {
                            tracing::error!(?page, %stage, "{reason}");
                        }
                        outcome = Some(ended);
                        break;
                    }
                    Ok(_) => {}
                    Err(RecvError::Lagged(missed)) => {
                        tracing::warn!(missed, "pipeline event stream lagged");
                    }
                    Err(RecvError::Closed) => {
                        bail!("pipeline event stream closed unexpectedly");
                    }
                }
            }
            () = signals.recv() => {
                if !interrupted {
                    interrupted = true;
                    tracing::warn!("interrupted; cancelling active run and waiting for cleanup");
                    pipeline.cancel(None);
                } else {
                    tracing::warn!("second signal received; aborting wait");
                    break;
                }
            }
        }
    }

    drop(pipeline);

    // Ensure all scheduler / GPU tasks finish and release their `Session` clone
    // before exporting or closing the SQLite store.
    let barrier_deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    while Arc::strong_count(&session) > 1 {
        if tokio::time::Instant::now() >= barrier_deadline {
            tracing::warn!("timed out waiting for pipeline tasks to release session");
            break;
        }
        tokio::select! {
            () = tokio::time::sleep(Duration::from_millis(25)) => {}
            () = signals.recv() => {
                tracing::warn!("second signal during cleanup wait");
                break;
            }
        }
    }

    let mut exported_path: Option<PathBuf> = None;
    let mut export_failed = false;
    if !interrupted
        && matches!(outcome, Some(Outcome::Done))
        && let Some((format, dest)) = plan.export
    {
        let export_res: Result<()> = async {
            match format {
                CliExportFormat::Png
                | CliExportFormat::Jpeg
                | CliExportFormat::Webp
                | CliExportFormat::Jxl
                | CliExportFormat::Psd => {
                    fs_err::create_dir_all(&dest)?;
                }
                CliExportFormat::Cbz | CliExportFormat::Pdf => {
                    if let Some(parent) = dest.parent()
                        && !parent.as_os_str().is_empty()
                    {
                        fs_err::create_dir_all(parent)?;
                    }
                }
            }
            let export_session = Arc::clone(&session);
            let export_dest = dest.clone();
            let target_lang = pipeline_config.translator.target.clone();
            let resolved = resolve_export_format(format, &export_config);
            tokio::task::spawn_blocking(move || {
                koharu_io::export(
                    &export_session,
                    Renderer::get(),
                    None,
                    resolved,
                    Some(&target_lang),
                    &export_dest,
                )
            })
            .await??;
            Ok(())
        }
        .await;

        match export_res {
            Ok(()) => {
                emitter.emit(OutputEvent::Export {
                    format: export_format_name(format),
                    path: &dest,
                    pages: pages_order.len(),
                });
                exported_path = Some(dest);
            }
            Err(err) => {
                tracing::error!("export failed: {err:#}");
                export_failed = true;
            }
        }
    }

    let pages_total = pages_order.len();
    let pages_failed = failed_pages.len();
    let pages_succeeded = if matches!(outcome, Some(Outcome::Done)) {
        pages_total.saturating_sub(pages_failed)
    } else {
        pages_order
            .iter()
            .filter(|id| {
                !failed_pages.contains(*id)
                    && finished_counts.get(*id).copied().unwrap_or(0) == plan.stages.len()
            })
            .count()
    };

    drop(session);
    drop(accel_lock);

    let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    emitter.emit(OutputEvent::Summary {
        pages_total,
        pages_succeeded,
        pages_failed,
        exported_path: exported_path.as_deref(),
        duration_ms,
    });

    Ok(exit_code_for(
        interrupted,
        outcome.as_ref(),
        pages_failed,
        metadata_failed,
        export_failed,
    ))
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(err) => {
            let code = match err.kind() {
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => 0,
                _ => 1,
            };
            let _ = err.print();
            return ExitCode::from(code);
        }
    };

    let json_mode = cli.json || !std::io::stdout().is_terminal();
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let subscriber = tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .with_env_filter(filter)
        .finish();
    tracing::subscriber::set_global_default(subscriber)
        .expect("failed to set global tracing subscriber");

    match run_cli(cli, json_mode).await {
        Ok(code) => ExitCode::from(code),
        Err(err) => {
            let _ = writeln!(std::io::stderr(), "{err:#}");
            ExitCode::from(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("koharu-cli").chain(args.iter().copied()))
    }

    #[test]
    fn parses_all_cli_flags_and_validates_overrides() {
        let cli = parse(&[
            "--project",
            "out.khr",
            "--stages",
            "detect,inpaint",
            "--detector",
            "koharu-layout-highres",
            "--recognizer",
            "baberu-ocr",
            "--inpainter",
            "lama",
            "--translator",
            "openai/gpt-4o",
            "--target",
            "en",
            "--vision",
            "--reasoning",
            "low",
            "--export-format",
            "cbz",
            "--export-out",
            "out.cbz",
            "--cbz-codec",
            "webp",
            "--quality",
            "85",
            "--lossless",
            "--json",
            "input.png",
        ])
        .unwrap();

        assert_eq!(cli.project.as_deref(), Some(Path::new("out.khr")));
        assert_eq!(cli.stages, vec![Stage::Detect, Stage::Inpaint]);
        assert_eq!(cli.detector, Some(Detector::KoharuLayoutHighres));
        assert_eq!(cli.recognizer, Some(Recognizer::BaberuOcr));
        assert_eq!(cli.inpainter, Some(Inpainter::Lama));
        assert_eq!(cli.translator.as_deref(), Some("openai/gpt-4o"));
        assert_eq!(
            cli.target.as_ref().map(ToString::to_string).as_deref(),
            Some("en")
        );
        assert!(cli.vision);
        assert_eq!(
            cli.reasoning,
            Some(Reasoning::Effort(koharu_llm::rig::completion::Effort::Low))
        );
        assert_eq!(cli.export_format, Some(CliExportFormat::Cbz));
        assert_eq!(cli.cbz_codec, Some(ImageCodec::Webp));
        assert_eq!(cli.quality, Some(85));
        assert!(cli.lossless);
        assert!(cli.json);
    }

    #[test]
    fn rejects_invalid_quality_stage_format_and_target() {
        assert!(parse(&["--quality", "0", "a.png"]).is_err());
        assert!(parse(&["--quality", "101", "a.png"]).is_err());
        assert!(parse(&["--stages", "", "a.png"]).is_err());
        assert!(parse(&["--stages", "nope", "a.png"]).is_err());
        assert!(parse(&["--export-format", "bmp", "--export-out", "a.bmp", "a.png"]).is_err());
        assert!(parse(&["--cbz-codec", "avif", "a.png"]).is_err());
        assert!(parse(&["--target", "zz", "a.png"]).is_err());
    }

    #[test]
    fn overrides_preserve_existing_true_flags_when_omitted() {
        let cli = parse(&["a.png"]).unwrap();
        let mut pipeline_cfg = PipelineConfig::default();
        pipeline_cfg.translator.vision = true;
        apply_pipeline_overrides(&mut pipeline_cfg, &cli);
        assert!(pipeline_cfg.translator.vision);

        let mut export_cfg = ExportConfig {
            lossless: true,
            ..Default::default()
        };
        apply_export_overrides(&mut export_cfg, &cli).unwrap();
        assert!(export_cfg.lossless);
    }

    #[test]
    fn validate_cli_rejects_inconsistent_khr_and_export_flags() {
        let dir = tempfile::tempdir().unwrap();
        let khr = dir.path().join("book.khr");
        let img = dir.path().join("page.png");
        fs_err::write(&khr, b"").unwrap();
        fs_err::write(&img, b"").unwrap();

        let mix = parse(&[khr.to_str().unwrap(), img.to_str().unwrap()]).unwrap();
        assert!(validate_cli(&mix).is_err());

        let diff_proj = parse(&[
            "--project",
            dir.path().join("other.khr").to_str().unwrap(),
            khr.to_str().unwrap(),
        ])
        .unwrap();
        assert!(validate_cli(&diff_proj).is_err());

        let no_out = parse(&["--export-format", "cbz", img.to_str().unwrap()]).unwrap();
        assert!(validate_cli(&no_out).is_err());

        let dir_cbz = parse(&[
            "--export-format",
            "cbz",
            "--export-out",
            dir.path().to_str().unwrap(),
            img.to_str().unwrap(),
        ])
        .unwrap();
        assert!(validate_cli(&dir_cbz).is_err());
    }

    #[test]
    fn exit_code_mapping_covers_all_outcomes() {
        assert_eq!(
            exit_code_for(false, Some(&Outcome::Done), 0, false, false),
            0
        );
        assert_eq!(
            exit_code_for(false, Some(&Outcome::Done), 1, false, false),
            2
        );
        assert_eq!(
            exit_code_for(false, Some(&Outcome::Done), 0, true, false),
            2
        );
        assert_eq!(
            exit_code_for(false, Some(&Outcome::Done), 0, false, true),
            1
        );
        assert_eq!(
            exit_code_for(
                false,
                Some(&Outcome::Failed {
                    page: None,
                    stage: Stage::Detect,
                    reason: "boom".into()
                }),
                0,
                false,
                false
            ),
            1
        );
        assert_eq!(
            exit_code_for(false, Some(&Outcome::Stopped), 0, false, false),
            130
        );
        assert_eq!(
            exit_code_for(true, Some(&Outcome::Done), 0, false, false),
            130
        );
        assert_eq!(exit_code_for(false, None, 0, false, false), 1);
    }
}
