use std::{
    sync::{Arc, LazyLock, mpsc as std_mpsc},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use parking_lot::Mutex;
use serde_json::Value;
use tracing::{Event, Subscriber, field::Visit, span};
use tracing_subscriber::{Layer, layer::Context as LayerContext, registry::LookupSpan};

mod fields;
mod machine_id;

pub use fields::{BuiltInField, BuiltInFields, EventField, EventFields, UserField, UserFields};

const TARGET: &str = "koharu_metrics";
const MEASUREMENT_ID: &str = "G-GFBM26LQGE";
const ENDPOINT: &str = "https://www.google.com/g/collect";

static METRICS: LazyLock<Mutex<Metrics>> = LazyLock::new(|| Mutex::new(Metrics::new()));

struct Metrics {
    session_id: u64,
    client_id: String,
    sequence: u64,
    request_context: BuiltInFields,
    user_properties: UserFields,
    engagement: Engagement,
    sender: tokio::sync::mpsc::UnboundedSender<Message>,
}

impl Metrics {
    fn new() -> Self {
        let session_id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_secs());
        let language = sys_locale::get_locale()
            .map(|value| {
                value
                    .split(['.', '@'])
                    .next()
                    .unwrap_or_default()
                    .replace('_', "-")
                    .to_ascii_lowercase()
            })
            .unwrap_or_else(|| "en-us".to_owned());
        let request_context = [
            (BuiltInField::Language, Value::String(language)),
            (
                BuiltInField::Architecture,
                Value::String(std::env::consts::ARCH.to_owned()),
            ),
            (
                BuiltInField::Bitness,
                Value::from(std::mem::size_of::<usize>() * 8),
            ),
            (BuiltInField::Mobile, Value::from(false)),
            (
                BuiltInField::Platform,
                Value::String(std::env::consts::OS.to_owned()),
            ),
            (
                BuiltInField::BrowserVersion,
                Value::String(format!("Koharu;{}", env!("CARGO_PKG_VERSION"))),
            ),
        ]
        .into_iter()
        .chain(
            sysinfo::System::os_version()
                .map(|version| (BuiltInField::PlatformVersion, Value::String(version))),
        )
        .collect();
        let user_properties = [
            (
                UserField::AppVersion,
                Value::String(env!("CARGO_PKG_VERSION").to_owned()),
            ),
            (
                UserField::ReleaseChannel,
                Value::String(
                    if cfg!(debug_assertions) {
                        "debug"
                    } else {
                        "stable"
                    }
                    .to_owned(),
                ),
            ),
        ]
        .into();
        Self {
            session_id,
            client_id: client_id_from_machine(
                &machine_id::get().expect("machine identifier must be available"),
            ),
            sequence: 0,
            request_context,
            user_properties,
            engagement: Engagement::new(Instant::now()),
            sender: spawn_worker(|| Arc::new(HttpCollector::new())),
        }
    }

    fn publish(&mut self, name: &str, fields: EventFields) {
        let engagement_millis = self.engagement.take_event_milliseconds(Instant::now());
        let mut built_in = self.request_context.clone();
        built_in.insert(BuiltInField::Version, Value::String("2".to_owned()));
        built_in.insert(
            BuiltInField::MeasurementId,
            Value::String(MEASUREMENT_ID.to_owned()),
        );
        built_in.insert(
            BuiltInField::ClientId,
            Value::String(self.client_id.clone()),
        );
        built_in.insert(
            BuiltInField::ProcessStart,
            Value::from(self.session_id.saturating_mul(1_000)),
        );
        built_in.insert(BuiltInField::SessionId, Value::from(self.session_id));
        built_in.insert(BuiltInField::SessionCount, Value::from(1));
        built_in.insert(BuiltInField::Fallback, Value::from(1));
        built_in.insert(BuiltInField::NonPersonalizedAds, Value::from(true));
        built_in.insert(BuiltInField::EventName, Value::String(name.to_owned()));
        self.sequence = self.sequence.saturating_add(1);
        built_in.insert(BuiltInField::Sequence, Value::from(self.sequence));
        if self.sequence == 1 {
            built_in.insert(BuiltInField::SessionStart, Value::from(true));
        }
        if self.engagement.engaged {
            built_in.insert(BuiltInField::EngagedSession, Value::from(true));
        }
        if engagement_millis > 0 || cfg!(debug_assertions) {
            built_in.insert(
                BuiltInField::EngagementTime,
                Value::from(engagement_millis.max(1)),
            );
        }
        if cfg!(debug_assertions) {
            built_in.insert(BuiltInField::DebugMode, Value::from(true));
        }

        _ = self.sender.send(Message::Event(Payload {
            built_in,
            event: fields,
            user: self.user_properties.clone(),
        }))
    }

    fn focused(&mut self, focused: bool) {
        if self.engagement.focused == focused {
            return;
        }
        self.engagement.set_focused(focused, Instant::now());
        if !focused && self.engagement.unreported >= Duration::from_secs(1) {
            self.publish("user_engagement", EventFields::new());
        }
    }

    fn flush(&self) {
        let (complete, wait) = std_mpsc::sync_channel(0);
        if self.sender.send(Message::Flush(complete)).is_ok() {
            let _ = wait.recv_timeout(Duration::from_secs(2));
        }
    }

    fn shutdown(&mut self) {
        self.engagement.advance(Instant::now());
        if self.engagement.unreported >= Duration::from_secs(1) {
            self.publish("user_engagement", EventFields::new());
        }
        let (complete, wait) = std_mpsc::sync_channel(0);
        if self.sender.send(Message::Shutdown(complete)).is_ok() {
            let _ = wait.recv_timeout(Duration::from_secs(2));
        }
    }
}

