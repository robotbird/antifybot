#!/usr/bin/env python3
"""Generate server-side token hashes and separate private client credentials."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import secrets

parser = argparse.ArgumentParser()
parser.add_argument("--output", type=Path, required=True)
parser.add_argument("--listen", default="127.0.0.1:53319")
parser.add_argument("--data-dir", default="/var/lib/antify-relay")
args = parser.parse_args()
args.output.mkdir(parents=True, exist_ok=True, mode=0o700)
devices = [{"id": f"desktop-{letter}", "alias": f"设备 {letter.upper()}", "token": secrets.token_urlsafe(32)} for letter in ("a", "b")]
server = {
    "listen": args.listen,
    "data_dir": args.data_dir,
    "devices": [{"id": d["id"], "alias": d["alias"], "token_sha256": hashlib.sha256(d["token"].encode()).hexdigest()} for d in devices],
    "max_file_bytes": 100 * 1024 * 1024,
    "max_storage_bytes": 1024 * 1024 * 1024,
    "file_ttl_secs": 86400,
    "message_ttl_secs": 7 * 86400,
}
for name, contents in (("server.json", server), ("credentials.json", {"devices": devices})):
    path = args.output / name
    with os.fdopen(os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), "w") as f:
        json.dump(contents, f, ensure_ascii=False, indent=2)
        f.write("\n")
    print(path)
