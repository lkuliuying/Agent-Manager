use crate::{
    source::{SessionKey, SourceConnection, SourceId, SourceKind},
    state::{Message, Session, TaskStatus, now, timestamp},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    ffi::c_void,
    path::PathBuf,
    sync::{Arc, Mutex, mpsc},
    thread,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::windows::named_pipe::{NamedPipeServer, ServerOptions},
    sync::{broadcast, mpsc as async_mpsc},
};

pub const PROTOCOL: u64 = 1;
const MAX_FRAME: usize = 32 * 1024 * 1024;
#[derive(Clone)]
pub enum Command {
    Retry(SourceKind),
    Rescan(SourceKind),
    Interest(Vec<SessionKey>),
    Stop,
}
pub enum Event {
    Connection(SourceId, String, SourceConnection),
    Catalog(SourceId, String, Vec<Session>),
    Session(String, Box<Session>),
    Error(SourceKind, String),
    SessionError(SourceId, String, String),
}
pub struct Observer {
    pub commands: async_mpsc::UnboundedSender<Command>,
    pub events: mpsc::Receiver<Event>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Observer {
    pub fn start() -> Self {
        let (commands, receiver) = async_mpsc::unbounded_channel();
        let (sender, events) = mpsc::channel();
        let worker = thread::spawn(move || {
            match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => {
                    if let Err(code) = runtime.block_on(serve(receiver, sender.clone())) {
                        for kind in [SourceKind::Opencode, SourceKind::Deepseek] {
                            sender.send(Event::Error(kind, code.into())).ok();
                        }
                    }
                }
                Err(_) => {
                    sender
                        .send(Event::Error(
                            SourceKind::Opencode,
                            "观察运行时初始化失败".into(),
                        ))
                        .ok();
                }
            }
        });
        Self {
            commands,
            events,
            worker: Some(worker),
        }
    }
}
impl Drop for Observer {
    fn drop(&mut self) {
        self.commands.send(Command::Stop).ok();
        if let Some(worker) = self.worker.take() {
            worker.join().ok();
        }
    }
}
pub fn home() -> PathBuf {
    // AppData 在打包应用的子进程中可能被重定向，跨应用观察必须使用共同可见的目录。
    PathBuf::from(
        std::env::var_os("USERPROFILE").unwrap_or_else(|| std::env::temp_dir().into_os_string()),
    )
    .join(".agmanage")
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}
fn current_sid() -> Result<String, &'static str> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, LocalFree},
        Security::{
            Authorization::ConvertSidToStringSidW, GetTokenInformation, TOKEN_QUERY, TOKEN_USER,
            TokenUser,
        },
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };
    unsafe {
        let mut token = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return Err("无法读取当前用户身份");
        }
        let mut size = 0;
        GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut size);
        let mut data = vec![0usize; (size as usize).div_ceil(std::mem::size_of::<usize>())];
        let ok = GetTokenInformation(token, TokenUser, data.as_mut_ptr().cast(), size, &mut size);
        CloseHandle(token);
        if ok == 0 {
            return Err("无法读取当前用户身份");
        }
        let mut sid = std::ptr::null_mut();
        if ConvertSidToStringSidW((*(data.as_ptr() as *const TOKEN_USER)).User.Sid, &mut sid) == 0 {
            return Err("无法转换当前用户身份");
        }
        let mut len = 0;
        while *sid.add(len) != 0 {
            len += 1;
        }
        let value = String::from_utf16_lossy(std::slice::from_raw_parts(sid, len));
        LocalFree(sid.cast());
        Ok(value)
    }
}
fn create_pipe(path: &str, sid: &str, first: bool) -> Result<NamedPipeServer, &'static str> {
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::{
            Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW,
            SECURITY_ATTRIBUTES,
        },
    };
    unsafe {
        let mut descriptor = std::ptr::null_mut();
        let sddl = wide(&format!("D:P(A;;GA;;;{sid})"));
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            1,
            &mut descriptor,
            std::ptr::null_mut(),
        ) == 0
        {
            return Err("无法设置观察通道权限");
        }
        let mut attributes = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor,
            bInheritHandle: 0,
        };
        let result = ServerOptions::new()
            .first_pipe_instance(first)
            .reject_remote_clients(true)
            .create_with_security_attributes_raw(
                path,
                (&mut attributes as *mut SECURITY_ATTRIBUTES).cast::<c_void>(),
            );
        LocalFree(descriptor);
        result.map_err(|_| "观察通道不可用；请检查是否已打开另一份管理器")
    }
}
async fn send(pipe: &mut NamedPipeServer, packet: Value) -> Result<(), &'static str> {
    if !readonly_request(&packet) {
        return Err("readonly_violation");
    }
    let bytes = serde_json::to_vec(&packet).map_err(|_| "encode_failed")?;
    pipe.write_all(&(bytes.len() as u32).to_le_bytes())
        .await
        .map_err(|_| "pipe_write_failed")?;
    pipe.write_all(&bytes)
        .await
        .map_err(|_| "pipe_write_failed")
}
pub fn readonly_request(packet: &Value) -> bool {
    packet["protocol"] == PROTOCOL
        && match packet["type"].as_str() {
            Some("catalog") => true,
            Some("snapshot" | "subscribe" | "unsubscribe") => {
                packet["id"].as_str().is_some_and(|id| !id.is_empty())
            }
            _ => false,
        }
}
async fn read(pipe: &mut NamedPipeServer) -> Result<Value, &'static str> {
    let size = pipe.read_u32_le().await.map_err(|_| "pipe_closed")? as usize;
    if size == 0 || size > MAX_FRAME {
        return Err("frame_size");
    }
    let mut bytes = vec![0; size];
    pipe.read_exact(&mut bytes)
        .await
        .map_err(|_| "pipe_closed")?;
    serde_json::from_slice(&bytes).map_err(|_| "frame_json")
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireMessage {
    id: String,
    author: String,
    kind: String,
    text: String,
    #[serde(default)]
    status: String,
    time: i64,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireSession {
    id: String,
    title: String,
    project: String,
    created: i64,
    status: String,
    #[serde(default)]
    history_complete: bool,
    #[serde(default)]
    activity: String,
    #[serde(default)]
    messages: Vec<WireMessage>,
}
fn decode_session(
    value: Value,
    source: &SourceId,
    seq: u64,
    snapshot: bool,
) -> Result<Session, &'static str> {
    let data: WireSession = serde_json::from_value(value).map_err(|_| "session_schema")?;
    if data.id.is_empty() || data.id.len() > 2048 {
        return Err("session_identity");
    }
    let mut result = Session::from_source(data.id, source.clone());
    result.title = data.title;
    result.project = data
        .project
        .trim_end_matches(['\\', '/'])
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or("")
        .into();
    result.created = timestamp(data.created, false);
    result.last_sync = now();
    result.fresh = true;
    result.status = match data.status.as_str() {
        "running" => TaskStatus::Running,
        "completed" => TaskStatus::Completed,
        "failed" => TaskStatus::Failed,
        "interrupted" => TaskStatus::Interrupted,
        "idle" => TaskStatus::Idle,
        "unknown" => TaskStatus::Unknown,
        _ => return Err("status_schema"),
    };
    result.activity = if !data.activity.is_empty() {
        data.activity
    } else if snapshot {
        result.status.label().into()
    } else {
        "已发现会话，正文按需同步".into()
    };
    result.history_complete = snapshot && data.history_complete;
    result.messages_loaded = snapshot;
    result.message_version = seq;
    let mut ids = HashSet::new();
    for m in data.messages {
        if m.id.is_empty()
            || !ids.insert(m.id.clone())
            || !["我", "Agent", "工具"].contains(&m.author.as_str())
        {
            return Err("message_schema");
        }
        result.messages.push(Message {
            id: m.id,
            author: m.author,
            kind: m.kind,
            text: m.text,
            status: m.status,
            time: timestamp(m.time, true),
        });
    }
    Ok(result)
}

