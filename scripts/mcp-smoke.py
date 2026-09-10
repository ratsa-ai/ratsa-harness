#!/usr/bin/env python3
"""Drive the RATSA-Harness MCP stdio server and print a readable transcript."""
import json
import subprocess
import sys

BIN = sys.argv[1] if len(sys.argv) > 1 else "./target/debug/ratsa-harness"

msgs = [
    {"jsonrpc": "2.0", "id": 1, "method": "initialize",
     "params": {"protocolVersion": "2024-11-05", "capabilities": {},
                "clientInfo": {"name": "t", "version": "0"}}},
    {"jsonrpc": "2.0", "method": "notifications/initialized"},
    {"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}},
    {"jsonrpc": "2.0", "id": 3, "method": "tools/call",
     "params": {"name": "ratsa_whoami", "arguments": {}}},
    {"jsonrpc": "2.0", "id": 4, "method": "tools/call",
     "params": {"name": "ratsa_search_sofs", "arguments": {"query": "舵机"}}},
    {"jsonrpc": "2.0", "id": 5, "method": "tools/call",
     "params": {"name": "ratsa_list_packages", "arguments": {"slug": "tgkgz48zc4"}}},
    {"jsonrpc": "2.0", "id": 6, "method": "tools/call",
     "params": {"name": "ratsa_pull_package",
                "arguments": {"slug": "tgkgz48zc4", "kind": "device",
                              "out_dir": "/tmp/ratsa-mcp-pull"}}},
    {"jsonrpc": "2.0", "id": 7, "method": "tools/call",
     "params": {"name": "ratsa_report_run",
                "arguments": {"slug": "tgkgz48zc4", "score": 91, "passed": 23,
                              "failed": 1, "summary": "MCP 回传"}}},
    {"jsonrpc": "2.0", "id": 8, "method": "tools/call",
     "params": {"name": "ratsa_submit_feedback",
                "arguments": {"slug": "tgkgz48zc4", "title": "MCP 反馈测试",
                              "detail": "由 MCP 工具提交", "category": "quality",
                              "severity": "low"}}},
]

payload = "".join(json.dumps(m, ensure_ascii=False) + "\n" for m in msgs)
proc = subprocess.run([BIN, "mcp"], input=payload, capture_output=True, text=True)
if proc.returncode != 0:
    print("exit:", proc.returncode, proc.stderr[:500])

for line in proc.stdout.splitlines():
    line = line.strip()
    if not line:
        continue
    m = json.loads(line)
    i = m.get("id")
    if i == 1:
        print("initialize ->", m["result"]["serverInfo"], m["result"]["protocolVersion"])
    elif i == 2:
        print("tools ->", len(m["result"]["tools"]), [t["name"] for t in m["result"]["tools"]])
    elif i:
        res = m["result"]
        text = res["content"][0]["text"]
        is_err = res["isError"]
        print("--- id=%s isError=%s" % (i, is_err))
        print(text[:420])