pub fn context(value: Value) {
    let fields = value
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(name, value)| {
            name.parse::<UserField>()
                .ok()
                .map(|field| (field, value.clone()))
        })
        .collect::<UserFields>();
    METRICS.lock().user_properties.extend(fields);
}

pub fn request_context(fields: BuiltInFields) {
    METRICS.lock().request_context.extend(fields);
}

#[must_use]
pub fn layer() -> MetricsLayer {
    let _ = &*METRICS;
    MetricsLayer
}

/// Updates the native foreground state used for GA4 engagement timing.
pub fn focused(focused: bool) {
    METRICS.lock().focused(focused);
}

/// Waits briefly for queued events to be handed to the transport.
pub fn flush() {
    METRICS.lock().flush();
}

/// Emits pending engagement and performs a bounded final transport flush.
pub fn shutdown() {
    METRICS.lock().shutdown();
}

pub struct MetricsLayer;

impl<S> Layer<S> for MetricsLayer
where
    S: Subscriber + for<'lookup> LookupSpan<'lookup>,
{
    fn on_new_span(
        &self,
        attributes: &span::Attributes<'_>,
        id: &span::Id,
        context: LayerContext<'_, S>,
    ) {
        if attributes.metadata().target() != TARGET {
            return;
        }
        let Some(span) = context.span(id) else {
            return;
        };
        let mut fields = FieldVisitor::default();
        attributes.record(&mut fields);
        span.extensions_mut().insert(MetricSpan {
            name: attributes.metadata().name().to_owned(),
            started: Instant::now(),
            fields: fields.values,
        });
    }

    fn on_record(&self, id: &span::Id, values: &span::Record<'_>, context: LayerContext<'_, S>) {
        let Some(span) = context.span(id) else {
            return;
        };
        let mut extensions = span.extensions_mut();
        let Some(metric) = extensions.get_mut::<MetricSpan>() else {
            return;
        };
        let mut fields = FieldVisitor::default();
        values.record(&mut fields);
        metric.fields.extend(fields.values);
    }

    fn on_event(&self, event: &Event<'_>, _context: LayerContext<'_, S>) {
        if event.metadata().target() != TARGET {
            return;
        }
        let mut fields = FieldVisitor::default();
        event.record(&mut fields);
        let Some(name) = fields.metric else {
            return;
        };
        METRICS.lock().publish(&name, fields.values);
    }

    fn on_close(&self, id: span::Id, context: LayerContext<'_, S>) {
        let Some(span) = context.span(&id) else {
            return;
        };
        let Some(metric) = span.extensions_mut().remove::<MetricSpan>() else {
            return;
        };
        let fields = [(
            EventField::DurationMs,
            Value::from(metric.started.elapsed().as_secs_f64() * 1000.0),
        )]
        .into_iter()
        .chain(metric.fields)
        .collect();
        METRICS.lock().publish(&metric.name, fields);
    }
}

