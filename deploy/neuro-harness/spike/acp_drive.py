#!/usr/bin/env python3
"""Spike driver: run ONE ACP turn against an agent subprocess and record raw traffic.

usage: acp_drive.py --state STATE.json --record OUT.ndjson --cwd DIR --prompt TEXT
                    [--mcp name=cmd,arg1,arg2 ...] -- agent_cmd [args...]

First call creates a session (session/new) and stores sessionId in STATE; later calls reuse it
via session/resume (preferred) or session/load. Every line in/out is appended to OUT as
{"dir": "in"|"out", "msg": ...}. Permission requests are auto-approved. Author: kejiqing
"""
import argparse
import json
import os
import subprocess
import sys
import threading
import time


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--state", required=True)
    ap.add_argument("--record", required=True)
    ap.add_argument("--cwd", required=True)
    ap.add_argument("--prompt", required=True)
    ap.add_argument("--mcp", action="append", default=[])
    ap.add_argument("--timeout", type=float, default=120)
    ap.add_argument("cmd", nargs=argparse.REMAINDER)
    a = ap.parse_args()
    cmd = a.cmd[1:] if a.cmd and a.cmd[0] == "--" else a.cmd

    mcp_servers = []
    for spec in a.mcp:
        name, rest = spec.split("=", 1)
        parts = rest.split(",")
        mcp_servers.append({"name": name, "command": parts[0], "args": parts[1:],
                            "env": [{"name": "MOCK_MCP_LOG", "value": os.environ.get("MOCK_MCP_LOG", "")}]})

    rec = open(a.record, "a", encoding="utf-8")
    proc = subprocess.Popen(cmd, cwd=a.cwd, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                            stderr=open(a.record + ".stderr", "a"), text=True, bufsize=1)
    pending, responses, lock = {}, {}, threading.Condition()
    next_id = [0]

    def send(obj):
        rec.write(json.dumps({"dir": "out", "t": time.time(), "msg": obj}) + "\n")
        rec.flush()
        proc.stdin.write(json.dumps(obj) + "\n")
        proc.stdin.flush()

    def request(method, params):
        next_id[0] += 1
        rid = next_id[0]
        send({"jsonrpc": "2.0", "id": rid, "method": method, "params": params})
        deadline = time.time() + a.timeout
        with lock:
            while rid not in responses:
                if not lock.wait(timeout=max(0.1, deadline - time.time())) and time.time() > deadline:
                    raise TimeoutError(method)
            return responses.pop(rid)

    def reader():
        for line in proc.stdout:
            line = line.strip()
            if not line:
                continue
            try:
                msg = json.loads(line)
            except json.JSONDecodeError:
                rec.write(json.dumps({"dir": "in-raw", "t": time.time(), "line": line}) + "\n")
                continue
            rec.write(json.dumps({"dir": "in", "t": time.time(), "msg": msg}) + "\n")
            rec.flush()
            if "method" in msg and "id" in msg:
                if msg["method"] == "session/request_permission":
                    opts = msg["params"].get("options", [])
                    pick = next((o for o in opts if o.get("kind") == "allow_once"), None) or \
                        next((o for o in opts if o.get("kind", "").startswith("allow")), None)
                    outcome = {"outcome": "selected", "optionId": pick["optionId"]} if pick else {"outcome": "cancelled"}
                    send({"jsonrpc": "2.0", "id": msg["id"], "result": {"outcome": outcome}})
                else:
                    send({"jsonrpc": "2.0", "id": msg["id"],
                          "error": {"code": -32601, "message": "method not supported by client"}})
            elif "id" in msg:
                with lock:
                    responses[msg["id"]] = msg
                    lock.notify_all()

    threading.Thread(target=reader, daemon=True).start()
    t0 = time.time()
    init = request("initialize", {"protocolVersion": 1,
                                  "clientCapabilities": {"fs": {"readTextFile": False, "writeTextFile": False},
                                                         "terminal": False},
                                  "clientInfo": {"name": "neuro-spike", "version": "0.0.1"}})
    caps = (init.get("result") or {}).get("agentCapabilities") or {}
    state = json.load(open(a.state)) if os.path.exists(a.state) else {}
    sid = state.get("sessionId")
    t1 = time.time()
    if sid:
        if (caps.get("sessionCapabilities") or {}).get("resume") is not None:
            r = request("session/resume", {"sessionId": sid, "cwd": a.cwd, "mcpServers": mcp_servers})
            mode = "resume"
        elif caps.get("loadSession"):
            r = request("session/load", {"sessionId": sid, "cwd": a.cwd, "mcpServers": mcp_servers})
            mode = "load"
        else:
            raise SystemExit("agent supports neither resume nor load")
    else:
        r = request("session/new", {"cwd": a.cwd, "mcpServers": mcp_servers})
        sid = (r.get("result") or {}).get("sessionId")
        mode = "new"
        json.dump({"sessionId": sid}, open(a.state, "w"))
    t2 = time.time()
    pr = request("session/prompt", {"sessionId": sid, "prompt": [{"type": "text", "text": a.prompt}]})
    t3 = time.time()
    summary = {"mode": mode, "sessionId": sid, "session_setup_error": r.get("error"),
               "prompt_result": pr.get("result"), "prompt_error": pr.get("error"),
               "ms": {"initialize": int((t1 - t0) * 1000), "session": int((t2 - t1) * 1000),
                      "prompt": int((t3 - t2) * 1000)},
               "agentCapabilities": caps}
    rec.write(json.dumps({"dir": "summary", "msg": summary}) + "\n")
    rec.close()
    print(json.dumps(summary, ensure_ascii=False))
    proc.stdin.close()
    try:
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        proc.kill()


if __name__ == "__main__":
    sys.exit(main())
