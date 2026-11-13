//! Upstream: src/breadcrumbs.rs.

use std::sync::Arc;

use sentry::{Breadcrumb, protocol::Map};
use serde_json::Value;
use tauri::{
    AppHandle, DragDropEvent, Manager, RunEvent, Runtime, Url, Webview, WebviewEvent, Window,
    WindowEvent,
    plugin::Builder,
    webview::{PageLoadEvent, PageLoadPayload},
};

use crate::context::TauriContext;

const DEFAULT_APP_EVENTS: &[&str] = &[
    "ready",
    "exit-requested",
    "exit",
    "resumed",
    "opened",
    "reopen",
];

const DEFAULT_WINDOW_EVENTS: &[&str] = &[
    "created",
    "focus",
    "blur",
    "close-requested",
    "destroyed",
    "theme-changed",
    "scale-change",
    "drag-drop",
    "suspended",
    "resumed",
];

const DEFAULT_WEBVIEW_EVENTS: &[&str] = &[
    "created",
    "navigation",
    "page-load-started",
    "page-load-finished",
];

/// Selects which events of a category create breadcrumbs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum EventFilter {
    /// The default events for the category. These skip events that fire
    /// at a high rate such as `resize`, `move` and `drag-over`.
    #[default]
    Default,
    /// All events of the category.
    All,
    /// No events of the category.
    None,
    /// Only the events with these names.
    Only(Vec<String>),
}

impl EventFilter {
    fn matches(&self, name: &str, defaults: &[&str]) -> bool {
        match self {
            EventFilter::Default => defaults.contains(&name),
            EventFilter::All => true,
            EventFilter::None => false,
            EventFilter::Only(names) => names.iter().any(|n| n == name),
        }
    }
}

/// Options for breadcrumbs from Tauri app, window and webview events.
///
/// Breadcrumbs have the category `tauri` and a message like `app.ready`,
/// `window.focus` or `webview.page-load-finished`.
#[derive(Debug, Clone, Default)]
pub struct BreadcrumbOptions {
    /// App events: `ready`, `exit-requested`, `exit`, `resumed`, `opened`
    /// and `reopen`.
    pub app: EventFilter,
    /// Window events: `created`, `focus`, `blur`, `close-requested`,
    /// `destroyed`, `theme-changed`, `scale-change`, `resize`, `move`,
    /// `drag-enter`, `drag-over`, `drag-drop`, `drag-leave`, `suspended`
    /// and `resumed`.
    pub window: EventFilter,
    /// Webview events: `created`, `navigation`, `page-load-started`,
    /// `page-load-finished`, `drag-enter`, `drag-over`, `drag-drop` and
    /// `drag-leave`.
    pub webview: EventFilter,
    /// Adds the window title to window and webview breadcrumbs.
    ///
    /// Default: `false`
    pub capture_window_titles: bool,
}

impl BreadcrumbOptions {
    /// No breadcrumbs for Tauri events.
    pub fn disabled() -> Self {
        Self {
            app: EventFilter::None,
            window: EventFilter::None,
            webview: EventFilter::None,
            capture_window_titles: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Category {
    App,
    Window,
    Webview,
}

impl Category {
    fn as_str(self) -> &'static str {
        match self {
            Category::App => "app",
            Category::Window => "window",
            Category::Webview => "webview",
        }
    }
}

#[derive(Debug, PartialEq)]
struct TauriEvent {
    category: Category,
    name: &'static str,
    label: Option<String>,
    data: Map<String, Value>,
}

impl TauriEvent {
    fn new(category: Category, name: &'static str, label: Option<&str>) -> Self {
        Self {
            category,
            name,
            label: label.map(str::to_owned),
            data: Map::new(),
        }
    }

    fn with(mut self, key: &str, value: impl Into<Value>) -> Self {
        self.data.insert(key.to_owned(), value.into());
        self
    }

    fn should_capture(&self, options: &BreadcrumbOptions) -> bool {
        match self.category {
            Category::App => options.app.matches(self.name, DEFAULT_APP_EVENTS),
            Category::Window => options.window.matches(self.name, DEFAULT_WINDOW_EVENTS),
            Category::Webview => options.webview.matches(self.name, DEFAULT_WEBVIEW_EVENTS),
        }
    }

    fn into_breadcrumb(self, title: Option<String>) -> Breadcrumb {
        let mut data = self.data;
        if let Some(label) = self.label {
            data.insert("label".into(), label.into());
        }
        if let Some(title) = title {
            data.insert("title".into(), title.into());
        }

        Breadcrumb {
            ty: "ui".into(),
            category: Some("tauri".into()),
            message: Some(format!("{}.{}", self.category.as_str(), self.name)),
            data,
            ..Default::default()
        }
    }
}

fn drag_drop_name(event: &DragDropEvent) -> Option<&'static str> {
    match event {
        DragDropEvent::Enter { .. } => Some("drag-enter"),
        DragDropEvent::Over { .. } => Some("drag-over"),
        DragDropEvent::Drop { .. } => Some("drag-drop"),
        DragDropEvent::Leave => Some("drag-leave"),
        _ => None,
    }
}

fn window_event(label: &str, event: &WindowEvent) -> Option<TauriEvent> {
    let window = |name| Some(TauriEvent::new(Category::Window, name, Some(label)));

    match event {
        WindowEvent::Resized(_) => window("resize"),
        WindowEvent::Moved(_) => window("move"),
        WindowEvent::CloseRequested { .. } => window("close-requested"),
        WindowEvent::Destroyed => window("destroyed"),
        WindowEvent::Focused(true) => window("focus"),
        WindowEvent::Focused(false) => window("blur"),
        WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
            window("scale-change").map(|e| e.with("scale_factor", *scale_factor))
        }
        WindowEvent::DragDrop(event) => drag_drop_name(event).and_then(window),
        WindowEvent::ThemeChanged(theme) => {
            window("theme-changed").map(|e| e.with("theme", theme.to_string()))
        }
        #[cfg(mobile)]
        WindowEvent::Suspended => window("suspended"),
        #[cfg(mobile)]
        WindowEvent::Resumed => window("resumed"),
        _ => None,
    }
}

