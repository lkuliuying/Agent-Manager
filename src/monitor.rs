use crate::{
    codex_state::CodexSession,
    state::{Session, string},
};
use rusqlite::{Connection, OpenFlags};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::windows::named_pipe::{ClientOptions, NamedPipeClient},
    sync::mpsc as async_mpsc,
};

// 当前 Codex Desktop 的 attachMessageReader 允许最大 256 MiB，长会话快照可能超过 32 MiB。
const MAX_CODEX_FRAME_BYTES: usize = 256 * 1024 * 1024;

pub enum Command {
    Rescan,
    Retry,
    Stop,
}
#[derive(Clone, Default, Serialize)]
pub struct Metrics {
    pub connections: u64,
    // 仅保存固定错误分类，便于区分主动重连与协议失败，不记录会话内容。
    pub disconnect_reasons: BTreeMap<&'static str, u64>,
    pub max_frame_bytes: u32,
    pub snapshots: u64,
    pub patch_batches: u64,
    pub content_updates: u64,
    pub revision_gaps: u64,
    pub rejected_events: u64,
    pub discovered: usize,
    pub index_version: u32,
    pub controls_sent: u64,
    pub subscriptions_closed: bool,
}
pub enum Event {
    Connection(&'static str, String),
    Session(Box<Session>),
    Metrics(Metrics),
}
pub struct Monitor {
    pub commands: async_mpsc::UnboundedSender<Command>,
    pub events: mpsc::Receiver<Event>,
    pub final_metrics: Arc<Mutex<Option<Metrics>>>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Monitor {
    pub fn start() -> Self {
        let (commands, receiver) = async_mpsc::unbounded_channel();
        let (sender, events) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let final_metrics = Arc::new(Mutex::new(None));
        let finished = final_metrics.clone();
        let worker = thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build();
            match runtime {
                Ok(rt) => {
                    let metrics = rt.block_on(run(receiver, sender, worker_stop));
                    *finished.lock().unwrap() = Some(metrics);
                }
                Err(_) => {
                    sender
                        .send(Event::Connection("error", "后台运行时初始化失败".into()))
                        .ok();
                }
            }
        });
        Self {
            commands,
            events,
            final_metrics,
            stop,
            worker: Some(worker),
        }
    }
}
impl Drop for Monitor {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.commands.send(Command::Stop).ok();
        if let Some(worker) = self.worker.take() {
            worker.join().ok();
        }
    }
}

fn discover() -> Result<(u32, Vec<String>), &'static str> {
    let home = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(|p| PathBuf::from(p).join(".codex")))
        .ok_or("index_home_missing")?;
    let mut choices = Vec::new();
    for entry in std::fs::read_dir(&home)
        .map_err(|_| "index_directory_unavailable")?
        .flatten()
    {
        let name = entry.file_name().to_string_lossy().into_owned();
        if let Some(version) = name
            .strip_prefix("state_")
            .and_then(|s| s.strip_suffix(".sqlite"))
            .and_then(|s| s.parse::<u32>().ok())
        {
            choices.push((version, entry.path()));
        }
    }
    let (version, path) = choices
        .into_iter()
        .max_by_key(|c| c.0)
        .ok_or("index_missing")?;
    let db = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|_| "index_open_failed")?;
    db.busy_timeout(Duration::from_millis(300))
        .map_err(|_| "index_busy")?;
    db.pragma_update(None, "query_only", true)
        .map_err(|_| "index_readonly_failed")?;
    let mut statement=db.prepare("SELECT id FROM threads WHERE source='vscode' AND archived=0 ORDER BY updated_at DESC LIMIT 1000").map_err(|_|"index_schema_unsupported")?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|_| "index_query_failed")?;
    let mut ids = Vec::new();
    for row in rows {
        let id = row.map_err(|_| "index_row_failed")?;
        if uuid::Uuid::parse_str(&id).is_ok() {
            ids.push(id);
        }
    }
    Ok((version, ids))
}

