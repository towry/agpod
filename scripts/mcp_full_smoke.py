#!/usr/bin/env python3
"""Smoke test for agpod-mcp full-access stdio mode."""

import argparse
import json
import os
import socket
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path


def rpc(proc: subprocess.Popen[str], message: dict) -> dict:
    assert proc.stdin is not None
    assert proc.stdout is not None
    proc.stdin.write(json.dumps(message) + "\n")
    proc.stdin.flush()
    while True:
        line = proc.stdout.readline()
        if line == "":
            stderr = ""
            if proc.stderr is not None:
                stderr = proc.stderr.read()
            raise RuntimeError(f"MCP server closed unexpectedly. stderr={stderr}")
        line = line.strip()
        if line:
            return json.loads(line)


def notify(proc: subprocess.Popen[str], message: dict) -> None:
    assert proc.stdin is not None
    proc.stdin.write(json.dumps(message) + "\n")
    proc.stdin.flush()


def pick_free_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


def main() -> int:
    parser = argparse.ArgumentParser(description="Smoke test agpod-mcp full mode over stdio.")
    parser.add_argument(
        "--repo-root",
        default="/Users/towry/workspace/agpod",
        help="Absolute repo root. Default: current agpod workspace path.",
    )
    parser.add_argument(
        "--data-dir",
        default=None,
        help="Temporary case DB path. Defaults to a unique temp path.",
    )
    parser.add_argument(
        "--server-addr",
        default=None,
        help="Temporary case server addr. Defaults to a random localhost port.",
    )
    args = parser.parse_args()

    repo_root = Path(args.repo_root)
    data_dir = args.data_dir or tempfile.mkdtemp(prefix="agpod-mcp-full-smoke-")
    server_addr = args.server_addr or f"127.0.0.1:{pick_free_port()}"
    env = os.environ.copy()
    env["AGPOD_CASE_DATA_DIR"] = data_dir
    env["AGPOD_CASE_SERVER_ADDR"] = server_addr

    shutil.rmtree(data_dir, ignore_errors=True)

    proc = subprocess.Popen(
        [str(repo_root / "target" / "debug" / "agpod-mcp")],
        cwd=repo_root,
        env=env,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    try:
        rpc(
            proc,
            {
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2025-06-18",
                    "capabilities": {},
                    "clientInfo": {"name": "smoke", "version": "1.0"},
                },
            },
        )
        notify(
            proc,
            {
                "jsonrpc": "2.0",
                "method": "notifications/initialized",
                "params": {},
            },
        )
        tools = rpc(proc, {"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}})
        opened = rpc(
            proc,
            {
                "jsonrpc": "2.0",
                "id": 3,
                "method": "tools/call",
                "params": {
                    "name": "case_open",
                    "arguments": {
                        "mode": "new",
                        "goal": "full smoke goal",
                        "direction": "full smoke direction",
                    },
                },
            },
        )
        current = rpc(
            proc,
            {
                "jsonrpc": "2.0",
                "id": 4,
                "method": "tools/call",
                "params": {"name": "case_current", "arguments": {}},
            },
        )
        listed = rpc(
            proc,
            {
                "jsonrpc": "2.0",
                "id": 5,
                "method": "tools/call",
                "params": {"name": "case_list", "arguments": {}},
            },
        )
        recall = rpc(
            proc,
            {
                "jsonrpc": "2.0",
                "id": 6,
                "method": "tools/call",
                "params": {
                    "name": "case_recall",
                    "arguments": {
                        "mode": "find",
                        "query": "full smoke",
                        "find_limit": 5,
                    },
                },
            },
        )
    finally:
        proc.terminate()

    opened_result = opened["result"]["structuredContent"]["result"]["raw"]
    current_result = current["result"]["structuredContent"]["result"]["raw"]
    list_result = listed["result"]["structuredContent"]["result"]["raw"]
    recall_result = recall["result"]["structuredContent"]["result"]["raw"]

    summary = {
        "tools": [tool["name"] for tool in tools["result"]["tools"]],
        "case_open": opened_result.get("ok"),
        "case_current": current_result.get("ok"),
        "case_list": list_result.get("ok"),
        "case_recall": recall_result.get("ok"),
        "opened_case_id": opened_result.get("case", {}).get("id"),
        "current_case_id": current_result.get("case", {}).get("id"),
    }
    print(json.dumps(summary, ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
