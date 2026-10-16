"""只读返回候选 ID 给父验证进程；不查询标题、正文、凭据或历史文件。"""
import json
import os
from pathlib import Path
import re
import sqlite3


def discover():
    root = Path(os.environ.get("CODEX_HOME") or Path.home() / ".codex")
    candidates = []
    for path in root.glob("state_*.sqlite"):
        match = re.fullmatch(r"state_(\d+)\.sqlite", path.name)
        if match:
            candidates.append((int(match.group(1)), path))
    if not candidates:
        raise RuntimeError("thread_index_not_found")
    version, path = max(candidates)
    connection = sqlite3.connect(path.as_uri() + "?mode=ro", uri=True, timeout=0.3)
    try:
        connection.execute("PRAGMA query_only=ON")
        fields = {row[1] for row in connection.execute("PRAGMA table_info(threads)")}
        required = {"id", "source", "originator", "archived", "updated_at"}
        if not required.issubset(fields):
            raise RuntimeError("unsupported_thread_index_schema")
        # Desktop 的旧根会话可能没有 originator；vscode 只作候选来源，须由 IPC 确认。
        total = connection.execute(
            "SELECT count(*) FROM threads WHERE source='vscode' AND archived=0"
        ).fetchone()[0]
        rows = connection.execute(
            "SELECT id FROM threads WHERE source='vscode' AND archived=0 "
            "ORDER BY updated_at DESC LIMIT 1000"
        ).fetchall()
        return {"indexVersion": version, "candidateTotal": total,
                "candidates": [row[0] for row in rows]}
    finally:
        connection.close()


try:
    print(json.dumps(discover()))
except Exception as error:
    # 不把 SQLite 错误中的本地路径或其他内容写入报告。
    print(json.dumps({"error": type(error).__name__}))
    raise SystemExit(1)