pub fn readonly_packet(packet: &Value) -> bool {
    match (string(packet, "type"), string(packet, "method")) {
        ("request", "initialize" | "thread-owner-discovery") => true,
        ("broadcast", "thread-stream-following-changed") => packet
            .pointer("/params/following")
            .is_some_and(Value::is_boolean),
        ("client-discovery-response", _) => {
            packet.pointer("/response/canHandle") == Some(&Value::Bool(false))
        }
        _ => false,
    }
}
async fn send(pipe: &mut NamedPipeClient, packet: Value) -> Result<(), &'static str> {
    if !readonly_packet(&packet) {
        return Err("readonly_violation");
    }
    let bytes = serde_json::to_vec(&packet).map_err(|_| "encode_failed")?;
    pipe.write_all(&(bytes.len() as u32).to_le_bytes())
        .await
        .map_err(|_| "pipe_write_failed")?;
    pipe.write_all(&bytes)
        .await
        .map_err(|_| "pipe_write_failed")?;
    Ok(())
}
async fn follow(
    pipe: &mut NamedPipeClient,
    client: &str,
    session: &CodexSession,
    enabled: bool,
) -> Result<(), &'static str> {
    send(pipe,json!({"type":"broadcast","method":"thread-stream-following-changed","version":1,"sourceClientId":client,"targetClientIds":[session.owner],"params":{"conversationId":session.id,"hostId":"local","following":enabled}})).await
}
enum Pending {
    Initialize(Instant),
    Owner(String, Instant),
}

fn next_packet(buffer: &mut Vec<u8>) -> Result<Option<Value>, &'static str> {
    if buffer.len() < 4 {
        return Ok(None);
    }
    let size = u32::from_le_bytes(buffer[..4].try_into().unwrap()) as usize;
    if size == 0 || size > MAX_CODEX_FRAME_BYTES {
        return Err("invalid_frame_size");
    }
    if buffer.len() < size + 4 {
        return Ok(None);
    }
    let packet = serde_json::from_slice(&buffer[4..size + 4]).map_err(|_| "invalid_frame_json")?;
    buffer.drain(..size + 4);
    Ok(Some(packet))
}

