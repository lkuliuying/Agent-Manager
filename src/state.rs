use crate::source::{SessionKey, SourceId};
use chrono::{Local, TimeZone};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq)]
pub struct Message {
    pub id: String,
    pub author: String,
    pub kind: String,
    pub text: String,
    pub status: String,
    pub time: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskStatus {
    Running,
    Completed,
    Failed,
    Interrupted,
    Idle,
    Unknown,
}
impl TaskStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Running => "运行中",
            Self::Completed => "本轮完成",
            Self::Failed => "失败",
            Self::Interrupted => "已中断",
            Self::Idle => "空闲",
            Self::Unknown => "状态未知",
        }
    }
    pub fn code(self) -> i32 {
        match self {
            Self::Running => 1,
            Self::Completed => 2,
            Self::Failed => 3,
            Self::Interrupted => 4,
            Self::Idle => 0,
            Self::Unknown => 5,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Session {
    pub id: String,
    pub native_id: String,
    pub source: SourceId,
    pub version: String,
    pub fresh: bool,
    pub title: String,
    pub project: String,
    pub status: TaskStatus,
    pub messages: Vec<Message>,
    pub message_version: u64,
    pub activity: String,
    pub created: String,
    pub last_sync: String,
    pub history_complete: bool,
    pub messages_loaded: bool,
}
impl Session {
    pub fn new(native_id: String, _owner: String) -> Self {
        Self::from_source(native_id, SourceId::codex())
    }
    pub fn from_source(native_id: String, source: SourceId) -> Self {
        Self {
            id: SessionKey {
                source: source.clone(),
                native_id: native_id.clone(),
            }
            .encoded(),
            native_id,
            version: source.kind.supported_version().into(),
            source,
            fresh: false,
            title: "正在同步会话…".into(),
            project: String::new(),
            status: TaskStatus::Unknown,
            messages: Vec::new(),
            message_version: 0,
            activity: "等待会话快照".into(),
            created: "—".into(),
            last_sync: "—".into(),
            history_complete: false,
            messages_loaded: false,
        }
    }
}

pub fn string<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}
pub fn timestamp(ms: i64, short: bool) -> String {
    if ms <= 0 {
        return "时间未知".into();
    }
    Local
        .timestamp_millis_opt(ms)
        .single()
        .map(|t| {
            t.format(if short { "%H:%M" } else { "%Y-%m-%d %H:%M:%S" })
                .to_string()
        })
        .unwrap_or_else(|| "时间未知".into())
}
pub fn now() -> String {
    Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}
