"""从桌面启动环境只读检查本项目插件路径，不启动任务或读取会话。"""
import json
import os
from pathlib import Path
import subprocess

root = Path(__file__).resolve().parent.parent
manager = Path(os.environ["USERPROFILE"]) / ".agmanage"
profile = Path(os.environ["USERPROFILE"]) / ".dsh" / "profiles" / "desktop"
link = profile / "node_modules" / "@dsh-external" / "agmanage-observer"
record = manager / "installation-3.json"
result = {"managerRecordExists": record.is_file(), "linkExists": link.is_dir(), "manifestExists": (link / "package.json").is_file()}
if record.is_file():
    installed = json.loads(record.read_text(encoding="utf-8"))
    executable = Path(installed["executable"])
    environment = {**os.environ, "ELECTRON_RUN_AS_NODE": "1"}
    run = subprocess.run([str(executable), "--expose-internals", str(root / "verification" / "check_harness_bundle.cjs"), str(executable.parent / "resources" / "app.asar" / "dsh"), str(profile)], env=environment, capture_output=True, text=True, encoding="utf-8", creationflags=subprocess.CREATE_NO_WINDOW, timeout=30)
    result.update({"resolverExitCode": run.returncode, "resolverOutput": run.stdout, "resolverError": run.stderr})
(root / "artifacts" / "observer" / "explorer-plugin-resolution.json").write_text(json.dumps(result, indent=2), encoding="utf-8")