async fn connected(
    pipe: &mut NamedPipeClient,
    commands: &mut async_mpsc::UnboundedReceiver<Command>,
    events: &mpsc::Sender<Event>,
    sessions: &mut HashMap<String, CodexSession>,
    metrics: &mut Metrics,
    stop: &AtomicBool,
) -> Result<bool, &'static str> {
    metrics.connections += 1;
    let generation = metrics.connections;
    let mut client = "initializing-client".to_string();
    let init_id = uuid::Uuid::new_v4().to_string();
    send(pipe,json!({"type":"request","requestId":init_id,"sourceClientId":client,"version":0,"method":"initialize","params":{"clientType":"agmanage-readonly-native"},"timeoutMs":6000})).await?;
    let mut pending = HashMap::from([(init_id, Pending::Initialize(Instant::now()))]);
    let mut candidates = VecDeque::<String>::new();
    let mut initialized = false;
    let mut next_scan = Instant::now();
    let mut index_error = None;
    let mut buffer = Vec::<u8>::new();
    let mut chunk = vec![0u8; 65536];
    let mut timer = tokio::time::interval(Duration::from_millis(100));
    let mut dirty = std::collections::HashSet::new();
    let mut snapshot_deadlines: HashMap<String, Instant> = HashMap::new();
    loop {
        tokio::select! {
            command=commands.recv()=>match command {
                Some(Command::Rescan)=>next_scan=Instant::now(),
                Some(Command::Retry)=>return Ok(false),
                Some(Command::Stop)|None=>{
                    let unsubscribe=async {for session in sessions.values(){follow(pipe,&client,session,false).await.ok();}};
                    tokio::time::timeout(Duration::from_millis(700),unsubscribe).await.ok();
                    metrics.subscriptions_closed=true;events.send(Event::Metrics(metrics.clone())).ok();return Ok(true);
                }
            },
            read=pipe.read(&mut chunk)=>{
                let read=read.map_err(|_|"pipe_read_failed")?;if read==0{return Err("pipe_closed");}
                buffer.extend_from_slice(&chunk[..read]);
                loop {
                    if buffer.len()>=4{metrics.max_frame_bytes=metrics.max_frame_bytes.max(u32::from_le_bytes(buffer[..4].try_into().unwrap()));}
                    let Some(packet)=next_packet(&mut buffer)? else {break;};
                    match string(&packet,"type") {
                        "response"=>{
                            let Some(request)=pending.remove(string(&packet,"requestId"))else{continue;};
                            match request {
                                Pending::Initialize(_)=>{
                                    client=packet.pointer("/result/clientId").and_then(Value::as_str).ok_or("protocol_initialize_unsupported")?.into();
                                    initialized=true;events.send(Event::Connection("syncing","已连接，正在发现并同步已有会话".into())).ok();
                                },
                                Pending::Owner(id,_)=>{
                                    let owner=string(&packet,"handledByClientId");
                                    if string(&packet,"resultType")=="success"&&!owner.is_empty(){
                                        let needs_follow=sessions.get(&id).is_none_or(|s|s.generation!=generation||s.owner!=owner||(!s.fresh&&!snapshot_deadlines.contains_key(&id)));
                                        let session=sessions.entry(id.clone()).or_insert_with(||CodexSession::new(id.clone(),owner.into()));
                                        if needs_follow {if session.owner!=owner{session.revision=None;}session.owner=owner.into();session.fresh=false;follow(pipe,&client,session,true).await?;snapshot_deadlines.insert(id.clone(),Instant::now());dirty.insert(id);}
                                    }else if let Some(session)=sessions.get_mut(&id){session.fresh=false;dirty.insert(id);}
                                }
                            }
                        },
                        "client-discovery-request"=>{send(pipe,json!({"type":"client-discovery-response","requestId":packet["requestId"],"response":{"canHandle":false}})).await?;},
                        "broadcast" if string(&packet,"method")=="thread-stream-state-changed"=>{
                            let params=&packet["params"];if string(params,"hostId")!="local"{metrics.rejected_events+=1;continue;}
                            let id=string(params,"conversationId");let Some(session)=sessions.get_mut(id)else{metrics.rejected_events+=1;continue;};
                            if session.owner!=string(&packet,"sourceClientId"){metrics.rejected_events+=1;continue;}
                            let change=&params["change"];
                            let previous_messages=session.messages.clone();
                            let applied=match string(change,"type") {
                                "snapshot"=>{
                                    let revision=change["revision"].as_u64().ok_or("snapshot_revision_missing")?;
                                    if session.revision.is_some_and(|r|r>revision){Err("snapshot_revision_regressed")}
                                    else{metrics.snapshots+=1;session.snapshot(change["conversationState"].clone(),revision,generation)}
                                },
                                "patches"=>{metrics.patch_batches+=1;session.patches(change,generation)},_=>continue,
                            };
                            match applied {
                                Ok(())=>{snapshot_deadlines.remove(id);if previous_messages!=session.messages{metrics.content_updates+=1;}dirty.insert(id.to_owned());},
                                Err("desktop_schema_unsupported")=>return Err("desktop_schema_unsupported"),
                                Err(_)=>{
                                    metrics.revision_gaps+=1;session.fresh=false;dirty.insert(id.to_owned());
                                    if !snapshot_deadlines.contains_key(id){follow(pipe,&client,session,false).await?;follow(pipe,&client,session,true).await?;snapshot_deadlines.insert(id.into(),Instant::now());}
                                }
                            }
                        },_=>{},
                    }
                }
            },
            _=timer.tick()=>{
                if stop.load(Ordering::Acquire){
                    let unsubscribe=async{for session in sessions.values(){follow(pipe,&client,session,false).await.ok();}};
                    tokio::time::timeout(Duration::from_millis(700),unsubscribe).await.ok();metrics.subscriptions_closed=true;events.send(Event::Metrics(metrics.clone())).ok();return Ok(true);
                }
                if pending.values().any(|p|matches!(p,Pending::Initialize(t) if t.elapsed()>Duration::from_secs(7))){return Err("initialize_timeout");}
                pending.retain(|_,p|!matches!(p,Pending::Owner(_,t) if t.elapsed()>Duration::from_secs(7)));
                if initialized&&Instant::now()>=next_scan&&candidates.is_empty() {
                    match discover(){
                        Ok((version,ids))=>{metrics.index_version=version;candidates=ids.into();index_error=None;},
                        Err(code)=>index_error=Some(code),
                    }
                    next_scan=Instant::now()+Duration::from_secs(20);
                }
                if initialized {
                    while pending.len()<16 {
                        let Some(id)=candidates.pop_front()else{break;};
                        let request_id=uuid::Uuid::new_v4().to_string();
                        send(pipe,json!({"type":"request","requestId":request_id,"sourceClientId":client,"version":1,"method":"thread-owner-discovery","params":{"conversationId":id,"hostId":"local"},"timeoutMs":6000})).await?;
                        pending.insert(request_id,Pending::Owner(id,Instant::now()));
                    }
                }
                for id in dirty.drain(){if let Some(session)=sessions.get(&id){let display=session.display();events.send(Event::Session(Box::new(display))).ok();}}
                let waiting=snapshot_deadlines.values().any(|t|t.elapsed()<Duration::from_secs(8));
                if initialized {
                    let healthy=sessions.values().filter(|s|s.fresh).count();
                    if let Some(code)=index_error{events.send(Event::Connection("index_error",format!("会话索引不可用，请检查访问权限并重新扫描（{code}）"))).ok();}
                    else{events.send(Event::Connection(if healthy>0{"connected"}else if waiting{"syncing"}else{"empty"},if healthy>0{format!("已同步 {healthy} 个会话")}else if waiting{"正在等待会话快照".into()}else{"未发现可监控会话；请在 Codex Desktop 中打开会话".into()})).ok();}
                }
                metrics.discovered=sessions.len();events.send(Event::Metrics(metrics.clone())).ok();
            }
        }
    }
}

