#!/usr/bin/env python3
"""Deterministic OpenAI-compatible mock LLM for neuro-harness ACP spikes.

Serves POST {/v1,}/chat/completions (stream and non-stream) and POST {/v1,}/responses
(stream only, minimal). Behaviour:
- last message is a tool result  -> final text quoting the tool output
- user text contains "call tool" -> one tool_call to the first tool whose name contains "echo_meta"
- otherwise                      -> text that echoes how many user turns are in history and the first one

Author: kejiqing
"""
import json
import sys
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

LOG = None


def log(obj):
    if LOG:
        with open(LOG, "a", encoding="utf-8") as fh:
            fh.write(json.dumps(obj, ensure_ascii=False) + "\n")


def _text_of(content):
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        return "".join(p.get("text", "") for p in content if isinstance(p, dict))
    return ""


def decide(messages, tools):
    last = messages[-1] if messages else {}
    if last.get("role") == "tool":
        return {"text": "TOOL_RESULT_SEEN: " + _text_of(last.get("content"))[:200]}
    users = [_text_of(m.get("content")) for m in messages if m.get("role") == "user"]
    current = users[-1] if users else ""
    if "call tool" in current.lower():
        for t in tools or []:
            name = (t.get("function") or {}).get("name") or t.get("name") or ""
            if "echo_meta" in name:
                return {"tool": name, "args": {"note": "spike"}}
    first = users[0][:80] if users else ""
    return {"text": f"MOCK_REPLY user_turns={len(users)} first={first!r}"}


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *_):
        pass

    def _send_sse(self, events):
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Cache-Control", "no-cache")
        self.send_header("Connection", "close")
        self.end_headers()
        for ev in events:
            self.wfile.write(ev.encode("utf-8"))
            self.wfile.flush()
        self.close_connection = True

    def do_GET(self):
        if self.path.rstrip("/").endswith("/models"):
            body = json.dumps({"object": "list", "data": [{"id": "mock-model", "object": "model"}]}).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
        self.send_response(404)
        self.send_header("Content-Length", "0")
        self.end_headers()

    def do_POST(self):
        n = int(self.headers.get("Content-Length") or 0)
        req = json.loads(self.rfile.read(n) or b"{}")
        log({"path": self.path, "req": req})
        if self.path.rstrip("/").endswith("/chat/completions"):
            return self._chat(req)
        if self.path.rstrip("/").endswith("/responses"):
            return self._responses(req)
        self.send_response(404)
        self.send_header("Content-Length", "0")
        self.end_headers()

    def _chat(self, req):
        d = decide(req.get("messages") or [], req.get("tools"))
        cid, created, model = "chatcmpl-mock", int(time.time()), req.get("model", "mock-model")
        usage = {"prompt_tokens": 11, "completion_tokens": 7, "total_tokens": 18}

        def chunk(delta, finish=None, with_usage=False):
            c = {"id": cid, "object": "chat.completion.chunk", "created": created, "model": model,
                 "choices": [{"index": 0, "delta": delta, "finish_reason": finish}]}
            if with_usage:
                c["usage"] = usage
            return "data: " + json.dumps(c) + "\n\n"

        if not req.get("stream"):
            msg = {"role": "assistant", "content": d.get("text")}
            if "tool" in d:
                msg = {"role": "assistant", "content": None, "tool_calls": [{
                    "id": "call_mock_1", "type": "function",
                    "function": {"name": d["tool"], "arguments": json.dumps(d["args"])}}]}
            body = json.dumps({"id": cid, "object": "chat.completion", "created": created, "model": model,
                               "choices": [{"index": 0, "message": msg,
                                            "finish_reason": "tool_calls" if "tool" in d else "stop"}],
                               "usage": usage}).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
        evs = [chunk({"role": "assistant", "content": ""})]
        if "tool" in d:
            evs.append(chunk({"tool_calls": [{"index": 0, "id": "call_mock_1", "type": "function",
                                              "function": {"name": d["tool"], "arguments": ""}}]}))
            evs.append(chunk({"tool_calls": [{"index": 0, "function": {"arguments": json.dumps(d["args"])}}]}))
            evs.append(chunk({}, "tool_calls"))
        else:
            text = d["text"]
            for i in range(0, len(text), 16):
                evs.append(chunk({"content": text[i:i + 16]}))
            evs.append(chunk({}, "stop"))
        evs.append("data: " + json.dumps({"id": cid, "object": "chat.completion.chunk", "created": created,
                                          "model": model, "choices": [], "usage": usage}) + "\n\n")
        evs.append("data: [DONE]\n\n")
        self._send_sse(evs)

    def _responses(self, req):
        items = req.get("input") or []
        msgs = []
        for it in items if isinstance(items, list) else []:
            if it.get("type") == "function_call_output":
                msgs.append({"role": "tool", "content": it.get("output", "")})
            elif it.get("role") in ("user", "assistant"):
                msgs.append({"role": it["role"], "content": it.get("content")})
        tools, ns_of = [], {}
        for t in req.get("tools") or []:
            if t.get("type") == "namespace":
                for sub in t.get("tools") or []:
                    tools.append({"name": sub.get("name")})
                    ns_of[sub.get("name")] = t.get("name")
            else:
                tools.append({"name": t.get("name")})
        d = decide(msgs, tools)
        rid = "resp_mock"
        usage = {"input_tokens": 11, "output_tokens": 7, "total_tokens": 18}

        def ev(name, data):
            data["type"] = name
            return f"event: {name}\ndata: {json.dumps(data)}\n\n"

        evs = [ev("response.created", {"response": {"id": rid, "status": "in_progress"}})]
        if "tool" in d:
            item = {"type": "function_call", "id": "fc_1", "call_id": "call_mock_1", "name": d["tool"],
                    "arguments": json.dumps(d["args"])}
            if d["tool"] in ns_of:
                item["namespace"] = ns_of[d["tool"]]
            evs.append(ev("response.output_item.added", {"output_index": 0, "item": item}))
            evs.append(ev("response.output_item.done", {"output_index": 0, "item": item}))
        else:
            item = {"type": "message", "id": "msg_1", "role": "assistant",
                    "content": [{"type": "output_text", "text": d["text"]}]}
            evs.append(ev("response.output_item.added", {"output_index": 0, "item": {**item, "content": []}}))
            evs.append(ev("response.output_text.delta", {"output_index": 0, "content_index": 0,
                                                         "item_id": "msg_1", "delta": d["text"]}))
            evs.append(ev("response.output_item.done", {"output_index": 0, "item": item}))
        evs.append(ev("response.completed", {"response": {"id": rid, "status": "completed", "usage": usage,
                                                         "output": [item]}}))
        self._send_sse(evs)


def main():
    global LOG
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 18080
    LOG = sys.argv[2] if len(sys.argv) > 2 else None
    ThreadingHTTPServer(("127.0.0.1", port), Handler).serve_forever()


if __name__ == "__main__":
    main()
