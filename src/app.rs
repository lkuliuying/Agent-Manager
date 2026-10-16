use crate::{
    AppWindow, BlockRow, CardGroup, MessageRow, SessionRow, SourceRow, gate, installation,
    monitor::{self, Command, Event, Metrics},
    observer,
    source::{SessionKey, SourceConnection, SourceId, SourceKind},
    state::{Message, Session, TaskStatus, now},
};
use pulldown_cmark::{Event as MarkdownEvent, Parser, Tag, TagEnd};
use slint::{ComponentHandle, Model, ModelRc, Timer, TimerMode, VecModel};
use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap, HashSet},
    path::Path,
    rc::Rc,
    time::{Duration, Instant},
};

#[path = "ui_check.rs"]
mod ui_check;

#[derive(Default)]
struct Reading {
    y: f32,
    follow: bool,
}
struct Cached {
    source: Message,
    fresh: bool,
    expanded: bool,
    row: MessageRow,
}
struct Models {
    card: Rc<VecModel<MessageRow>>,
    detail: Rc<VecModel<MessageRow>>,
    cached: HashMap<(String, bool), Cached>,
}
impl Default for Models {
    fn default() -> Self {
        Self {
            card: Rc::new(VecModel::default()),
            detail: Rc::new(VecModel::default()),
            cached: HashMap::new(),
        }
    }
}
struct State {
    sessions: BTreeMap<String, Session>,
    order: Vec<String>,
    watched: Vec<String>,
    manual_selection: bool,
    hidden: HashSet<String>,
    readings: HashMap<(String, bool), Reading>,
    expanded: HashSet<(String, String, bool)>,
    models: HashMap<String, Models>,
    sidebar: Rc<VecModel<SessionRow>>,
    detail_rows: Rc<VecModel<SessionRow>>,
    groups: Rc<VecModel<CardGroup>>,
    card_models: Vec<Rc<VecModel<SessionRow>>>,
    selected: String,
    query: String,
    filter: i32,
    columns: usize,
    dirty: bool,
    offline: bool,
    connection: String,
    metrics: Metrics,
    toast_until: Option<Instant>,
    source_filter: i32,
    sources: HashMap<SourceId, (String, SourceConnection)>,
    source_rows: Rc<VecModel<SourceRow>>,
    installations: HashMap<SourceKind, installation::Info>,
    installing: HashSet<SourceKind>,
    interests: HashSet<SessionKey>,
    source_faults: HashMap<SourceKind, String>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            sessions: BTreeMap::new(),
            order: Vec::new(),
            watched: Vec::new(),
            manual_selection: false,
            hidden: HashSet::new(),
            readings: HashMap::new(),
            expanded: HashSet::new(),
            models: HashMap::new(),
            sidebar: Rc::new(VecModel::default()),
            detail_rows: Rc::new(VecModel::default()),
            groups: Rc::new(VecModel::default()),
            card_models: Vec::new(),
            selected: String::new(),
            query: String::new(),
            filter: 0,
            columns: 3,
            dirty: true,
            offline: true,
            connection: "正在连接".into(),
            metrics: Metrics::default(),
            toast_until: None,
            source_filter: 0,
            sources: HashMap::new(),
            source_rows: Rc::new(VecModel::default()),
            installations: HashMap::new(),
            installing: HashSet::new(),
            interests: HashSet::new(),
            source_faults: HashMap::new(),
        }
    }
}
fn text_limit(text: &str, lines: usize, chars: usize) -> String {
    let shortened = text.lines().take(lines).collect::<Vec<_>>().join("\n");
    let mut result = shortened.chars().take(chars).collect::<String>();
    if result.len() < text.len() {
        result.push_str("\n…");
    }
    result
}
fn blocks(text: &str, tool: bool) -> Vec<BlockRow> {
    if tool {
        return vec![BlockRow {
            text: text.into(),
            code: true,
            heading: 0,
        }];
    }
    let mut rows = Vec::new();
    let mut buffer = String::new();
    let mut code = false;
    let mut heading = 0;
    let flush = |rows: &mut Vec<BlockRow>, buffer: &mut String, code: bool, heading: i32| {
        if !buffer.trim().is_empty() {
            rows.push(BlockRow {
                text: std::mem::take(buffer).trim_end().into(),
                code,
                heading,
            });
        }
    };
    for event in Parser::new(text) {
        match event {
            MarkdownEvent::Start(Tag::CodeBlock(_)) => {
                flush(&mut rows, &mut buffer, code, heading);
                code = true;
            }
            MarkdownEvent::End(TagEnd::CodeBlock) => {
                flush(&mut rows, &mut buffer, code, heading);
                code = false;
            }
            MarkdownEvent::Start(Tag::Heading { .. }) => {
                flush(&mut rows, &mut buffer, code, heading);
                heading = 1;
            }
            MarkdownEvent::End(TagEnd::Heading(_)) => {
                flush(&mut rows, &mut buffer, code, heading);
                heading = 0;
            }
            MarkdownEvent::Start(Tag::Item) => buffer.push_str("• "),
            MarkdownEvent::End(TagEnd::Paragraph | TagEnd::Item) => {
                flush(&mut rows, &mut buffer, code, heading)
            }
            MarkdownEvent::Text(t)
            | MarkdownEvent::Code(t)
            | MarkdownEvent::Html(t)
            | MarkdownEvent::InlineHtml(t) => buffer.push_str(&t),
            MarkdownEvent::SoftBreak | MarkdownEvent::HardBreak => buffer.push('\n'),
            MarkdownEvent::Rule => {
                flush(&mut rows, &mut buffer, code, heading);
            }
            _ => {}
        }
    }
    flush(&mut rows, &mut buffer, code, heading);
    if rows.is_empty() {
        rows.push(BlockRow {
            text: text.into(),
            code: false,
            heading: 0,
        });
    }
    rows
}
fn tool_status(message: &Message, fresh: bool) -> (String, i32) {
    match message.status.as_str() {
        "completed" => ("执行成功".into(), 1),
        "failed" | "error" => ("执行失败".into(), 3),
        "inProgress" | "in_progress" | "running" => (
            if fresh {
                "执行中"
            } else {
                "上次执行中"
            }
            .into(),
            if fresh { 2 } else { 6 },
        ),
        "interrupted" | "cancelled" => ("已中断".into(), 0),
        _ => ("已记录".into(), 0),
    }
}
fn sync_rows<T: Clone + PartialEq + 'static>(model: &VecModel<T>, rows: Vec<T>) {
    while model.row_count() > rows.len() {
        model.remove(model.row_count() - 1);
    }
    for (i, row) in rows.into_iter().enumerate() {
        if i >= model.row_count() {
            model.push(row);
        } else if model.row_data(i).as_ref() != Some(&row) {
            model.set_row_data(i, row);
        }
    }
}
impl State {
    fn observer_event(&mut self, event: observer::Event) {
        match event {
            observer::Event::Connection(source, epoch, connection) => {
                if !connection.connected
                    && self
                        .sources
                        .get(&source)
                        .is_some_and(|(current, _)| current != &epoch)
                {
                    return;
                }
                if connection.connected
                    || self
                        .sources
                        .get(&source)
                        .is_some_and(|(current, _)| current == &epoch)
                {
                    for session in self.sessions.values_mut().filter(|s| s.source == source) {
                        session.fresh = false;
                    }
                    self.sources.insert(source, (epoch, connection));
                }
            }
            observer::Event::Catalog(source, epoch, sessions) => {
                if !self
                    .sources
                    .get(&source)
                    .is_some_and(|(current, c)| current == &epoch && c.connected)
                {
                    return;
                }
                self.source_faults.remove(&source.kind);
                let ids: HashSet<_> = sessions.iter().map(|s| s.id.clone()).collect();
                for session in self
                    .sessions
                    .values_mut()
                    .filter(|s| s.source == source && !ids.contains(&s.id))
                {
                    session.fresh = false;
                }
                for mut session in sessions {
                    if let Some(previous) = self.sessions.get(&session.id) {
                        session.messages = previous.messages.clone();
                        session.messages_loaded = previous.messages_loaded;
                        session.message_version = previous.message_version;
                        session.history_complete = previous.history_complete;
                        // 目录更新不能证明缓存正文已追上新连接；标题仍以最新目录为准。
                        if previous.messages_loaded {
                            session.fresh = previous.fresh;
                            if session.status == TaskStatus::Unknown {
                                session.status = previous.status;
                                session.activity = previous.activity.clone();
                            }
                        }
                    }
                    self.ingest(session);
                }
            }
            observer::Event::Session(epoch, session) => {
                if !self
                    .sources
                    .get(&session.source)
                    .is_some_and(|(current, c)| current == &epoch && c.connected)
                {
                    return;
                }
                self.source_faults.remove(&session.source.kind);
                let mut session = *session;
                session.message_version = self
                    .sessions
                    .get(&session.id)
                    .map(|s| s.message_version + u64::from(s.messages != session.messages))
                    .unwrap_or(1);
                self.ingest(session);
            }
            observer::Event::SessionError(source, epoch, id) => {
                if !self
                    .sources
                    .get(&source)
                    .is_some_and(|(current, _)| current == &epoch)
                {
                    return;
                }
                if let Some(session) = self.sessions.get_mut(
                    &SessionKey {
                        source,
                        native_id: id,
                    }
                    .encoded(),
                ) {
                    session.fresh = false;
                    session.activity = "读取失败，请重新扫描".into();
                }
            }
            observer::Event::Error(kind, note) => {
                self.source_faults.insert(kind, note);
            }
        }
        self.dirty = true;
    }
    fn wanted(&self, detail: bool) -> HashSet<SessionKey> {
        let mut ids = self.watched_ids();
        if detail {
            ids.push(self.selected.clone());
        }
        ids.into_iter()
            .filter_map(|id| self.sessions.get(&id))
            .filter(|s| s.source.kind != SourceKind::Codex)
            .map(|s| SessionKey {
                source: s.source.clone(),
                native_id: s.native_id.clone(),
            })
            .collect()
    }
    fn watched_ids(&self) -> Vec<String> {
        if self.manual_selection || !self.watched.is_empty() {
            self.watched.clone()
        } else {
            self.order
                .iter()
                .filter(|id| !self.hidden.contains(*id))
                .take(3)
                .cloned()
                .collect()
        }
    }
    fn toggle_watch(&mut self, id: &str) {
        if !self.manual_selection {
            self.watched = self.watched_ids();
            self.manual_selection = true;
        }
        if self.watched.iter().any(|selected| selected == id) {
            self.watched.retain(|selected| selected != id);
            self.hidden.insert(id.into());
        } else {
            self.hidden.remove(id);
            self.watched.push(id.into());
        }
        self.dirty = true;
    }
    fn ingest(&mut self, session: Session) {
        let id = session.id.clone();
        if !self.sessions.contains_key(&id) {
            self.order.push(id.clone());
            if self.selected.is_empty() {
                self.selected = id.clone();
            }
        }
        if !self.hidden.contains(&id)
            && !self.watched.contains(&id)
            && session.status == TaskStatus::Running
        {
            self.watched.push(id.clone());
        }
        self.sessions.insert(id, session);
        self.dirty = true;
    }
    fn row(&mut self, id: &str, detail: bool) -> SessionRow {
        let selected = self.watched_ids().iter().any(|selected| selected == id);
        let s = &self.sessions[id];
        let models = self.models.entry(id.into()).or_default();
        for mode in [false, true] {
            let mut rows = Vec::new();
            for message in &s.messages {
                let expanded = self
                    .expanded
                    .contains(&(id.into(), message.id.clone(), mode));
                let key = (message.id.clone(), mode);
                let row = if let Some(cached) = models.cached.get(&key).filter(|cached| {
                    cached.source == *message
                        && cached.fresh == s.fresh
                        && cached.expanded == expanded
                }) {
                    cached.row.clone()
                } else {
                    let tool = message.author == "工具";
                    let limit = if mode { 2000 } else { 650 };
                    let can_expand = message.text.chars().count() > limit
                        || message.text.lines().count() > if tool { 4 } else { 9 };
                    let text = if can_expand && !expanded {
                        text_limit(&message.text, if tool { 4 } else { 9 }, limit)
                    } else {
                        message.text.clone()
                    };
                    let (status, status_code) = tool_status(message, s.fresh);
                    let content = models
                        .cached
                        .get(&key)
                        .filter(|cached| {
                            cached.source.text == message.text && cached.expanded == expanded
                        })
                        .map(|cached| cached.row.blocks.clone())
                        .unwrap_or_else(|| {
                            ModelRc::from(Rc::new(VecModel::from(blocks(&text, tool))))
                        });
                    let row = MessageRow {
                        id: message.id.as_str().into(),
                        author: message.author.as_str().into(),
                        kind: message.kind.as_str().into(),
                        time: message.time.as_str().into(),
                        status: status.into(),
                        status_code,
                        tool,
                        user: message.author == "我",
                        expanded,
                        can_expand,
                        blocks: content,
                    };
                    models.cached.insert(
                        key,
                        Cached {
                            source: message.clone(),
                            fresh: s.fresh,
                            expanded,
                            row: row.clone(),
                        },
                    );
                    row
                };
                rows.push(row);
            }
            sync_rows(
                if mode {
                    models.detail.as_ref()
                } else {
                    models.card.as_ref()
                },
                rows,
            );
        }
        let reading = self.readings.entry((id.into(), detail)).or_insert(Reading {
            y: 0.,
            follow: true,
        });
        SessionRow {
            id: id.into(),
            native_id: s.native_id.as_str().into(),
            messages_loaded: s.messages_loaded,
            source: s.source.kind.label().into(),
            source_code: s.source.kind.code(),
            version: s.version.as_str().into(),
            title: s.title.as_str().into(),
            project: s.project.as_str().into(),
            status: s.status.label().into(),
            status_code: s.status.code(),
            fresh: s.fresh,
            activity: s.activity.as_str().into(),
            created: s.created.as_str().into(),
            synced: s.last_sync.as_str().into(),
            messages: ModelRc::from(if detail {
                models.detail.clone()
            } else {
                models.card.clone()
            }),
            message_version: (s.message_version % i32::MAX as u64) as i32,
            scroll_y: reading.y,
            follow: reading.follow,
            history_note: if !s.messages_loaded {
                "会话正文按需加载"
            } else if s.history_complete {
                "当前订阅已提供完整历史"
            } else {
                "显示来源当前提供的历史范围"
            }
            .into(),
            selected,
        }
    }
    fn render(&mut self, ui: &AppWindow) {
        self.dirty = false;
        let query = self.query.to_lowercase();
        let ids: Vec<_> = self
            .order
            .iter()
            .filter(|id| {
                let s = &self.sessions[*id];
                (self.source_filter == 0 || s.source.kind.code() == self.source_filter)
                    && (query.is_empty()
                        || s.title.to_lowercase().contains(&query)
                        || s.project.to_lowercase().contains(&query))
            })
            .cloned()
            .collect();
        let sidebar = ids.iter().map(|id| self.row(id, false)).collect();
        sync_rows(&self.sidebar, sidebar);
        let detail_ids = self.order.clone();
        let details = detail_ids.iter().map(|id| self.row(id, true)).collect();
        sync_rows(&self.detail_rows, details);
        let watched: HashSet<_> = self.watched_ids().into_iter().collect();
        let matching_ids = ids.clone();
        let card_ids: Vec<_> = ids
            .into_iter()
            .filter(|id| {
                let s = &self.sessions[id];
                match self.filter {
                    1 => s.fresh && s.status == TaskStatus::Running,
                    2 => s.fresh && s.status == TaskStatus::Completed,
                    3 => !s.fresh || s.status == TaskStatus::Unknown,
                    _ => watched.contains(id),
                }
            })
            .collect();
        let chunks: Vec<Vec<_>> = card_ids
            .chunks(self.columns.max(1))
            .map(|c| c.to_vec())
            .collect();
        while self.card_models.len() > chunks.len() {
            self.card_models.pop();
            self.groups.remove(self.groups.row_count() - 1);
        }
        for (i, ids) in chunks.iter().enumerate() {
            if i >= self.card_models.len() {
                let model = Rc::new(VecModel::default());
                self.card_models.push(model.clone());
                self.groups.push(CardGroup {
                    cards: ModelRc::from(model),
                });
            }
            let rows = ids.iter().map(|id| self.row(id, false)).collect();
            sync_rows(&self.card_models[i], rows);
        }
        if self.sessions.contains_key(&self.selected) {
            let id = self.selected.clone();
            ui.set_selected(self.row(&id, true));
        }
        let matching: Vec<_> = matching_ids
            .iter()
            .filter_map(|id| self.sessions.get(id))
            .collect();
        ui.set_total(matching.len() as i32);
        ui.set_running(
            matching
                .iter()
                .filter(|s| s.fresh && s.status == TaskStatus::Running)
                .count() as i32,
        );
        ui.set_completed(
            matching
                .iter()
                .filter(|s| s.fresh && s.status == TaskStatus::Completed)
                .count() as i32,
        );
        ui.set_stale(matching.iter().filter(|s| !s.fresh).count() as i32);
        self.offline = !self.sources.values().any(|(_, c)| c.connected) && !ui.get_demo();
        ui.set_offline(self.offline);
        self.connection = if ui.get_stale() > 0 {
            format!(
                "{} 个会话等待重新同步，请查看对应来源的连接状态",
                ui.get_stale()
            )
        } else if self.offline {
            "等待原应用及观察插件连接".into()
        } else {
            "已连接的来源正在同步会话".into()
        };
        ui.set_connection(self.connection.as_str().into());
        ui.set_synced(
            matching
                .iter()
                .map(|s| s.last_sync.as_str())
                .max()
                .unwrap_or("—")
                .into(),
        );
        let mut rows = Vec::new();
        for kind in SourceKind::ALL {
            let connections: Vec<_> = self
                .sources
                .iter()
                .filter(|(id, _)| id.kind == kind)
                .map(|(_, (_, c))| c)
                .collect();
            let connected = connections.iter().any(|c| c.connected);
            let info = self.installations.get(&kind);
            let fault = self.source_faults.get(&kind);
            let note = if let Some(fault) = fault {
                fault.clone()
            } else if connected {
                format!(
                    "{} 个观察实例已连接",
                    connections.iter().filter(|c| c.connected).count()
                )
            } else {
                connections
                    .last()
                    .map(|c| c.note.clone())
                    .unwrap_or_else(|| {
                        info.map(|i| i.note.clone())
                            .unwrap_or_else(|| SourceConnection::waiting(kind).note)
                    })
            };
            let installed = info.is_some_and(|i| i.installed);
            rows.push(SourceRow {
                code: kind.code(),
                label: kind.label().into(),
                connected,
                status: if fault.is_some() {
                    "读取异常"
                } else if connected {
                    "已连接"
                } else if kind != SourceKind::Codex && !installed {
                    "未接入"
                } else {
                    "待连接"
                }
                .into(),
                note: note.into(),
                version: connections
                    .iter()
                    .find(|c| c.connected)
                    .map(|c| c.version.as_str())
                    .unwrap_or(kind.supported_version())
                    .into(),
                installed,
                path: info.map(|i| i.executable.as_str()).unwrap_or("").into(),
                busy: self.installing.contains(&kind),
            });
        }
        sync_rows(&self.source_rows, rows);
        ui.set_diagnostics(format!("Codex：连接 {} 次 · 快照 {} · 增量 {} · 重新同步 {}\n观察插件：{} 个实例\n任务控制请求：{}\n正文缓存：仅本次运行内存",self.metrics.connections,self.metrics.snapshots,self.metrics.patch_batches,self.metrics.revision_gaps,self.sources.keys().filter(|id|id.kind!=SourceKind::Codex).count(),self.metrics.controls_sent).into());
    }
}