struct MetricSpan {
    name: String,
    started: Instant,
    fields: EventFields,
}

#[derive(Default)]
struct FieldVisitor {
    metric: Option<String>,
    values: EventFields,
}

impl FieldVisitor {
    fn insert(&mut self, field: &tracing::field::Field, value: Value) {
        if field.name() == "metric" {
            self.metric = value.as_str().map(str::to_owned);
        } else if let Ok(field) = field.name().parse() {
            self.values.insert(field, value);
        }
    }
}

impl Visit for FieldVisitor {
    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.insert(field, Value::from(value));
    }

    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.insert(field, Value::from(value));
    }

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.insert(field, Value::from(value));
    }

    fn record_f64(&mut self, field: &tracing::field::Field, value: f64) {
        if value.is_finite() {
            self.insert(field, Value::from(value));
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.insert(field, Value::String(value.to_owned()));
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.insert(field, Value::String(format!("{value:?}")));
    }
}

enum Message {
    Event(Payload),
    Flush(std_mpsc::SyncSender<()>),
    Shutdown(std_mpsc::SyncSender<()>),
}

fn spawn_worker<T>(
    transport: impl FnOnce() -> Arc<T> + Send + 'static,
) -> tokio::sync::mpsc::UnboundedSender<Message>
where
    T: Collector + 'static,
{
    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    let _ = std::thread::Builder::new()
        .name("koharu-metrics".to_owned())
        .spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("metrics Tokio runtime must be created")
                .block_on(async move {
                    let transport = transport();
                    while let Some(message) = receiver.recv().await {
                        match message {
                            Message::Event(event) => {
                                let _ = transport.send(&Request::new(&event)).await;
                            }
                            Message::Flush(complete) => {
                                let _ = complete.send(());
                            }
                            Message::Shutdown(complete) => {
                                let _ = complete.send(());
                                return;
                            }
                        }
                    }
                });
        });
    sender
}

trait Collector: Send + Sync {
    async fn send(&self, request: &Request) -> Result<(), String>;
}

struct HttpCollector {
    client: reqwest::Client,
}

impl HttpCollector {
    fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(2))
                .user_agent(format!(
                    "Koharu/{} (CEF; {}; {})",
                    env!("CARGO_PKG_VERSION"),
                    std::env::consts::OS,
                    std::env::consts::ARCH
                ))
                .build()
                .expect("metrics HTTP client must be created"),
        }
    }
}

impl Collector for HttpCollector {
    async fn send(&self, request: &Request) -> Result<(), String> {
        self.client
            .post(request.url.clone())
            .header(reqwest::header::CACHE_CONTROL, "no-cache")
            .header(reqwest::header::PRAGMA, "no-cache")
            .header(reqwest::header::CONTENT_TYPE, "text/plain;charset=UTF-8")
            .body(request.body.clone())
            .send()
            .await
            .map_err(|error| error.to_string())?
            .error_for_status()
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
}

struct Request {
    url: url::Url,
    body: String,
}

struct Payload {
    built_in: BuiltInFields,
    event: EventFields,
    user: UserFields,
}

impl Request {
    fn new(payload: &Payload) -> Self {
        let mut query = url::form_urlencoded::Serializer::new(String::new());
        let mut body = url::form_urlencoded::Serializer::new(String::new());
        for (field, value) in &payload.built_in {
            if let Some((name, value)) = field.encode(value) {
                if matches!(
                    *field,
                    BuiltInField::EventName
                        | BuiltInField::SessionStart
                        | BuiltInField::EngagementTime
                        | BuiltInField::DebugMode
                ) {
                    body.append_pair(&name, &value);
                } else {
                    query.append_pair(&name, &value);
                }
            }
        }
        for (field, value) in &payload.event {
            if let Some((name, value)) = field.encode(value) {
                body.append_pair(&name, &value);
            }
        }
        for (field, value) in &payload.user {
            if let Some((name, value)) = field.encode(value) {
                body.append_pair(&name, &value);
            }
        }
        Self {
            url: url::Url::parse(&format!("{ENDPOINT}?{}", query.finish()))
                .expect("metrics collection URL must be valid"),
            body: body.finish(),
        }
    }
}

fn client_id_from_machine(machine_id: &str) -> String {
    let digest = blake3::derive_key("dev.koharu.metrics.client-id.v1", machine_id.as_bytes());
    let first = u64::from_be_bytes(digest[0..8].try_into().expect("digest length"));
    let second = u64::from_be_bytes(digest[8..16].try_into().expect("digest length"));
    format!("{}.{}", first.max(1), second.max(1))
}

struct Engagement {
    focused: bool,
    accounted_at: Instant,
    unreported: Duration,
    total: Duration,
    engaged: bool,
}

impl Engagement {
    fn new(now: Instant) -> Self {
        Self {
            focused: true,
            accounted_at: now,
            unreported: Duration::ZERO,
            total: Duration::ZERO,
            engaged: false,
        }
    }

