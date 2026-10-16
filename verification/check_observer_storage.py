"""只读核对观察插件的物理存储路径，防止 AppData 重定向造成虚假安装成功。"""
import ctypes
import json
import msvcrt
import os
from pathlib import Path

root = Path(os.environ["USERPROFILE"]) / ".agmanage"
get_final = ctypes.WinDLL("kernel32", use_last_error=True).GetFinalPathNameByHandleW
get_final.argtypes = [ctypes.c_void_p, ctypes.c_wchar_p, ctypes.c_uint32, ctypes.c_uint32]
get_final.restype = ctypes.c_uint32
paths = [root / "installation-2.json", root / "installation-3.json", root / "plugins/opencode/index.mjs", Path(os.environ["USERPROFILE"]) / ".dsh/profiles/desktop/node_modules/@dsh-external/agmanage-observer/package.json"]
if (root / "observer.json").exists():
    paths.append(root / "observer.json")
rows = []
for path in paths:
    with path.open("rb") as file:
        buffer = ctypes.create_unicode_buffer(32768)
        if not get_final(msvcrt.get_osfhandle(file.fileno()), buffer, len(buffer), 0):
            raise ctypes.WinError(ctypes.get_last_error())
        physical = Path(buffer.value.removeprefix("\\\\?\\"))
        rows.append({"requested": str(path), "physical": str(physical), "sharedRoot": physical.is_relative_to(root)})
result = {"passed": all(row["sharedRoot"] for row in rows), "files": rows}
output = Path(__file__).resolve().parent.parent / "artifacts/observer/storage-physical-paths.json"
output.write_text(json.dumps(result, indent=2), encoding="utf-8")
print(json.dumps({"passed": result["passed"], "filesChecked": len(rows)}))
raise SystemExit(0 if result["passed"] else 1)
