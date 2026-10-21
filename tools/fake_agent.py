#!/usr/bin/env python3
"""A stand-in for Claude Code, Codex and Antigravity in the dock gate (WX-4).

    MUSIALIZER_AGENT_CLAUDE=... MUSIALIZER_AGENT_CODEX=... MUSIALIZER_AGENT_AGY=...

The dock starts this with the real CLI's arguments; the protocol is read off
them (`app-server` is Codex, `--print=` is Antigravity, anything else is
Claude). Each user message gets one scripted turn in that protocol's shapes:
some streamed text, one tool call with its result, and a final message that
echoes the user's words, so the gate can see its own message came back.

A message containing the word `ask` makes the Claude and Codex fakes request
permission first and report the answer in their reply ("permission: allow").

It never touches the network, never runs the "tool" it reports, and exits
when its input closes. The real agents are never started by any check.
"""
import json
import sys


def out(obj):
    sys.stdout.write(json.dumps(obj) + "\n")
    sys.stdout.flush()


def lines():
    for raw in sys.stdin:
        raw = raw.strip()
        if raw:
            yield json.loads(raw)


def claude():
    session = "fake-session"
    turn = 0
    stream = lines()
    for message in stream:
        if message.get("type") != "user":
            continue
        turn += 1
        text = message["message"]["content"][0]["text"]
        last = text.splitlines()[-1]
        out({"type": "system", "subtype": "init", "model": "fake-claude", "session_id": session})
        decision = None
        if "ask" in last:
            out({"type": "control_request", "request_id": f"req-{turn}",
                 "request": {"subtype": "can_use_tool", "tool_name": "Bash",
                             "input": {"command": "rm -rf build/fake"}}})
            for reply in stream:
                if reply.get("type") == "control_response":
                    decision = reply["response"]["response"]["behavior"]
                    break
        mid = f"m{turn}"
        out({"type": "stream_event", "event": {"type": "message_start", "message": {"id": mid}}})
        out({"type": "assistant", "message": {"id": mid, "content": [
            {"type": "tool_use", "id": f"t{turn}", "name": "Bash",
             "input": {"command": "tools/headless_run.sh --still 3 fake.png"}}]}})
        out({"type": "user", "message": {"content": [
            {"type": "tool_result", "tool_use_id": f"t{turn}", "content": "wrote fake.png", "is_error": False}]}})
        mid = f"n{turn}"
        reply = f"fake reply to: {last}"
        if decision:
            reply += f" (permission: {decision})"
        out({"type": "stream_event", "event": {"type": "message_start", "message": {"id": mid}}})
        half = len(reply) // 2
        for part in (reply[:half], reply[half:]):
            out({"type": "stream_event", "event": {"type": "content_block_delta", "index": 0,
                                                   "delta": {"type": "text_delta", "text": part}}})
        out({"type": "assistant", "message": {"id": mid, "content": [{"type": "text", "text": reply}]}})
        out({"type": "result", "subtype": "success", "is_error": False, "duration_ms": 1200,
             "total_cost_usd": 0.001, "result": reply, "session_id": session})


def antigravity():
    turn = 0
    for message in lines():
        if message.get("event") != "user":
            continue
        turn += 1
        text = message["message"]["content"]
        last = text.splitlines()[-1]
        out({"event": "init", "conversation_id": "fake-conversation", "init": {"model": "fake-gemini"}})
        out({"event": "step_update", "step_update": {"step_index": 0, "state": "DONE", "step_type": "user_input"}})
        out({"event": "step_update", "step_update": {"step_index": 1, "state": "DONE", "step_type": "tool",
             "tool_name": "run_command", "tool_info": {"parameters": {"CommandLine": "echo fake"}}}})
        reply = f"fake reply to: {last}"
        out({"event": "step_update", "step_update": {"step_index": 2, "state": "ACTIVE",
             "step_type": "agent_response", "text_delta": reply}})
        out({"event": "result", "result": {"conversation_id": "fake-conversation", "status": "SUCCESS",
             "response": reply, "duration_seconds": 1.0, "usage": {"total_tokens": 42}}})


def codex():
    thread = None
    turn = 0
    pending = {}
    for message in lines():
        method = message.get("method")
        if method == "initialize":
            out({"id": message["id"], "result": {"userAgent": "fake"}})
        elif method == "thread/start":
            thread = "fake-thread"
            out({"id": message["id"], "result": {"thread": {"id": thread}, "model": "fake-gpt"}})
        elif method == "turn/start":
            turn += 1
            text = message["params"]["input"][0]["text"]
            last = text.splitlines()[-1]
            out({"id": message["id"], "result": {"turn": {"id": f"turn{turn}"}}})
            out({"method": "turn/started", "params": {"turn": {"id": f"turn{turn}"}}})
            if "ask" in last:
                pending[900 + turn] = last
                out({"id": 900 + turn, "method": "item/commandExecution/requestApproval",
                     "params": {"itemId": f"x{turn}", "command": "rm -rf build/fake"}})
                continue
            finish_codex(turn, last, None)
        elif "id" in message and message["id"] in pending:
            last = pending.pop(message["id"])
            finish_codex(message["id"] - 900, last, message.get("result", {}).get("decision"))


def finish_codex(turn, last, decision):
    out({"method": "item/started", "params": {"item": {"type": "commandExecution", "id": f"x{turn}",
         "commandActions": [{"command": "echo fake"}], "status": "inProgress"}}})
    out({"method": "item/completed", "params": {"item": {"type": "commandExecution", "id": f"x{turn}",
         "commandActions": [{"command": "echo fake"}], "status": "completed", "exitCode": 0,
         "aggregatedOutput": "fake\n"}}})
    reply = f"fake reply to: {last}"
    if decision:
        reply += f" (permission: {decision})"
    out({"method": "item/agentMessage/delta", "params": {"itemId": f"a{turn}", "delta": reply}})
    out({"method": "item/completed", "params": {"item": {"type": "agentMessage", "id": f"a{turn}", "text": reply}}})
    out({"method": "thread/tokenUsage/updated", "params": {"tokenUsage": {"total": {"totalTokens": 42}}}})
    out({"method": "turn/completed", "params": {"turn": {"id": f"turn{turn}"}}})


def main():
    args = sys.argv[1:]
    if "app-server" in args:
        codex()
    elif "--print=" in args:
        antigravity()
    else:
        claude()


if __name__ == "__main__":
    main()
