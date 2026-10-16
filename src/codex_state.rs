use crate::state::{Message, Session, TaskStatus, now, string, timestamp};
use serde_json::Value;
use std::{
    collections::HashSet,
    ops::{Deref, DerefMut},
};

#[derive(Clone, Debug)]
pub struct CodexSession {
    pub display: Session,
    pub id: String,
    pub owner: String,
    pub raw: Value,
    pub revision: Option<u64>,
    pub generation: u64,
}
impl Deref for CodexSession {
    type Target = Session;
    fn deref(&self) -> &Session {
        &self.display
    }
}
impl DerefMut for CodexSession {
    fn deref_mut(&mut self) -> &mut Session {
        &mut self.display
    }
}

pub fn turns(raw: &Value) -> Vec<&Value> {
    let canonical = raw
        .pointer("/turnHistory/history/entitiesByKey")
        .and_then(Value::as_object);
    let mut result: Vec<_> = if let Some(entities) = canonical.filter(|m| !m.is_empty()) {
        entities.values().collect()
    } else {
        raw.get("turns")
            .and_then(Value::as_array)
            .map(|a| a.iter().collect())
            .unwrap_or_default()
    };
    // 当前 Desktop 的 canonical 历史使用映射；按轮次时间恢复稳定顺序，不依赖键顺序。
    result.sort_by_key(|t| {
        t.get("turnStartedAtMs")
            .and_then(Value::as_i64)
            .unwrap_or(0)
    });
    result
}

