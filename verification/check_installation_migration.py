"""在临时用户目录验证旧插件入口迁移、重复安装和卸载，不改真实 Agent 配置。"""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

binary = Path(sys.argv[1]).resolve()
host = Path(sys.argv[2]).resolve()
fixture_parent = Path(__file__).resolve().parent.parent / "artifacts" / "observer"
with tempfile.TemporaryDirectory(prefix="migration-", dir=fixture_parent) as temporary:
    root = Path(temporary).resolve()
    assert root.is_relative_to(fixture_parent.resolve())
    profile = root / "user"
    local = root / "local"
    config = root / "config"
    config.mkdir()
    legacy = (local / "AgManage/plugins/opencode/index.mjs").as_uri()
    current = (profile / ".agmanage/plugins/opencode/index.mjs").as_uri()
    files = [config / "opencode.json", config / "opencode.jsonc"]
    for file in files:
        file.write_text("// 保留测试注释\n" + json.dumps({"theme": "dark", "plugin": ["foreign-fixture", legacy]}), encoding="utf-8")
    environment = {**os.environ, "USERPROFILE": str(profile), "LOCALAPPDATA": str(local), "OPENCODE_CONFIG_DIR": str(config)}
    environment.pop("OPENCODE_CONFIG", None)
    def run(action):
        result = subprocess.run([str(binary), "--manage-source", "opencode", action, str(host)], env=environment, capture_output=True, text=True, encoding="utf-8", creationflags=subprocess.CREATE_NO_WINDOW, timeout=20)
        assert result.returncode == 0, result.stderr
    run("install")
    run("install")
    for file in files:
        source = file.read_text(encoding="utf-8")
        value = json.loads(source.split("\n", 1)[1])
        assert source.startswith("// 保留测试注释") and value["theme"] == "dark"
        assert legacy not in value["plugin"] and value["plugin"].count(current) <= 1
        assert "foreign-fixture" in value["plugin"]
    assert json.loads(files[-1].read_text(encoding="utf-8").split("\n", 1)[1])["plugin"].count(current) == 1
    assert (profile / ".agmanage/plugins/opencode/index.mjs").is_file()
    run("uninstall")
    for file in files:
        assert json.loads(file.read_text(encoding="utf-8").split("\n", 1)[1])["plugin"] == ["foreign-fixture"]
print(json.dumps({"passed": True, "temporaryProfileOnly": True, "migration": True, "idempotent": True, "uninstallPreservesForeignPlugin": True}))