fn demo_sessions() -> Vec<Session> {
    let titles = [
        "修复登录流程异常",
        "重构下载页交互",
        "整理部署文档",
        "优化模型调用策略",
        "新增用户反馈看板",
        "修复移动端样式问题",
        "同步产品需求文档",
        "探索多 Agent 协作",
        "验证任务恢复流程",
        "整理操作日志",
        "检查项目配置",
        "完善使用说明",
    ];
    titles.into_iter().enumerate().map(|(i,title)|{
        let mut session=Session::new(format!("demo-{i}"),"demo".into());session.id=format!("demo-{i}");session.source=SourceId{kind:SourceKind::ALL[i%3],instance:"demo".into()};session.version=session.source.kind.supported_version().into();session.messages_loaded=true;session.title=title.into();session.project=["PrivateAgent","GitHubSP","PDeploy","LLMPlayground"][i%4].into();
        session.status=if [0,1,8].contains(&i){TaskStatus::Running}else if [2,4].contains(&i){TaskStatus::Completed}else{TaskStatus::Idle};
        session.fresh=true;session.created="2026-10-15 10:12:03".into();session.last_sync="2026-10-15 10:18:21".into();session.activity=if i==0{"正在检查 OAuth 回调日志"}else if i==1{"整理组件状态与按钮反馈"}else{"已整理本轮结果"}.into();
        let content=match i {
            0=>vec![("我","message","请帮我检查登录流程中 OAuth 回调失败的问题，看看最近的日志有什么异常？"),("Agent","message","我会先查看相关的服务日志，定位 OAuth 回调失败的具体原因。"),("工具","run_command","$ tail -n 50 logs/auth.log\nOAuth callback: state validation failed\n… output truncated"),("Agent","message","从日志看，回调时 state 参数校验失败，可能是前端传递的 state 与服务端不一致。我继续检查 OAuth 配置和相关代码。"),("工具","search_code","在项目中搜索 \"oauth state\"\n找到 12 个文件，查看匹配内容"),("Agent","message","在回调处理代码中发现了 session 状态保存的时序问题：\n```ts\nconst storedState = req.session?.oauthState;\nconst incomingState = req.query.state;\nif (storedState !== incomingState) {\n  return reportStateMismatch();\n}\n```\n下一步核对 session 写入与回调到达的顺序。")],
            1=>vec![("我","message","下载页的按钮在移动端有时点击无响应，帮我检查一下组件的交互逻辑。"),("Agent","message","我先查看下载页的组件代码，分析按钮的点击事件和状态管理。"),("工具","read_file","src/pages/download/DownloadButton.tsx\nexport function DownloadButton() {\n  const [loading, setLoading] = useState(false);\n  return <Button loading={loading} />;\n}"),("我","message","看起来是防抖的问题，帮我加一个节流处理。"),("Agent","message","我来修改代码，添加节流，避免短时间内重复触发点击事件。"),("工具","apply_patch","修改了 1 个文件\nsrc/pages/download/DownloadButton.tsx\n+12  -3")],
            _=>vec![("我","message","请根据最近的部署记录，整理一份新的部署文档，包含环境准备、启动步骤和常见问题。"),("Agent","message","我将先收集相关的部署配置和历史记录，然后整理成结构化文档。"),("工具","read_file","docs/deploy/history.md\n# 部署记录\n## 环境准备\n检查运行时和服务配置。"),("Agent","message","已完成文档整理，包含环境准备、启动步骤和常见问题说明。"),("工具","run_command","$ pnpm build:docs\nBuild completed successfully.")],
        };
        session.messages=content.into_iter().enumerate().map(|(j,(author,kind,text))|Message{id:format!("{i}-{j}"),author:author.into(),kind:kind.into(),text:text.into(),status:if i==0&&kind=="search_code"{"inProgress"}else{"completed"}.into(),time:format!("10:{:02}",13+j)}).collect();session
    }).collect()
}