fn run_event(event: &RunEvent) -> Option<TauriEvent> {
    let app = |name| Some(TauriEvent::new(Category::App, name, None));

    match event {
        RunEvent::Ready => app("ready"),
        RunEvent::ExitRequested { code, .. } => app("exit-requested").map(|e| match code {
            Some(code) => e.with("code", *code),
            None => e,
        }),
        RunEvent::Exit => app("exit"),
        RunEvent::Resumed => app("resumed"),
        #[cfg(any(target_os = "macos", target_os = "ios", target_os = "android"))]
        RunEvent::Opened { .. } => app("opened"),
        #[cfg(target_os = "macos")]
        RunEvent::Reopen { .. } => app("reopen"),
        RunEvent::WindowEvent { label, event, .. } => window_event(label, event),
        RunEvent::WebviewEvent {
            label,
            event: WebviewEvent::DragDrop(event),
            ..
        } => {
            drag_drop_name(event).map(|name| TauriEvent::new(Category::Webview, name, Some(label)))
        }
        _ => None,
    }
}

fn page_load_name(event: PageLoadEvent) -> &'static str {
    match event {
        PageLoadEvent::Started => "page-load-started",
        PageLoadEvent::Finished => "page-load-finished",
    }
}

fn add(
    event: Option<TauriEvent>,
    options: &BreadcrumbOptions,
    title: impl FnOnce() -> Option<String>,
) {
    let Some(event) = event.filter(|e| e.should_capture(options)) else {
        return;
    };

    let title = if options.capture_window_titles && event.category != Category::App {
        title()
    } else {
        None
    };

    sentry::add_breadcrumb(event.into_breadcrumb(title));
}

pub(crate) fn on_event<R: Runtime>(
    options: &BreadcrumbOptions,
    app: &AppHandle<R>,
    event: &RunEvent,
) {
    let event = run_event(event);
    let label = event.as_ref().and_then(|e| e.label.clone());
    add(event, options, || {
        app.get_webview_window(&label?).and_then(|w| w.title().ok())
    });
}

pub(crate) fn on_window_created<R: Runtime>(options: &BreadcrumbOptions, window: &Window<R>) {
    let event = TauriEvent::new(Category::Window, "created", Some(window.label()));
    add(Some(event), options, || window.title().ok());
}

pub(crate) fn on_webview_created<R: Runtime>(options: &BreadcrumbOptions, webview: &Webview<R>) {
    let event = TauriEvent::new(Category::Webview, "created", Some(webview.label()));
    add(Some(event), options, || webview.window().title().ok());
}

pub(crate) fn on_navigation<R: Runtime>(
    options: &BreadcrumbOptions,
    webview: &Webview<R>,
    url: &Url,
) {
    let event = TauriEvent::new(Category::Webview, "navigation", Some(webview.label()))
        .with("url", url.as_str());
    add(Some(event), options, || webview.window().title().ok());
}

pub(crate) fn on_page_load<R: Runtime>(
    options: &BreadcrumbOptions,
    webview: &Webview<R>,
    payload: &PageLoadPayload<'_>,
) {
    let event = TauriEvent::new(
        Category::Webview,
        page_load_name(payload.event()),
        Some(webview.label()),
    )
    .with("url", payload.url().as_str());
    add(Some(event), options, || webview.window().title().ok());
}