async fn run(
    mut commands: async_mpsc::UnboundedReceiver<Command>,
    events: mpsc::Sender<Event>,
    stop: Arc<AtomicBool>,
) -> Metrics {
    let mut sessions = HashMap::new();
    let mut metrics = Metrics::default();
    let mut backoff = 1u64;
    loop {
        if stop.load(Ordering::Acquire) {
            break;
        }
        events
            .send(Event::Connection(
                "connecting",
                "正在连接本机 Codex Desktop".into(),
            ))
            .ok();
        let outcome = match ClientOptions::new().open(r"\\.\pipe\codex-ipc") {
            Ok(mut pipe) => {
                connected(
                    &mut pipe,
                    &mut commands,
                    &events,
                    &mut sessions,
                    &mut metrics,
                    &stop,
                )
                .await
            }
            Err(_) => Err("desktop_pipe_unavailable"),
        };
        if matches!(outcome, Ok(true)) {
            break;
        }
        if sessions.values().any(|s| s.fresh) {
            backoff = 1;
        }
        for session in sessions.values_mut() {
            session.fresh = false;
            let display = session.display();
            events.send(Event::Session(Box::new(display))).ok();
        }
        let reason = outcome.err().unwrap_or("manual_reconnect");
        *metrics.disconnect_reasons.entry(reason).or_default() += 1;
        events
            .send(Event::Connection(
                "reconnecting",
                format!("监控连接已断开，{backoff} 秒后重连 · {reason}"),
            ))
            .ok();
        tokio::select! {
            command=commands.recv()=>if matches!(command,Some(Command::Stop)|None){break;},
            _=tokio::time::sleep(Duration::from_secs(backoff))=>{},
        }
        backoff = (backoff * 2).min(15);
    }
    metrics.subscriptions_closed = true;
    events.send(Event::Metrics(metrics.clone())).ok();
    metrics
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_snapshot_fragments_preserve_the_next_packet() {
        let text = "x".repeat(32 * 1024 * 1024 + 1);
        let first = serde_json::to_vec(&json!({"type":"broadcast","params":{"text":text}})).unwrap();
        let second = serde_json::to_vec(&json!({"type":"response","result":{"ok":true}})).unwrap();
        let mut buffer = Vec::new();
        buffer.extend_from_slice(&(first.len() as u32).to_le_bytes()[..2]);
        assert_eq!(next_packet(&mut buffer).unwrap(), None);
        buffer.extend_from_slice(&(first.len() as u32).to_le_bytes()[2..]);
        buffer.extend_from_slice(&first[..first.len() - 1]);
        assert_eq!(next_packet(&mut buffer).unwrap(), None);
        buffer.push(*first.last().unwrap());
        buffer.extend_from_slice(&(second.len() as u32).to_le_bytes());
        buffer.extend_from_slice(&second);
        let packet = next_packet(&mut buffer).unwrap().unwrap();
        assert_eq!(packet["params"]["text"].as_str().unwrap().len(), text.len());
        assert_eq!(next_packet(&mut buffer).unwrap().unwrap()["result"]["ok"], true);
        assert!(buffer.is_empty());
    }

    #[test]
    fn invalid_frames_remain_rejected_without_allocating_the_payload() {
        for size in [0, (MAX_CODEX_FRAME_BYTES + 1) as u32, u32::MAX] {
            let mut buffer = size.to_le_bytes().to_vec();
            assert_eq!(next_packet(&mut buffer), Err("invalid_frame_size"));
            assert_eq!(buffer.len(), 4);
        }
        let mut invalid = vec![1, 0, 0, 0, b'{'];
        assert_eq!(next_packet(&mut invalid), Err("invalid_frame_json"));
    }

    #[test]
    fn rejects_all_task_controls() {
        for method in [
            "thread/resume",
            "turn/start",
            "turn/interrupt",
            "thread/create",
            "approval/respond",
            "send-message",
        ] {
            assert!(!readonly_packet(&json!({"type":"request","method":method})));
        }
        assert!(readonly_packet(
            &json!({"type":"request","method":"thread-owner-discovery"})
        ));
        assert!(!readonly_packet(
            &json!({"type":"client-discovery-response","response":{"canHandle":true}})
        ));
    }
}
