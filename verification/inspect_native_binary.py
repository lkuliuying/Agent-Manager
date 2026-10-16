"""读取本地 PE 架构、导入表和摘要；不执行程序或读取用户数据。"""
import hashlib
import json
from pathlib import Path
import struct
import sys

path = Path(sys.argv[1])
data = path.read_bytes()
pe = struct.unpack_from("<I", data, 0x3C)[0]
if data[pe:pe + 4] != b"PE\0\0":
    raise SystemExit("invalid_pe")
machine, sections = struct.unpack_from("<HH", data, pe + 4)
optional_size = struct.unpack_from("<H", data, pe + 20)[0]
optional = pe + 24
magic = struct.unpack_from("<H", data, optional)[0]
subsystem = struct.unpack_from("<H", data, optional + 68)[0]
directories = optional + (112 if magic == 0x20B else 96)
import_rva = struct.unpack_from("<I", data, directories + 8)[0]
section_start = optional + optional_size

def offset(rva):
    for index in range(sections):
        start = section_start + index * 40
        size, address, raw_size, raw = struct.unpack_from("<IIII", data, start + 8)
        if address <= rva < address + max(size, raw_size):
            return raw + rva - address
    raise ValueError("unmapped_rva")

imports = []
position = offset(import_rva)
while True:
    fields = struct.unpack_from("<IIIII", data, position)
    if not any(fields):
        break
    name_offset = offset(fields[3])
    imports.append(data[name_offset:data.index(0, name_offset)].decode("ascii"))
    position += 20

separate_runtime = [name for name in imports if any(part in name.lower() for part in ("vcruntime", "msvcp", "webview", "tauri", "python", "node"))]
report = {"machine": hex(machine), "x64": machine == 0x8664, "guiSubsystem": subsystem == 2, "bytes": len(data), "sha256": hashlib.sha256(data).hexdigest(), "imports": sorted(imports), "separateRuntimeImports": separate_runtime}
print(json.dumps(report, ensure_ascii=False, indent=2))
if not report["x64"] or not report["guiSubsystem"] or separate_runtime:
    raise SystemExit(2)
