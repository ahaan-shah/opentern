#!/usr/bin/env python3
# A stand-in for the `claude` CLI in tests: speaks the stream-json protocol
# for one turn. It calls the app's get_chart and open_symbol tools, asks for
# permission once, streams a reply and reports usage.
import json, sys

def out(o):
    sys.stdout.write(json.dumps(o) + "\n")
    sys.stdout.flush()

def wait_for(request_id):
    for line in sys.stdin:
        d = json.loads(line)
        if d.get("type") == "control_response" and d["response"]["request_id"] == request_id:
            return d["response"]["response"]
    sys.exit(0)

args = sys.argv[1:]
if args[:2] == ["auth", "status"]:
    out({"loggedIn": True})
    sys.exit(0)
for line in sys.stdin:
    d = json.loads(line)
    if d.get("type") != "user":
        continue
    kinds = ",".join(b["type"] for b in d["message"]["content"])
    attached = "attached_file" in json.dumps(d["message"]["content"])
    out({"type": "system", "subtype": "init", "session_id": "fake-session", "model": "fake-model"})
    out({"type": "assistant", "parent_tool_use_id": None, "message": {"content": [
        {"type": "tool_use", "id": "t1", "name": "mcp__opentern__get_chart", "input": {}}],
        "usage": {"input_tokens": 3, "cache_read_input_tokens": 1000, "output_tokens": 7}}})
    out({"type": "control_request", "request_id": "m1", "request": {"subtype": "mcp_message", "server_name": "opentern",
         "message": {"jsonrpc": "2.0", "id": 5, "method": "tools/call", "params": {"name": "get_chart", "arguments": {}}}}})
    chart = json.loads(wait_for("m1")["mcp_response"]["result"]["content"][0]["text"])
    out({"type": "user", "parent_tool_use_id": None, "message": {"content": [
        {"type": "tool_result", "tool_use_id": "t1", "content": [{"type": "text", "text": chart["symbol"]}], "is_error": False}]}})
    out({"type": "control_request", "request_id": "p1", "request": {"subtype": "can_use_tool", "tool_name": "Write",
         "input": {"file_path": "note.txt", "content": "leaf"}, "description": "note.txt", "permission_suggestions": []}})
    allowed = wait_for("p1")["behavior"]
    out({"type": "control_request", "request_id": "m2", "request": {"subtype": "mcp_message", "server_name": "opentern",
         "message": {"jsonrpc": "2.0", "id": 6, "method": "tools/call", "params": {"name": "open_symbol", "arguments": {"symbol": "ETHUSDT"}}}}})
    opened = wait_for("m2")["mcp_response"]["result"]["content"][0]["text"]
    out({"type": "rate_limit_event", "rate_limit_info": {"unifiedWindows": {"five_hour": {"utilization": 0.25, "resetsAt": 1800000000}}}})
    out({"type": "stream_event", "parent_tool_use_id": None, "event": {"type": "content_block_start", "content_block": {"type": "text"}}})
    for piece in ["The chart was ", chart["symbol"], ". Write was ", allowed, ". ", opened, " Blocks ", kinds, " File ", str(attached), " Mode ", args[args.index("--permission-mode") + 1]]:
        out({"type": "stream_event", "parent_tool_use_id": None, "event": {"type": "content_block_delta", "delta": {"type": "text_delta", "text": piece}}})
    out({"type": "result", "subtype": "success", "is_error": False, "total_cost_usd": 0.01,
         "usage": {"input_tokens": 3, "cache_read_input_tokens": 1000, "cache_creation_input_tokens": 50, "output_tokens": 7}})