/// Divergence: the run events also update `context`, which upstream's
/// `Plugin` impl did in its own `on_event`; a `Builder` has one.
pub(crate) fn attach<R: Runtime>(
    builder: Builder<R>,
    options: Arc<BreadcrumbOptions>,
    context: Arc<TauriContext>,
) -> Builder<R> {
    let (o1, o2, o3, o4, o5) = (
        options.clone(),
        options.clone(),
        options.clone(),
        options.clone(),
        options,
    );
    builder
        .on_window_ready(move |window| on_window_created(&o1, &window))
        .on_webview_ready(move |webview| on_webview_created(&o2, &webview))
        .on_navigation(move |webview, url| {
            on_navigation(&o3, webview, url);
            true
        })
        .on_page_load(move |webview, payload| on_page_load(&o4, webview, payload))
        .on_event(move |app, event| {
            on_event(&o5, app, event);
            context.on_event(app, event);
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri::{PhysicalPosition, PhysicalSize, Theme};

    fn captured(event: Option<TauriEvent>, options: &BreadcrumbOptions) -> Option<Breadcrumb> {
        event
            .filter(|e| e.should_capture(options))
            .map(|e| e.into_breadcrumb(None))
    }

    #[test]
    fn maps_app_events() {
        let options = BreadcrumbOptions::default();

        let crumb = captured(run_event(&RunEvent::Ready), &options).expect("present");
        assert_eq!(crumb.category.as_deref(), Some("tauri"));
        assert_eq!(crumb.ty, "ui");
        assert_eq!(crumb.message.as_deref(), Some("app.ready"));
        assert!(crumb.data.is_empty());

        let crumb = captured(run_event(&RunEvent::Exit), &options).expect("present");
        assert_eq!(crumb.message.as_deref(), Some("app.exit"));

        let crumb = captured(run_event(&RunEvent::Resumed), &options).expect("present");
        assert_eq!(crumb.message.as_deref(), Some("app.resumed"));

        assert!(run_event(&RunEvent::MainEventsCleared).is_none());
    }

    #[test]
    fn maps_window_events() {
        let options = BreadcrumbOptions::default();

        let crumb =
            captured(window_event("main", &WindowEvent::Focused(true)), &options).expect("present");
        assert_eq!(crumb.message.as_deref(), Some("window.focus"));
        assert_eq!(crumb.data.get("label"), Some(&Value::from("main")));

        let crumb = captured(window_event("main", &WindowEvent::Focused(false)), &options)
            .expect("present");
        assert_eq!(crumb.message.as_deref(), Some("window.blur"));

        let crumb =
            captured(window_event("main", &WindowEvent::Destroyed), &options).expect("present");
        assert_eq!(crumb.message.as_deref(), Some("window.destroyed"));

        let crumb = captured(
            window_event("main", &WindowEvent::ThemeChanged(Theme::Dark)),
            &options,
        )
        .expect("present");
        assert_eq!(crumb.message.as_deref(), Some("window.theme-changed"));
        assert_eq!(crumb.data.get("theme"), Some(&Value::from("dark")));
    }

    #[test]
    fn skips_noisy_events_by_default() {
        let options = BreadcrumbOptions::default();

        let resized = WindowEvent::Resized(PhysicalSize::new(1, 1));
        let moved = WindowEvent::Moved(PhysicalPosition::new(1, 1));
        let drag_over = WindowEvent::DragDrop(DragDropEvent::Over {
            position: PhysicalPosition::new(1.0, 1.0),
        });

        for event in [&resized, &moved, &drag_over] {
            assert!(window_event("main", event).is_some());
            assert!(captured(window_event("main", event), &options).is_none());
        }

        let options = BreadcrumbOptions {
            window: EventFilter::All,
            ..Default::default()
        };
        let crumb = captured(window_event("main", &resized), &options).expect("present");
        assert_eq!(crumb.message.as_deref(), Some("window.resize"));
    }

    #[test]
    fn filters_by_category() {
        let options = BreadcrumbOptions {
            app: EventFilter::None,
            window: EventFilter::Only(vec!["blur".into()]),
            ..Default::default()
        };

        assert!(captured(run_event(&RunEvent::Ready), &options).is_none());
        assert!(captured(window_event("main", &WindowEvent::Focused(true)), &options).is_none());
        assert!(captured(window_event("main", &WindowEvent::Focused(false)), &options).is_some());

        let options = BreadcrumbOptions::disabled();
        let event = TauriEvent::new(Category::Webview, "created", Some("main"));
        assert!(captured(Some(event), &options).is_none());
    }

    #[test]
    fn maps_webview_events() {
        let options = BreadcrumbOptions::default();

        let event = TauriEvent::new(
            Category::Webview,
            page_load_name(PageLoadEvent::Finished),
            Some("main"),
        )
        .with("url", "tauri://localhost/");
        let crumb = captured(Some(event), &options).expect("present");
        assert_eq!(crumb.message.as_deref(), Some("webview.page-load-finished"));
        assert_eq!(crumb.data.get("label"), Some(&Value::from("main")));
        assert_eq!(
            crumb.data.get("url"),
            Some(&Value::from("tauri://localhost/"))
        );
    }

    #[test]
    fn adds_title_only_when_enabled() {
        let capture = |capture_window_titles| {
            let options = BreadcrumbOptions {
                capture_window_titles,
                ..Default::default()
            };
            let events = sentry::test::with_captured_events(|| {
                let event = TauriEvent::new(Category::Window, "focus", Some("main"));
                add(Some(event), &options, || Some("My App".into()));
                sentry::capture_message("test", sentry::Level::Info);
            });
            events[0].breadcrumbs[0].data.get("title").cloned()
        };

        assert_eq!(capture(false), None);
        assert_eq!(capture(true), Some(Value::from("My App")));
    }
}
