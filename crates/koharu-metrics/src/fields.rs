use std::collections::BTreeMap;

use serde_json::Value;

pub type BuiltInFields = BTreeMap<BuiltInField, Value>;
pub type EventFields = BTreeMap<EventField, Value>;
pub type UserFields = BTreeMap<UserField, Value>;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, strum::AsRefStr)]
pub enum BuiltInField {
    #[strum(serialize = "v")]
    Version,
    #[strum(serialize = "tid")]
    MeasurementId,
    #[strum(serialize = "cid")]
    ClientId,
    #[strum(serialize = "_p")]
    ProcessStart,
    #[strum(serialize = "sid")]
    SessionId,
    #[strum(serialize = "sct")]
    SessionCount,
    #[strum(serialize = "gaf")]
    Fallback,
    #[strum(serialize = "seg")]
    EngagedSession,
    #[strum(serialize = "npa")]
    NonPersonalizedAds,
    #[strum(serialize = "en")]
    EventName,
    #[strum(serialize = "_s")]
    Sequence,
    #[strum(serialize = "_ss")]
    SessionStart,
    #[strum(serialize = "_et")]
    EngagementTime,
    #[strum(serialize = "ep.debug_mode")]
    DebugMode,
    #[strum(serialize = "ul")]
    Language,
    #[strum(serialize = "sr")]
    ScreenResolution,
    #[strum(serialize = "uaa")]
    Architecture,
    #[strum(serialize = "uab")]
    Bitness,
    #[strum(serialize = "uafvl")]
    BrowserVersion,
    #[strum(serialize = "uamb")]
    Mobile,
    #[strum(serialize = "uap")]
    Platform,
    #[strum(serialize = "uapv")]
    PlatformVersion,
}

#[derive(
    Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, strum::AsRefStr, strum::EnumString,
)]
#[strum(serialize_all = "snake_case")]
pub enum EventField {
    Attempt,
    Bytes,
    Changed,
    CharacterCount,
    CompletedCount,
    DurationMs,
    Empty,
    Enabled,
    EntityCount,
    Error,
    Format,
    Height,
    Method,
    Mode,
    Model,
    Operation,
    Origin,
    Outcome,
    PageCount,
    PageNumber,
    Percentage,
    Phase,
    PointCount,
    Provider,
    Reason,
    Resource,
    Scope,
    SegmentCount,
    Setting,
    Stage,
    StageCount,
    State,
    Size,
    TargetLanguage,
    Tool,
    ToolCount,
    TotalBytes,
    TotalCount,
    UsedBytes,
    Vision,
    Width,
}

#[derive(
    Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, strum::AsRefStr, strum::EnumString,
)]
#[strum(serialize_all = "snake_case")]
pub enum UserField {
    AppVersion,
    ReleaseChannel,
    ComputeBackend,
    DeviceType,
    GpuModel,
    VramBytes,
    CpuCoreCount,
}

impl BuiltInField {
    pub(crate) fn encode(self, value: &Value) -> Option<(String, String)> {
        Some((self.as_ref().to_owned(), encode_value(value, 100)?))
    }
}

impl EventField {
    pub(crate) fn encode(self, value: &Value) -> Option<(String, String)> {
        Some((
            format!(
                "ep{}.{}",
                if value.is_number() { "n" } else { "" },
                self.as_ref()
            ),
            encode_value(value, 100)?,
        ))
    }
}

impl UserField {
    pub(crate) fn encode(self, value: &Value) -> Option<(String, String)> {
        Some((
            format!(
                "up{}.{}",
                if value.is_number() { "n" } else { "" },
                self.as_ref()
            ),
            encode_value(value, 36)?,
        ))
    }
}

fn encode_value(value: &Value, string_limit: usize) -> Option<String> {
    match value {
        Value::String(value) => Some(value.chars().take(string_limit).collect()),
        Value::Number(value) => Some(value.to_string()),
        Value::Bool(value) => Some(if *value { "1" } else { "0" }.to_owned()),
        Value::Null => Some(String::new()),
        Value::Array(_) | Value::Object(_) => None,
    }
}
