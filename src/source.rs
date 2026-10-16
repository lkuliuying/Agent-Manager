use serde::{Deserialize, Serialize};

#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum SourceKind {
    #[default]
    Codex,
    Opencode,
    Deepseek,
}
impl SourceKind {
    pub const ALL: [Self; 3] = [Self::Codex, Self::Opencode, Self::Deepseek];
    pub fn label(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::Opencode => "OpenCode",
            Self::Deepseek => "DeepSeek Harness",
        }
    }
    pub fn code(self) -> i32 {
        match self {
            Self::Codex => 1,
            Self::Opencode => 2,
            Self::Deepseek => 3,
        }
    }
    pub fn from_code(value: i32) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.code() == value)
    }
    pub fn supported_version(self) -> &'static str {
        match self {
            Self::Codex => "Desktop IPC v1",
            Self::Opencode => "1.18.27",
            Self::Deepseek => "0.2.0-rc.2",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SourceId {
    pub kind: SourceKind,
    pub instance: String,
}
impl SourceId {
    pub fn codex() -> Self {
        Self {
            kind: SourceKind::Codex,
            instance: "desktop".into(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionKey {
    pub source: SourceId,
    pub native_id: String,
}
impl SessionKey {
    pub fn encoded(&self) -> String {
        serde_json::to_string(self).expect("会话键可序列化")
    }
}

#[derive(Clone, Debug)]
pub struct SourceConnection {
    pub connected: bool,
    pub note: String,
    pub version: String,
}
impl SourceConnection {
    pub fn waiting(kind: SourceKind) -> Self {
        Self {
            connected: false,
            note: if kind == SourceKind::Codex {
                "正在连接"
            } else {
                "等待观察插件连接"
            }
            .into(),
            version: kind.supported_version().into(),
        }
    }
}