pub fn run(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let verify_ui = args.iter().any(|a| a == "--verify-ui");
    let demo = verify_ui || args.iter().any(|a| a == "--demo" || a == "--capture-demo");
    let capture = args.iter().any(|a| a == "--capture-demo");
    let live_smoke = args.iter().any(|a| a == "--smoke-live");
    // 每次验证先写入未完成标记，提前关闭窗口时不能误读上次通过的报告。
    for (enabled, path) in [
        (verify_ui, "artifacts/native-ui/interaction-result.json"),
        (live_smoke, "artifacts/native-live/result.json"),
    ] {
        if enabled {
            let path = Path::new(path);
            std::fs::create_dir_all(path.parent().unwrap())?;
            std::fs::write(path, b"{\"passed\":false,\"status\":\"running\"}")?;
        }
    }
    let ui = AppWindow::new()?;
    ui.set_demo(demo);
    ui.set_can_open(!demo && crate::native::codex_available());
    let state = Rc::new(RefCell::new(State::default()));
    ui.set_sessions(ModelRc::from(state.borrow().sidebar.clone()));
    ui.set_sources(ModelRc::from(state.borrow().source_rows.clone()));
    ui.set_detail_sessions(ModelRc::from(state.borrow().detail_rows.clone()));
    ui.set_groups(ModelRc::from(state.borrow().groups.clone()));
    let monitor = if demo {
        None
    } else {
        Some(monitor::Monitor::start())
    };
    let observer = if demo {
        None
    } else {
        Some(observer::Observer::start())
    };
    let (installation_tx, installation_rx) =
        std::sync::mpsc::channel::<(SourceKind, Result<installation::Info, String>)>();
    if !demo {
        for kind in [SourceKind::Opencode, SourceKind::Deepseek] {
            state
                .borrow_mut()
                .installations
                .insert(kind, installation::inspect(kind, ""));
        }
    }
    let final_metrics = monitor.as_ref().map(|m| m.final_metrics.clone());
    if demo {
        let mut s = state.borrow_mut();
        for session in demo_sessions() {
            s.ingest(session);
        }
        s.watched = vec!["demo-0".into(), "demo-1".into(), "demo-2".into()];
        s.offline = false;
        s.connection = "演示模式 · 已连接".into();
        for kind in SourceKind::ALL {
            s.sources.insert(
                SourceId {
                    kind,
                    instance: "demo".into(),
                },
                (
                    "demo".into(),
                    SourceConnection {
                        connected: true,
                        note: "演示连接".into(),
                        version: kind.supported_version().into(),
                    },
                ),
            );
        }
        for id in s.order.clone() {
            for detail in [false, true] {
                s.readings.insert(
                    (id.clone(), detail),
                    Reading {
                        y: 0.,
                        follow: false,
                    },
                );
            }
        }
    }
    {
        let weak = ui.as_weak();
        let navigation_state = state.clone();
        ui.on_open_codex(move |id| {
            let native_id = navigation_state
                .borrow()
                .sessions
                .get(id.as_str())
                .filter(|s| s.source.kind == SourceKind::Codex)
                .map(|s| s.native_id.clone());
            if let Some(ui) = weak.upgrade()
                && let Some(native_id) = native_id
                && let Err(error) = crate::native::open_codex(&native_id)
            {
                ui.set_toast(error.into());
            }
        });
        let source_state = state.clone();
        ui.on_source_filter_changed(move |code| {
            let mut s = source_state.borrow_mut();
            s.source_filter = code;
            s.dirty = true;
        });
        let path_state = state.clone();
        ui.on_source_path_changed(move |code, path| {
            if let Some(kind) = SourceKind::from_code(code) {
                if let Some(info) = path_state.borrow_mut().installations.get_mut(&kind) {
                    info.executable = path.to_string();
                }
            }
        });
        let action_state = state.clone();
        let weak = ui.as_weak();
        let tx = installation_tx.clone();
        ui.on_manage_source(move |code, action| {
            let Some(kind) = SourceKind::from_code(code).filter(|k| *k != SourceKind::Codex) else {
                return;
            };
            if demo {
                if let Some(ui) = weak.upgrade() {
                    ui.set_toast("演示模式不修改插件配置".into());
                }
                return;
            }
            let mut s = action_state.borrow_mut();
            if !s.installing.insert(kind) {
                return;
            }
            let path = s
                .installations
                .get(&kind)
                .map(|i| i.executable.clone())
                .unwrap_or_default();
            s.dirty = true;
            let tx = tx.clone();
            std::thread::spawn(move || {
                let result = if action == 0 {
                    Ok(installation::inspect(kind, &path))
                } else {
                    installation::change(kind, &path, action == 1).map_err(str::to_owned)
                };
                tx.send((kind, result)).ok();
            });
        });
        let close_state = state.clone();
        let weak = ui.as_weak();
        ui.window().on_close_requested(move || {
            if close_state.borrow().installing.is_empty() {
                slint::CloseRequestResponse::HideWindow
            } else {
                if let Some(ui) = weak.upgrade() {
                    ui.set_toast("正在保存接入配置，请稍候".into());
                }
                slint::CloseRequestResponse::KeepWindowShown
            }
        });
        let s = state.clone();
        ui.on_search(move |query| {
            let mut s = s.borrow_mut();
            s.query = query.to_string();
            s.dirty = true;
        });
        let s = state.clone();
        ui.on_filter_changed(move |value| {
            let mut s = s.borrow_mut();
            s.filter = value;
            s.dirty = true;
        });
        let s = state.clone();
        ui.on_columns_changed(move |value| {
            if let Ok(mut s) = s.try_borrow_mut() {
                s.columns = (value as usize).max(1);
                s.dirty = true;
            }
        });
        let s = state.clone();
        let weak = ui.as_weak();
        ui.on_open_detail(move |id| {
            let mut s = s.borrow_mut();
            s.selected = id.to_string();
            s.dirty = true;
            if let Some(ui) = weak.upgrade() {
                ui.set_view(1);
            }
        });
        let s = state.clone();
        ui.on_toggle_watch(move |id| {
            s.borrow_mut().toggle_watch(id.as_str());
        });
        let s = state.clone();
        ui.on_scroll_state(move |id, y, follow, detail| {
            if let Ok(mut s) = s.try_borrow_mut() {
                s.readings
                    .insert((id.to_string(), detail), Reading { y, follow });
            }
        });
        let s = state.clone();
        ui.on_toggle_tool(move |id, message, detail| {
            let mut s = s.borrow_mut();
            let key = (id.to_string(), message.to_string(), detail);
            if !s.expanded.remove(&key) {
                s.expanded.insert(key);
            }
            s.dirty = true;
        });
        let s = state.clone();
        let weak = ui.as_weak();
        ui.on_copy_message(move |id, message| {
            let text = s
                .borrow()
                .sessions
                .get(id.as_str())
                .and_then(|session| session.messages.iter().find(|m| m.id == message.as_str()))
                .map(|m| m.text.clone());
            if let (Some(text), Some(ui)) = (text, weak.upgrade()) {
                ui.invoke_copy_text(text.into());
                ui.set_toast("已复制消息原文".into());
                s.borrow_mut().toast_until = Some(Instant::now() + Duration::from_secs(2));
            }
        });
        if let Some(monitor) = &monitor {
            let tx = monitor.commands.clone();
            ui.on_retry(move |code| {
                if code == 0 || code == 1 {
                    tx.send(Command::Retry).ok();
                }
            });
            let tx = monitor.commands.clone();
            ui.on_rescan(move |code| {
                if code == 0 || code == 1 {
                    tx.send(Command::Rescan).ok();
                }
            });
        } else {
            let s = state.clone();
            ui.on_retry(move |_code| {
                let mut s = s.borrow_mut();
                for session in s.sessions.values_mut() {
                    session.fresh = true;
                }
                s.offline = false;
                s.connection = "演示模式 · 已重新同步".into();
                s.dirty = true;
            });
            let weak = ui.as_weak();
            ui.on_rescan(move |_code| {
                if let Some(ui) = weak.upgrade() {
                    ui.set_toast("演示数据 · 共 12 个会话".into());
                }
            });
        }
    }
    if let Some(observer) = &observer {
        let bridge = observer.commands.clone();
        let codex = monitor.as_ref().map(|m| m.commands.clone());
        ui.on_retry(move |code| {
            if (code == 0 || code == 1)
                && let Some(tx) = &codex
            {
                tx.send(Command::Retry).ok();
            }
            for kind in [SourceKind::Opencode, SourceKind::Deepseek] {
                if code == 0 || kind.code() == code {
                    bridge.send(observer::Command::Retry(kind)).ok();
                }
            }
        });
        let bridge = observer.commands.clone();
        let codex = monitor.as_ref().map(|m| m.commands.clone());
        ui.on_rescan(move |code| {
            if (code == 0 || code == 1)
                && let Some(tx) = &codex
            {
                tx.send(Command::Rescan).ok();
            }
            for kind in [SourceKind::Opencode, SourceKind::Deepseek] {
                if code == 0 || kind.code() == code {
                    bridge.send(observer::Command::Rescan(kind)).ok();
                }
            }
        });
    }
    let timer = Timer::default();
    let weak = ui.as_weak();
    let shared = state.clone();
    let start = Instant::now();
    let phase = Rc::new(RefCell::new(0u8));
    let step = phase.clone();
    let command_tx = monitor.as_ref().map(|m| m.commands.clone());
    // 所有 Slint 模型仅由 UI 线程更新；后台事件批量合并，避免流式 token 引发整页重建。
    timer.start(TimerMode::Repeated,Duration::from_millis(100),move||{
        let Some(ui)=weak.upgrade()else{return;};let mut s=shared.borrow_mut();
        // 从实际原生客户区提供宽度，避免响应式布局与窗口的隐式首选尺寸形成绑定环。
        ui.set_available_width(ui.window().size().width as f32/ui.window().scale_factor()-260.);
        let columns=ui.get_columns().max(1) as usize;if s.columns!=columns{s.columns=columns;s.dirty=true;}
        if let Some(monitor)=&monitor {
            let mut latest=HashMap::new();
            for event in monitor.events.try_iter(){match event {Event::Session(session)=>{latest.insert(session.id.clone(),*session);},Event::Connection(kind,note)=>{let connected=["connected","empty"].contains(&kind);let id=SourceId::codex();if s.sources.get(&id).is_none_or(|(_,c)|c.connected!=connected||c.note!=note){s.sources.insert(id,("codex".into(),SourceConnection{connected,note,version:SourceKind::Codex.supported_version().into()}));s.dirty=true;}},Event::Metrics(metrics)=>s.metrics=metrics}}
            for session in latest.into_values(){s.ingest(session);}
        }
        if live_smoke {
            for kind in [SourceKind::Opencode,SourceKind::Deepseek] {
                let sample:Vec<_>=s.order.iter().filter(|id|s.sessions[*id].source.kind==kind).take(2).cloned().collect();
                for id in sample {if !s.watched.contains(&id){s.watched.push(id);s.dirty=true;}}
            }
        }
        if let Some(observer)=&observer {
            for event in observer.events.try_iter(){s.observer_event(event);}
            let wanted=s.wanted(ui.get_view()==1);if wanted!=s.interests{observer.commands.send(observer::Command::Interest(wanted.iter().cloned().collect())).ok();s.interests=wanted;}
        }
        for (kind,result) in installation_rx.try_iter(){s.installing.remove(&kind);match result{Ok(info)=>{ui.set_toast(info.note.as_str().into());s.installations.insert(kind,info);},Err(note)=>{ui.set_toast(note.as_str().into());if let Some(info)=s.installations.get_mut(&kind){info.note=note;}}}s.toast_until=Some(Instant::now()+Duration::from_secs(8));s.dirty=true;}
        if s.toast_until.is_some_and(|t|Instant::now()>=t){s.toast_until=None;ui.set_toast("".into());}
        if s.dirty{s.render(&ui);}
        if capture {
            let elapsed=start.elapsed().as_millis();let mut phase=step.borrow_mut();
            let result=match *phase {
                0 if elapsed>700=>{std::fs::create_dir_all("artifacts/native-ui").ok();let r=gate::save_snapshot(ui.window(),Path::new("artifacts/native-ui/01-overview.png"));*phase=1;s.selected="demo-0".into();s.dirty=true;ui.set_view(1);r},
                1 if elapsed>1500=>{let r=gate::save_snapshot(ui.window(),Path::new("artifacts/native-ui/02-detail.png"));*phase=2;ui.set_view(0);s.offline=true;s.connection="监控连接已断开，正在重新连接".into();if let Some((_,c))=s.sources.get_mut(&SourceId{kind:SourceKind::Deepseek,instance:"demo".into()}){c.connected=false;c.note="演示来源已断开".into();}for session in s.sessions.values_mut().filter(|s|s.source.kind==SourceKind::Deepseek){session.fresh=false;}s.dirty=true;r},
                2 if elapsed>2300=>{let r=gate::save_snapshot(ui.window(),Path::new("artifacts/native-ui/03-reconnect.png"));*phase=3;ui.window().set_size(slint::LogicalSize::new(980.,700.));r},
                3 if elapsed>3200=>{let r=gate::save_snapshot(ui.window(),Path::new("artifacts/native-ui/04-compact.png"));*phase=4;ui.window().set_size(slint::LogicalSize::new(860.,600.));ui.set_view(2);r},
                4 if elapsed>4000=>{let r=gate::save_snapshot(ui.window(),Path::new("artifacts/native-ui/05-setup-minimum.png"));*phase=5;slint::quit_event_loop().ok();r},_=>Ok(()),
            };if let Err(error)=result{eprintln!("原生截图失败：{error}");slint::quit_event_loop().ok();}
        }
        if live_smoke {
            let elapsed=start.elapsed().as_secs();let mut phase=step.borrow_mut();
            if *phase==0&&elapsed>=8{if let Some(tx)=&command_tx{tx.send(Command::Retry).ok();}if let Some(observer)=&observer{for kind in [SourceKind::Opencode,SourceKind::Deepseek]{observer.commands.send(observer::Command::Retry(kind)).ok();}}*phase=1;}
            if elapsed>=24{
                std::fs::create_dir_all("artifacts/native-live").ok();
                let result=serde_json::json!({"metrics":s.metrics,"uiSessions":s.sessions.len(),"uiMessages":s.sessions.values().map(|s|s.messages.len()).sum::<usize>(),"freshSessions":s.sessions.values().filter(|s|s.fresh).count(),"codexNavigationAvailable":ui.get_can_open(),"nativeWindow":true,"rawContentSaved":false,"sources":SourceKind::ALL.map(|kind|serde_json::json!({"kind":kind,"sessions":s.sessions.values().filter(|x|x.source.kind==kind).count(),"fresh":s.sessions.values().filter(|x|x.source.kind==kind&&x.fresh).count(),"freshSnapshots":s.sessions.values().filter(|x|x.source.kind==kind&&x.fresh&&x.messages_loaded).count(),"messages":s.sessions.values().filter(|x|x.source.kind==kind).map(|x|x.messages.len()).sum::<usize>()})),"checkedAt":now()});
                std::fs::write("artifacts/native-live/result.json",serde_json::to_vec_pretty(&result).unwrap()).expect("无法保存接入验证结果");println!("{result}");slint::quit_event_loop().ok();
            }
        }
    });
    let ui_verification = verify_ui.then(|| ui_check::start(&ui, state.clone()));
    ui.run()?;
    drop(ui_verification);
    timer.stop();
    drop(timer);
    if live_smoke
        && let Some(final_metrics) = final_metrics
        && let Some(metrics) = final_metrics.lock().unwrap().as_ref()
    {
        let path = Path::new("artifacts/native-live/result.json");
        let mut report: serde_json::Value = serde_json::from_slice(&std::fs::read(path)?)?;
        if report["nativeWindow"] != true {
            return Err("真实会话验证提前结束".into());
        }
        report["shutdownMetrics"] = serde_json::to_value(metrics)?;
        std::fs::write(path, serde_json::to_vec_pretty(&report)?)?;
    }
    if verify_ui {
        let report: serde_json::Value = serde_json::from_slice(&std::fs::read(
            "artifacts/native-ui/interaction-result.json",
        )?)?;
        if report["passed"] != true {
            return Err("原生界面交互验证失败".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reconnect_isolates_sources_and_rejects_old_connection_events() {
        let mut state = State::default();
        let a = SourceId {
            kind: SourceKind::Opencode,
            instance: "a".into(),
        };
        let b = SourceId {
            kind: SourceKind::Deepseek,
            instance: "b".into(),
        };
        let connected = SourceConnection {
            connected: true,
            note: String::new(),
            version: String::new(),
        };
        state.observer_event(observer::Event::Connection(
            a.clone(),
            "old".into(),
            connected.clone(),
        ));
        state.observer_event(observer::Event::Connection(
            b.clone(),
            "other".into(),
            connected.clone(),
        ));
        for (source, epoch) in [(&a, "old"), (&b, "other")] {
            let mut session = Session::from_source("same".into(), source.clone());
            session.fresh = true;
            session.messages_loaded = true;
            session.title = "原内容".into();
            state.observer_event(observer::Event::Session(epoch.into(), Box::new(session)));
        }
        assert_eq!(state.sessions.len(), 2);
        state.observer_event(observer::Event::Connection(
            a.clone(),
            "new".into(),
            connected,
        ));
        let mut late = Session::from_source("same".into(), a.clone());
        late.fresh = true;
        late.title = "迟到的旧内容".into();
        state.observer_event(observer::Event::Session("old".into(), Box::new(late)));
        assert!(
            !state.sessions[&SessionKey {
                source: a.clone(),
                native_id: "same".into()
            }
            .encoded()]
                .fresh
        );
        assert!(
            state.sessions[&SessionKey {
                source: b,
                native_id: "same".into()
            }
            .encoded()]
                .fresh
        );
        assert!(state.sessions.values().all(|s| s.title == "原内容"));
        let mut catalog = Session::from_source("same".into(), a.clone());
        catalog.fresh = true;
        catalog.title = "更新后的标题".into();
        catalog.status = TaskStatus::Running;
        state.observer_event(observer::Event::Catalog(
            a.clone(),
            "new".into(),
            vec![catalog],
        ));
        let session = &state.sessions[&SessionKey {
            source: a,
            native_id: "same".into(),
        }
        .encoded()];
        assert_eq!(session.title, "更新后的标题");
        assert!(!session.fresh);
    }
    #[test]
    fn markdown_preserves_chinese_code_and_headings() {
        let result = blocks(
            "## 中文标题\n\n正文 **强调**\n\n```ts\nconst state = '中文';\n```",
            false,
        );
        assert_eq!(result[0].heading, 1);
        assert!(result.iter().any(|b| b.code && b.text.contains("中文")));
        assert!(result.iter().any(|b| b.text.contains("正文 强调")));
    }
    #[test]
    fn selection_survives_updates_and_completion() {
        let mut s = State::default();
        for session in demo_sessions() {
            s.ingest(session);
        }
        s.toggle_watch("demo-0");
        let mut changed = s.sessions["demo-0"].clone();
        changed.status = TaskStatus::Running;
        s.ingest(changed);
        assert!(!s.watched.iter().any(|id| id == "demo-0"));
        let mut completed = s.sessions["demo-1"].clone();
        completed.status = TaskStatus::Completed;
        s.ingest(completed);
        assert!(s.watched.iter().any(|id| id == "demo-1"));
    }
    #[test]
    fn idle_defaults_respect_manual_removal() {
        let mut state = State::default();
        for mut session in demo_sessions() {
            session.status = TaskStatus::Completed;
            state.ingest(session);
        }
        let initial = state.watched_ids();
        assert_eq!(initial.len(), 3);
        for id in initial {
            state.toggle_watch(&id);
        }
        assert!(state.watched_ids().is_empty());
    }
}