    fn advance(&mut self, now: Instant) {
        if self.focused {
            let elapsed = now.saturating_duration_since(self.accounted_at);
            self.unreported += elapsed;
            self.total += elapsed;
            if self.total >= Duration::from_secs(10) {
                self.engaged = true;
            }
        }
        self.accounted_at = now;
    }

    fn take_event_milliseconds(&mut self, now: Instant) -> u64 {
        self.advance(now);
        let milliseconds = self.unreported.as_millis().try_into().unwrap_or(u64::MAX);
        self.unreported = Duration::ZERO;
        milliseconds
    }

    fn set_focused(&mut self, focused: bool, now: Instant) {
        self.advance(now);
        self.focused = focused;
        self.accounted_at = now;
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, sync::Mutex as StdMutex};

    use super::*;

    struct RecordingCollector {
        requests: StdMutex<Vec<(String, String)>>,
    }

    impl RecordingCollector {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                requests: StdMutex::new(Vec::new()),
            })
        }
    }

    impl Collector for RecordingCollector {
        async fn send(&self, request: &Request) -> Result<(), String> {
            self.requests
                .lock()
                .expect("test collector lock poisoned")
                .push((request.url.as_str().to_owned(), request.body.clone()));
            Ok(())
        }
    }

    #[test]
    fn client_id_is_stable_and_dotted_decimal() {
        let first = client_id_from_machine("machine");
        assert_eq!(first, client_id_from_machine("machine"));
        let mut pieces = first.split('.');
        assert!(
            pieces
                .next()
                .is_some_and(|piece| piece.parse::<u64>().is_ok_and(|value| value > 0))
        );
        assert!(
            pieces
                .next()
                .is_some_and(|piece| piece.parse::<u64>().is_ok_and(|value| value > 0))
        );
        assert!(pieces.next().is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn metrics_initialize_inside_async_runtime() {
        drop(Metrics::new());
    }

    #[test]
    fn request_keeps_event_data_in_the_body() {
        let request = Request::new(&Payload {
            built_in: [
                (BuiltInField::Version, Value::String("2".to_owned())),
                (
                    BuiltInField::MeasurementId,
                    Value::String(MEASUREMENT_ID.to_owned()),
                ),
                (BuiltInField::EventName, Value::String("typed".to_owned())),
                (BuiltInField::DebugMode, Value::from(true)),
            ]
            .into(),
            event: [
                (EventField::Model, Value::String("a&b c".to_owned())),
                (EventField::DurationMs, Value::from(42)),
                (EventField::Enabled, Value::from(true)),
            ]
            .into(),
            user: [(UserField::AppVersion, Value::String("1.0.0".to_owned()))].into(),
        });
        let query = url::form_urlencoded::parse(
            request
                .url
                .query()
                .expect("metrics request must have a query")
                .as_bytes(),
        )
        .collect::<BTreeMap<_, _>>();
        let body = url::form_urlencoded::parse(request.body.as_bytes()).collect::<BTreeMap<_, _>>();
        assert_eq!(query.get("v").map(|value| value.as_ref()), Some("2"));
        assert_eq!(
            query.get("tid").map(|value| value.as_ref()),
            Some(MEASUREMENT_ID)
        );
        assert_eq!(body.get("en").map(|value| value.as_ref()), Some("typed"));
        assert_eq!(
            body.get("ep.model").map(|value| value.as_ref()),
            Some("a&b c")
        );
        assert_eq!(
            body.get("epn.duration_ms").map(|value| value.as_ref()),
            Some("42")
        );
        assert_eq!(
            body.get("ep.enabled").map(|value| value.as_ref()),
            Some("1")
        );
        assert_eq!(
            body.get("up.app_version").map(|value| value.as_ref()),
            Some("1.0.0")
        );
        assert_eq!(
            body.get("ep.debug_mode").map(|value| value.as_ref()),
            Some("1")
        );
        assert!(!request.url.as_str().contains("ep."));
        assert!(!request.url.as_str().contains("up."));
    }

    #[test]
    fn metrics_publish_debug_events_and_user_properties() {
        let collector = RecordingCollector::new();
        let mut metrics = Metrics::new();
        metrics.sender = spawn_worker({
            let collector = collector.clone();
            move || collector
        });
        metrics
            .user_properties
            .extend([(UserField::ComputeBackend, Value::String("wgpu".to_owned()))]);
        metrics.publish("event_one", EventFields::new());
        metrics.publish("event_two", EventFields::new());
        metrics.flush();
        let requests = collector.requests.lock().expect("collector lock");
        assert_eq!(requests.len(), 2);
        assert!(requests.iter().all(|(url, _)| url.starts_with(ENDPOINT)));
        assert!(requests.iter().all(|(url, _)| url.contains("_p=")));
        assert!(requests.iter().all(|(url, _)| url.contains("sct=1")));
        assert_eq!(
            requests
                .iter()
                .map(|(_, body)| body.matches("up.compute_backend").count())
                .sum::<usize>(),
            2
        );
        assert!(
            requests
                .iter()
                .all(|(_, body)| body.contains("ep.debug_mode=1"))
        );
        assert!(requests.iter().all(|(_, body)| body.contains("_et=")));
    }

    #[test]
    fn engagement_accumulates_only_while_focused() {
        let now = Instant::now();
        let mut engagement = Engagement::new(now);
        engagement.advance(now + Duration::from_secs(3));
        assert_eq!(
            engagement.take_event_milliseconds(now + Duration::from_secs(3)),
            3_000
        );
        engagement.set_focused(false, now + Duration::from_secs(3));
        engagement.advance(now + Duration::from_secs(8));
        assert_eq!(
            engagement.take_event_milliseconds(now + Duration::from_secs(8)),
            0
        );
        engagement.set_focused(true, now + Duration::from_secs(8));
        engagement.advance(now + Duration::from_secs(10));
        assert_eq!(
            engagement.take_event_milliseconds(now + Duration::from_secs(10)),
            2_000
        );
    }

    #[test]
    fn channel_sends_each_event() {
        let collector = RecordingCollector::new();
        let sender = spawn_worker({
            let collector = collector.clone();
            move || collector
        });
        (0..3).for_each(|index| {
            sender
                .send(Message::Event(Payload {
                    built_in: [(
                        BuiltInField::EventName,
                        Value::String(format!("event_{index}")),
                    )]
                    .into(),
                    event: EventFields::new(),
                    user: UserFields::new(),
                }))
                .expect("metrics worker must be available");
        });
        let (complete, wait) = std_mpsc::sync_channel(0);
        sender
            .send(Message::Flush(complete))
            .expect("metrics worker must be available");
        wait.recv_timeout(Duration::from_secs(2))
            .expect("metrics worker must flush");
        assert_eq!(collector.requests.lock().expect("collector lock").len(), 3);
        let (complete, wait) = std_mpsc::sync_channel(0);
        sender
            .send(Message::Shutdown(complete))
            .expect("metrics worker must be available");
        wait.recv_timeout(Duration::from_secs(2))
            .expect("metrics worker must shut down");
    }
}
