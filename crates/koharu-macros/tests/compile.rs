//! rustc-level expansion tests for `#[command]` and `routes!`.
//!
//! These compile generated items (protocol function, HTTP handler, route module,
//! and `routes!`) so namespace collisions, missing extractors, and Host typing
//! fail the build instead of only token-shape assertions.

#![allow(dead_code)]

extern crate koharu_macros_test_app as koharu_app;
extern crate koharu_macros_test_specta as specta;

use koharu_app::host::Host;
use koharu_app::{
    AgentState, CanvasChannel, CanvasState, Channel, CurrentProject, Desktop, Error, ExportBytes,
    JobChannel, JobId, LoginEvent, Pipeline, Processing, Scope, StartupState, WebviewWindow,
};
use koharu_macros::{command, routes};

#[command]
async fn process(pipeline: Pipeline, scope: Scope) -> Result<JobId, Error> {
    let _ = pipeline;
    let _ = scope;
    Ok(JobId)
}

#[command]
async fn process_high_arity(
    pipeline: Pipeline,
    scope: Scope,
    project: CurrentProject,
    processing: Processing,
    jobs: JobChannel,
    desktop: Desktop,
    canvas: CanvasChannel,
    prompt: String,
) -> Result<JobId, Error> {
    let _ = (
        pipeline, scope, project, processing, jobs, desktop, canvas, prompt,
    );
    Ok(JobId)
}

#[command]
async fn subscribe(host: Host, on_canvas: Channel<CanvasState>) -> Result<StartupState, Error> {
    let _ = (host, on_canvas);
    Ok(StartupState)
}

#[command]
async fn get_project(project: CurrentProject) -> Result<(), Error> {
    let _ = project;
    Ok(())
}

#[command]
async fn run_agent(
    state: AgentState,
    pipeline: Pipeline,
    on_event: Channel<LoginEvent>,
    prompt: String,
) -> Result<JobId, Error> {
    let _ = (state, pipeline, on_event, prompt);
    Ok(JobId)
}

#[command]
async fn import(
    source: String,
    paths: Option<Vec<String>>,
    project: CurrentProject,
) -> Result<(), Error> {
    let _ = (source, paths, project);
    Ok(())
}

#[command]
async fn export(
    format: String,
    destination: Option<String>,
    window: WebviewWindow,
) -> Result<Option<ExportBytes>, Error> {
    let _ = (format, destination, window);
    Ok(None)
}

#[command]
async fn get_font_preview(family_name: String, desktop: Desktop) -> Result<JobId, Error> {
    let _ = (family_name, desktop);
    Ok(JobId)
}

#[test]
fn generated_http_route_types_compile() {
    let _ = process;
    let _ = process_high_arity;
    let _ = subscribe;
    let _ = get_project;
    let _ = run_agent;
    let _ = import;
    let _ = export;
    let _ = get_font_preview;
    let _: axum::Router<koharu_app::host::Host> = routes!(
        process,
        process_high_arity,
        subscribe,
        get_project,
        run_agent,
        import,
        export,
        get_font_preview
    );
}

#[test]
fn payloads_accept_camel_case() {
    let preview: GetFontPreviewPayload =
        serde_json::from_str(r#"{"familyName":"Inter"}"#).expect("camelCase POST body");
    assert_eq!(preview.family_name, "Inter");

    let import: ImportPayload = serde_json::from_str(r#"{"source":"files","paths":["a.png"]}"#)
        .expect("multipart fields stay unchanged");
    assert_eq!(import.source, "files");
    assert_eq!(import.paths, Some(vec!["a.png".to_owned()]));
}