async fn client(
    mut pipe: NamedPipeServer,
    events: mpsc::Sender<Event>,
    mut commands: broadcast::Receiver<Command>,
    interests: Arc<Mutex<HashSet<SessionKey>>>,
) {
    let Ok(Ok(hello)) = tokio::time::timeout(Duration::from_secs(10), read(&mut pipe)).await else {
        return;
    };
    let Ok(kind) = serde_json::from_value::<SourceKind>(hello["kind"].clone()) else {
        return;
    };
    if kind == SourceKind::Codex {
        return;
    }
    let version = hello["version"].as_str().unwrap_or("unknown");
    if hello["type"] != "hello"
        || hello["protocol"] != PROTOCOL
        || version != kind.supported_version()
        || hello["seq"] != 1
    {
        events
            .send(Event::Error(
                kind,
                format!(
                    "插件协议或宿主版本不兼容；当前支持 {}",
                    kind.supported_version()
                ),
            ))
            .ok();
        return;
    }
    let Some(instance) = hello["instance"]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 128)
    else {
        return;
    };
    let source = SourceId {
        kind,
        instance: instance.into(),
    };
    let epoch = uuid::Uuid::new_v4().to_string();
    events
        .send(Event::Connection(
            source.clone(),
            epoch.clone(),
            SourceConnection {
                connected: true,
                note: "观察插件已连接".into(),
                version: version.into(),
            },
        ))
        .ok();
    let result = connected(
        &mut pipe,
        &source,
        &epoch,
        &events,
        &mut commands,
        interests,
    )
    .await;
    events
        .send(Event::Connection(
            source,
            epoch,
            SourceConnection {
                connected: false,
                note: format!("连接已断开 · {}", result.err().unwrap_or("已取消订阅")),
                version: version.into(),
            },
        ))
        .ok();
}
async fn connected(
    pipe: &mut NamedPipeServer,
    source: &SourceId,
    epoch: &str,
    events: &mpsc::Sender<Event>,
    commands: &mut broadcast::Receiver<Command>,
    interests: Arc<Mutex<HashSet<SessionKey>>>,
) -> Result<(), &'static str> {
    send(pipe, json!({"protocol":PROTOCOL,"type":"catalog"})).await?;
    let mut subscribed = HashSet::<String>::new();
    let mut last_seq = 1;
    let mut timer = tokio::time::interval(Duration::from_millis(300));
    let mut heartbeat = std::time::Instant::now();
    // read_exact 不能在 select! 中反复取消；分块缓冲保留跨帧读取状态。
    let (mut reader, mut writer) = tokio::io::split(pipe);
    let mut buffer = Vec::new();
    let mut chunk = vec![0u8; 65536];
    loop {
        tokio::select! {
            command = commands.recv() => match command {
                Ok(Command::Stop) | Err(_) => return Ok(()),
                Ok(Command::Retry(kind)) if kind == source.kind => return Ok(()),
                Ok(Command::Rescan(kind)) if kind == source.kind => {
                    write_request(&mut writer, json!({"protocol":PROTOCOL,"type":"catalog"})).await?;
                    for id in &subscribed { write_request(&mut writer,json!({"protocol":PROTOCOL,"type":"snapshot","id":id})).await?; }
                },
                _ => {},
            },
            received = reader.read(&mut chunk) => {
                let count = received.map_err(|_| "pipe_read_failed")?;
                if count == 0 { return Err("pipe_closed"); }
                buffer.extend_from_slice(&chunk[..count]);
                while buffer.len() >= 4 {
                    let size = u32::from_le_bytes(buffer[..4].try_into().unwrap()) as usize;
                    if size == 0 || size > MAX_FRAME { return Err("frame_size"); }
                    if buffer.len() < size + 4 { break; }
                    let packet: Value = serde_json::from_slice(&buffer[4..size+4]).map_err(|_| "frame_json")?;
                    buffer.drain(..size+4);
                    let seq = packet["seq"].as_u64().ok_or("sequence_missing")?;
                    if seq <= last_seq { continue; }
                    if seq != last_seq + 1 { return Err("sequence_gap"); }
                    last_seq = seq; heartbeat = std::time::Instant::now();
                    match packet["type"].as_str() {
                        Some("heartbeat") => {},
                        Some("catalog") => {
                            let values = packet["sessions"].as_array().ok_or("catalog_schema")?;
                            let sessions = values.iter().map(|v| decode_session(v.clone(),source,seq,false)).collect::<Result<Vec<_>,_>>()?;
                            events.send(Event::Catalog(source.clone(),epoch.into(),sessions)).ok();
                        },
                        Some("snapshot") => {
                            let session = decode_session(packet["session"].clone(),source,seq,true)?;
                            if subscribed.contains(&session.native_id) { events.send(Event::Session(epoch.into(),Box::new(session))).ok(); }
                        },
                        Some("error") => {
                            if let Some(id) = packet["id"].as_str() { events.send(Event::SessionError(source.clone(),epoch.into(),id.into())).ok(); }
                            else { events.send(Event::Error(source.kind,"插件读取会话目录失败；请重新扫描或检查宿主版本".into())).ok(); }
                        },
                        _ => return Err("event_unsupported"),
                    }
                }
            },
            _ = timer.tick() => {
                if heartbeat.elapsed() > Duration::from_secs(30) { return Err("heartbeat_timeout"); }
                let wanted: HashSet<String> = interests.lock().unwrap().iter().filter(|key| key.source == *source).map(|key| key.native_id.clone()).collect();
                for id in wanted.difference(&subscribed) { write_request(&mut writer,json!({"protocol":PROTOCOL,"type":"subscribe","id":id})).await?; }
                for id in subscribed.difference(&wanted) { write_request(&mut writer,json!({"protocol":PROTOCOL,"type":"unsubscribe","id":id})).await?; }
                subscribed = wanted;
            }
        }
    }
}
async fn write_request<W: tokio::io::AsyncWrite + Unpin>(
    writer: &mut W,
    packet: Value,
) -> Result<(), &'static str> {
    if !readonly_request(&packet) {
        return Err("readonly_violation");
    }
    let bytes = serde_json::to_vec(&packet).map_err(|_| "encode_failed")?;
    writer
        .write_all(&(bytes.len() as u32).to_le_bytes())
        .await
        .map_err(|_| "pipe_write_failed")?;
    writer
        .write_all(&bytes)
        .await
        .map_err(|_| "pipe_write_failed")
}
async fn serve(
    mut commands: async_mpsc::UnboundedReceiver<Command>,
    events: mpsc::Sender<Event>,
) -> Result<(), &'static str> {
    let sid = current_sid()?;
    let pipe_path = format!(r"\\.\pipe\agmanage-observer-{}", sid.to_lowercase());
    let mut server = create_pipe(&pipe_path, &sid, true)?;
    let descriptor = home().join("observer.json");
    std::fs::create_dir_all(home()).map_err(|_| "无法创建本地观察目录")?;
    std::fs::write(
        &descriptor,
        serde_json::to_vec(&json!({"protocol":PROTOCOL,"pipe":pipe_path})).unwrap(),
    )
    .map_err(|_| "无法写入观察通道地址")?;
    let interests = Arc::new(Mutex::new(HashSet::new()));
    let (control, _) = broadcast::channel(32);
    let mut clients = tokio::task::JoinSet::new();
    loop {
        tokio::select! {
            command = commands.recv() => match command {
                Some(Command::Interest(keys)) => *interests.lock().unwrap() = keys.into_iter().collect(),
                Some(Command::Stop) | None => break,
                Some(command) => { control.send(command).ok(); },
            },
            result = server.connect() => {
                result.map_err(|_| "观察通道连接失败")?;
                let next = create_pipe(&pipe_path,&sid,false)?;
                let connected = std::mem::replace(&mut server,next);
                clients.spawn(client(connected,events.clone(),control.subscribe(),interests.clone()));
            },
            _ = clients.join_next(), if !clients.is_empty() => {},
        }
    }
    control.send(Command::Stop).ok();
    if tokio::time::timeout(Duration::from_millis(800), async {
        while clients.join_next().await.is_some() {}
    })
    .await
    .is_err()
    {
        clients.abort_all();
    }
    // 只删除自己仍拥有的入口，不影响原应用或其他配置。
    if std::fs::read_to_string(&descriptor)
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .is_some_and(|v| v["pipe"] == pipe_path)
    {
        std::fs::remove_file(descriptor).ok();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_identity_and_readonly_boundary() {
        let a = SourceId {
            kind: SourceKind::Opencode,
            instance: "one".into(),
        };
        let b = SourceId {
            kind: SourceKind::Deepseek,
            instance: "one".into(),
        };
        assert_ne!(
            SessionKey {
                source: a.clone(),
                native_id: "same".into()
            }
            .encoded(),
            SessionKey {
                source: b,
                native_id: "same".into()
            }
            .encoded()
        );
        for action in ["send-message", "resume", "stop", "approval", "execute"] {
            assert!(!readonly_request(
                &json!({"protocol":1,"type":action,"id":"s"})
            ));
        }
        assert!(readonly_request(
            &json!({"protocol":1,"type":"snapshot","id":"s"})
        ));
        let bad = json!({"id":"s","title":"演示","project":"P","created":0,"status":"unknown","messages":[{"id":"a","author":"Agent","kind":"message","text":"A","time":0},{"id":"a","author":"Agent","kind":"message","text":"B","time":0}]});
        assert!(decode_session(bad, &a, 2, true).is_err());
    }
}