fn content_text(value: &Value) -> String {
    if let Some(text) = value.as_str() {
        return text.into();
    }
    value
        .as_array()
        .map(|parts| {
            parts
                .iter()
                .filter_map(|p| {
                    if p.as_str().is_some() {
                        return p.as_str().map(str::to_owned);
                    }
                    match string(p, "type") {
                        "text" | "inputText" => Some(string(p, "text").into()),
                        "localImage" | "image" => Some("[图片附件]".into()),
                        _ => None,
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

pub fn normalize(raw: &Value) -> Vec<Message> {
    let mut messages = Vec::new();
    let mut seen = HashSet::new();
    for turn in turns(raw) {
        let started = turn
            .get("turnStartedAtMs")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        let turn_id = string(turn, "turnId");
        let Some(items) = turn.get("items").and_then(Value::as_array) else {
            continue;
        };
        for (index, item) in items.iter().enumerate() {
            let kind = string(item, "type");
            let id = format!(
                "{turn_id}:{}",
                if string(item, "id").is_empty() {
                    index.to_string()
                } else {
                    string(item, "id").into()
                }
            );
            if !seen.insert(id.clone()) {
                continue;
            }
            let (author, label, text) = match kind {
                "userMessage" => ("我", "message", content_text(&item["content"])),
                "steeringUserMessage" => ("我", "message", content_text(&item["input"])),
                "agentMessage" => ("Agent", "message", string(item, "text").to_owned()),
                // 只显示来源提供的可见摘要，不读取内部推理正文。
                "reasoning" => ("Agent", "思考摘要", content_text(&item["summary"])),
                "commandExecution" => (
                    "工具",
                    "run_command",
                    format!(
                        "$ {}\n{}",
                        string(item, "command"),
                        string(item, "aggregatedOutput")
                    ),
                ),
                "fileChange" => {
                    let text = item["changes"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .map(|c| format!("{}\n{}", string(c, "path"), string(c, "diff")))
                                .collect::<Vec<_>>()
                                .join("\n\n")
                        })
                        .unwrap_or_default();
                    ("工具", "apply_patch", text)
                }
                "mcpToolCall" => {
                    let tool = string(item, "tool");
                    let text = format!(
                        "{}\n{}",
                        string(item, "server"),
                        serde_json::to_string_pretty(&item["result"]).unwrap_or_default()
                    );
                    messages.push(Message {
                        id,
                        author: "工具".into(),
                        kind: if tool.is_empty() {
                            "mcp_tool".into()
                        } else {
                            tool.into()
                        },
                        text,
                        status: string(item, "status").into(),
                        time: timestamp(started, true),
                    });
                    continue;
                }
                "webSearch" => ("工具", "web_search", string(item, "query").into()),
                "imageView" => ("工具", "view_image", string(item, "path").into()),
                "plan" | "planUpdate" => ("Agent", "步骤计划", string(item, "text").into()),
                _ => continue,
            };
            if text.trim().is_empty() {
                continue;
            }
            let item_time = turn
                .get(if author == "工具" {
                    "commandExecutionStartedAtMsById"
                } else {
                    "agentMessageCompletedAtMsById"
                })
                .and_then(|m| m.get(string(item, "id")))
                .and_then(Value::as_i64)
                .unwrap_or(started);
            messages.push(Message {
                id,
                author: author.into(),
                kind: label.into(),
                text,
                status: string(item, "status").into(),
                time: timestamp(item_time, true),
            });
        }
    }
    messages
}

fn task_status(raw: &Value) -> TaskStatus {
    let runtime = raw
        .pointer("/threadRuntimeStatus/type")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    if runtime == "active" {
        return TaskStatus::Running;
    }
    if runtime == "systemError" {
        return TaskStatus::Failed;
    }
    if runtime != "idle" {
        return TaskStatus::Unknown;
    }
    match turns(raw).last().map(|t| string(t, "status")).unwrap_or("") {
        "completed" => TaskStatus::Completed,
        "failed" => TaskStatus::Failed,
        "interrupted" | "cancelled" => TaskStatus::Interrupted,
        _ => TaskStatus::Idle,
    }
}

impl CodexSession {
    pub fn new(id: String, owner: String) -> Self {
        Self {
            display: Session::new(id.clone(), owner.clone()),
            id,
            owner,
            raw: Value::Null,
            revision: None,
            generation: 0,
        }
    }
    pub fn display(&self) -> Session {
        self.display.clone()
    }

    pub fn snapshot(
        &mut self,
        raw: Value,
        revision: u64,
        generation: u64,
    ) -> Result<(), &'static str> {
        if string(&raw, "id") != self.id
            || (!string(&raw, "hostId").is_empty() && string(&raw, "hostId") != "local")
        {
            return Err("identity_mismatch");
        }
        if !raw.get("turns").is_some_and(Value::is_array)
            && !raw
                .pointer("/turnHistory/history/entitiesByKey")
                .is_some_and(Value::is_object)
        {
            return Err("desktop_schema_unsupported");
        }
        self.raw = raw;
        self.revision = Some(revision);
        self.generation = generation;
        self.fresh = true;
        self.project();
        Ok(())
    }
    pub fn patches(&mut self, changes: &Value, generation: u64) -> Result<(), &'static str> {
        if self.generation != generation
            || !self.fresh
            || changes["baseRevision"].as_u64() != self.revision
        {
            return Err("revision_gap");
        }
        let revision = changes["revision"].as_u64().ok_or("invalid_revision")?;
        if self.revision.is_some_and(|old| revision <= old) {
            return Err("revision_not_advanced");
        }
        // 整批应用到候选快照，失败时保留完整旧状态，避免半批增量污染后续更新。
        let mut next = self.raw.clone();
        let patches = changes["patches"].as_array().ok_or("invalid_patches")?;
        for patch in patches {
            apply_patch(&mut next, patch)?;
        }
        if string(&next, "id") != self.id {
            return Err("identity_mismatch");
        }
        self.raw = next;
        self.revision = Some(revision);
        self.project();
        Ok(())
    }
    fn project(&mut self) {
        self.title = string(&self.raw, "title").to_owned();
        if self.title.is_empty() {
            self.title = "未命名会话".into();
        }
        let cwd = string(&self.raw, "cwd");
        self.project = cwd
            .trim_end_matches(['\\', '/'])
            .rsplit(['\\', '/'])
            .next()
            .filter(|s| !s.is_empty())
            .unwrap_or("Codex Desktop")
            .into();
        self.status = task_status(&self.raw);
        let messages = normalize(&self.raw);
        if messages != self.messages {
            self.message_version += 1;
            self.messages = messages;
        }
        self.created = timestamp(self.raw["createdAt"].as_i64().unwrap_or(0), false);
        self.last_sync = now();
        self.messages_loaded = true;
        self.history_complete = self
            .raw
            .pointer("/turnHistory/history/isComplete")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        self.activity = if self.status == TaskStatus::Running {
            self.messages
                .last()
                .map(|m| {
                    if m.author == "工具" {
                        format!("最近工具：{}", m.kind)
                    } else {
                        "正在处理任务".into()
                    }
                })
                .unwrap_or_else(|| "正在处理任务".into())
        } else {
            self.status.label().into()
        };
    }
}

fn segment(value: &Value) -> Result<String, &'static str> {
    value
        .as_str()
        .map(str::to_owned)
        .or_else(|| value.as_u64().map(|n| n.to_string()))
        .ok_or("invalid_patch_path")
}
pub fn apply_patch(root: &mut Value, patch: &Value) -> Result<(), &'static str> {
    let path = patch["path"].as_array().ok_or("invalid_patch_path")?;
    let op = string(patch, "op");
    if !["add", "replace", "remove"].contains(&op) {
        return Err("invalid_patch_op");
    }
    if op != "remove" && patch.get("value").is_none() {
        return Err("missing_patch_value");
    }
    if path.is_empty() {
        if op == "remove" {
            return Err("root_remove");
        }
        *root = patch.get("value").ok_or("missing_patch_value")?.clone();
        return Ok(());
    }
    let mut parent = root;
    for part in &path[..path.len() - 1] {
        let key = segment(part)?;
        parent = if parent.is_array() {
            parent
                .as_array_mut()
                .unwrap()
                .get_mut(key.parse::<usize>().map_err(|_| "invalid_array_index")?)
                .ok_or("missing_patch_parent")?
        } else {
            parent.get_mut(&key).ok_or("missing_patch_parent")?
        };
    }
    let key = segment(path.last().unwrap())?;
    let value = patch.get("value").cloned().unwrap_or(Value::Null);
    if let Some(a) = parent.as_array_mut() {
        if key == "length" && op == "replace" {
            let size = value.as_u64().ok_or("invalid_array_length")? as usize;
            if size > a.len() {
                return Err("array_length_growth");
            }
            a.truncate(size);
            return Ok(());
        }
        let index = key.parse::<usize>().map_err(|_| "invalid_array_index")?;
        match op {
            "add" if index <= a.len() => a.insert(index, value),
            "replace" if index < a.len() => a[index] = value,
            "remove" if index < a.len() => {
                a.remove(index);
            }
            _ => return Err("array_index_out_of_bounds"),
        }
    } else if let Some(object) = parent.as_object_mut() {
        match op {
            "remove" => {
                if object.remove(&key).is_none() {
                    return Err("missing_patch_target");
                }
            }
            "replace" if !object.contains_key(&key) => return Err("missing_patch_target"),
            _ => {
                object.insert(key, value);
            }
        }
    } else {
        return Err("invalid_patch_parent");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn raw(status: &str) -> Value {
        json!({"id":"test-a","hostId":"local","title":"演示","threadRuntimeStatus":{"type":"idle"},"turns":[{"turnId":"turn-a","turnStartedAtMs":20,"status":status,"items":[{"id":"tool-a","type":"commandExecution","status":"completed","command":"echo demo"}]}]})
    }
    #[test]
    fn idle_requires_turn_terminal() {
        assert_eq!(task_status(&raw("inProgress")), TaskStatus::Idle);
        assert_eq!(task_status(&raw("completed")), TaskStatus::Completed);
    }
    #[test]
    fn canonical_order_and_messages() {
        let v = json!({"turns":[],"turnHistory":{"history":{"entitiesByKey":{"z":{"turnId":"early","turnStartedAtMs":10,"items":[{"id":"a","type":"userMessage","content":[{"type":"text","text":"中文"}]}]},"a":{"turnId":"late","turnStartedAtMs":20,"items":[{"id":"b","type":"agentMessage","text":"回复"}]}}}}});
        let messages = normalize(&v);
        assert_eq!(messages[0].text, "中文");
        assert_eq!(messages[1].text, "回复");
    }
    #[test]
    fn patch_batch_is_atomic_and_isolated() {
        let mut a = CodexSession::new("test-a".into(), "owner".into());
        a.snapshot(raw("inProgress"), 1, 1).unwrap();
        let old = a.raw.clone();
        let bad = json!({"baseRevision":1,"revision":2,"patches":[{"op":"replace","path":["title"],"value":"修改"},{"op":"replace","path":["missing","child"],"value":1}]});
        assert!(a.patches(&bad, 1).is_err());
        assert_eq!(a.raw, old);
        assert_eq!(a.revision, Some(1));
        assert!(a.patches(&bad, 2).is_err());
    }
    #[test]
    fn array_patch_contract() {
        let mut value = json!({"items":[1,2]});
        apply_patch(
            &mut value,
            &json!({"op":"add","path":["items",1],"value":3}),
        )
        .unwrap();
        apply_patch(
            &mut value,
            &json!({"op":"replace","path":["items","length"],"value":1}),
        )
        .unwrap();
        assert_eq!(value, json!({"items":[1]}));
    }
    #[test]
    fn invalid_revision_and_schema_preserve_snapshot() {
        let mut s = CodexSession::new("test-a".into(), "owner".into());
        s.snapshot(raw("inProgress"), 5, 1).unwrap();
        let before = s.raw.clone();
        for revision in [Value::Null, json!(4)] {
            let change = json!({"baseRevision":5,"revision":revision,"patches":[{"op":"replace","path":["title"],"value":"新标题"}]});
            assert!(s.patches(&change, 1).is_err());
            assert_eq!(s.raw, before);
        }
        assert!(s.snapshot(json!({"id":"test-a"}), 6, 1).is_err());
        assert_eq!(s.raw, before);
    }
}
