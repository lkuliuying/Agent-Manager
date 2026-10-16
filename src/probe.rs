use crate::{
    observer::{self, Command, Event},
    source::{SessionKey, SourceKind},
};
use serde_json::json;
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

// 只保存计数和错误分类，真实标题、会话标识和正文不会进入验收文件。
pub fn run(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let seconds = args
        .iter()
        .position(|a| a == "--seconds")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(15)
        .clamp(2, 60);
    let observer = observer::Observer::start();
    let mut sessions = HashMap::new();
    let mut connections = HashMap::new();
    let mut connection_counts = HashMap::<SourceKind, (usize, usize, String)>::new();
    let mut errors = Vec::new();
    let mut wanted = HashMap::new();
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(seconds) {
        if let Ok(event) = observer.events.recv_timeout(Duration::from_millis(200)) {
            match event {
                Event::Connection(source, epoch, state) => {
                    let counts = connection_counts.entry(source.kind).or_default();
                    if state.connected {
                        counts.0 += 1;
                    } else {
                        counts.1 += 1;
                        counts.2 = state.note.clone();
                    }
                    connections.insert(source, (epoch, state));
                }
                Event::Catalog(source, epoch, rows) => {
                    if !connections
                        .get(&source)
                        .is_some_and(|(e, s)| *e == epoch && s.connected)
                    {
                        continue;
                    }
                    let keys = rows
                        .iter()
                        .take(3)
                        .map(|s| SessionKey {
                            source: s.source.clone(),
                            native_id: s.native_id.clone(),
                        })
                        .collect::<Vec<_>>();
                    for row in rows {
                        sessions.entry(row.id.clone()).or_insert(row);
                    }
                    wanted.insert(source, keys);
                    let interest = wanted.values().flatten().cloned().collect();
                    observer.commands.send(Command::Interest(interest))?;
                }
                Event::Session(epoch, session) => {
                    if connections
                        .get(&session.source)
                        .is_some_and(|(e, s)| *e == epoch && s.connected)
                    {
                        sessions.insert(session.id.clone(), *session);
                    }
                }
                Event::Error(kind, code) => errors.push(json!({"kind":kind,"code":code})),
                Event::SessionError(source, _, _) => {
                    errors.push(json!({"kind":source.kind,"code":"snapshot_read_failed"}))
                }
            }
        }
    }
    let sources=[SourceKind::Opencode,SourceKind::Deepseek].map(|kind|{
        let (connected, disconnected, note) = connection_counts.get(&kind).cloned().unwrap_or_default();
        json!({"kind":kind,"connections":connections.iter().filter(|(id,(_,state))|id.kind==kind&&state.connected).count(),"connectEvents":connected,"disconnectEvents":disconnected,"lastDisconnect":note,"sessions":sessions.values().filter(|s|s.source.kind==kind).count(),"snapshots":sessions.values().filter(|s|s.source.kind==kind&&s.messages_loaded).count(),"messages":sessions.values().filter(|s|s.source.kind==kind).map(|s|s.messages.len()).sum::<usize>()})
    });
    drop(observer);
    let report = json!({"sources":sources,"errors":errors,"controlsSent":0,"rawContentSaved":false,"elapsedSeconds":seconds});
    if let Some(path) = args
        .iter()
        .position(|a| a == "--probe-output")
        .and_then(|i| args.get(i + 1))
    {
        let path = std::path::Path::new(path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, serde_json::to_vec_pretty(&report)?)?;
    }
    println!("{report}");
    Ok(())
}
