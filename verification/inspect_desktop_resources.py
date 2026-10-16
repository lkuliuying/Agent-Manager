"""只读检查已安装 Electron 应用的公开代码，不读取用户配置或会话存储。"""

import argparse
import json
from pathlib import Path
import re
import struct


def entries(nodes, prefix=""):
    for name, node in nodes.items():
        path = f"{prefix}/{name}" if prefix else name
        if "files" in node:
            yield from entries(node["files"], path)
        elif "offset" in node:
            yield path, node


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("archive", type=Path)
    parser.add_argument("--file", default="")
    parser.add_argument("--term", action="append", default=[])
    parser.add_argument("--context", type=int, default=180)
    parser.add_argument("--limit", type=int, default=12)
    parser.add_argument("--skip-hits", type=int, default=0)
    parser.add_argument("--hits", type=int, default=3)
    args = parser.parse_args()

    with args.archive.open("rb") as source:
        prefix = source.read(16)
        header_size = struct.unpack_from("<I", prefix, 4)[0]
        json_size = struct.unpack_from("<I", prefix, 12)[0]
        header = json.loads(source.read(json_size))
        data_start = 8 + header_size
        files = list(entries(header["files"]))

        def read(node):
            source.seek(data_start + int(node["offset"]))
            return source.read(node["size"]).decode("utf-8", errors="replace")

        package = next((node for path, node in files if path == "package.json"), None)
        if package:
            data = json.loads(read(package))
            print(json.dumps({"package": {key: data.get(key) for key in ("name", "version", "main")}}, ensure_ascii=False))

        if not args.term:
            paths = [path for path, node in files if re.search(r"(?:main|preload|ipc|rpc|bridge|socket|approval|permission)[^/]*\.(?:js|cjs|mjs)$", path, re.I) and "node_modules/" not in path]
            print(json.dumps({"code_files": paths[:100], "file_count": len(files)}, ensure_ascii=False))
            return

        remaining = args.limit
        for path, node in files:
            if not path.endswith((".js", ".cjs", ".mjs")) or "node_modules/" in path:
                continue
            if args.file and args.file not in path:
                continue
            text = read(node)
            for term in args.term:
                positions = list(re.finditer(re.escape(term), text))
                if not positions:
                    continue
                result = {"file": path, "term": term, "hits": len(positions)}
                if args.file:
                    result["contexts"] = [text[max(0, match.start() - args.context):match.end() + args.context] for match in positions[args.skip_hits:args.skip_hits + args.hits]]
                print(json.dumps(result, ensure_ascii=False))
                remaining -= 1
                if remaining <= 0:
                    return


if __name__ == "__main__":
    main()
