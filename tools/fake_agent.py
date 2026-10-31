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
A message containing `state?` adds what the fake was told about its model,
effort and access to the reply, as each real CLI would hold it: Claude's from
its flags and control requests, Codex's from its thread and turn requests,
Antigravity's from its flags. `agy models` is answered as a listing.

Words for the chat's pictures, each read from the fake's working directory,
where the gate wrote them with PIL: `picture` names `sheet.png` in the reply on
every protocol; on Claude `broken` names a file that is no picture, `unwritten`
one that does not exist, `repeat` names the sheet in a tool call, its output and
the reply, `rewrite` shows the sheet and changes its mtime before naming it
again, `half` pauses the stream after `sheet.png`, the head of a longer name,
and `listing` makes a tool print nine names. On Codex `viewed` is an
`imageView` item, which holds the path and nothing else.

It never touches the network, never runs the "tool" it reports, and exits
when its input closes. The real agents are never started by any check.
"""
import json
import os
import sys
import time

ARGS = sys.argv[1:]


def flag(name, default=None):
    return ARGS[ARGS.index(name) + 1] if name in ARGS else default


CLAUDE_MODELS = [
    {"value": "default", "resolvedModel": "fake-opus", "displayName": "Default (recommended)",
     "description": "Fake Opus, the default", "supportedEffortLevels": ["low", "high", "max"],
     "supportsAutoMode": True},
    {"value": "fake-opus", "resolvedModel": "fake-opus", "displayName": "Fake Opus",
     "description": "For fake complex work", "supportedEffortLevels": ["low", "high", "max"],
     "supportsAutoMode": True},
    {"value": "fake-haiku", "resolvedModel": "fake-haiku", "displayName": "Fake Haiku",
     "description": "No levels, no auto mode"},
    {"value": "fake-capped", "resolvedModel": "fake-capped", "displayName": "Fake Capped",
     "description": "Applies high when asked for max", "supportedEffortLevels": ["low", "max"],
     "supportsAutoMode": True},
    {"value": "fake-refused", "resolvedModel": "fake-refused", "displayName": "Fake Refused",
     "description": "The fake refuses this one", "supportedEffortLevels": ["low"],
     "supportsAutoMode": True},
]

CODEX_MODELS = [
    {"id": "fake-sol", "model": "fake-sol", "displayName": "Fake Sol", "description": "A fake flagship",
     "hidden": False, "isDefault": True, "defaultReasoningEffort": "low",
     "supportedReasoningEfforts": [{"reasoningEffort": e, "description": e} for e in ("low", "high", "ultra")]},
    {"id": "fake-luna", "model": "fake-luna", "displayName": "Fake Luna", "description": "A fake small one",
     "hidden": False, "isDefault": False, "defaultReasoningEffort": "medium",
     "supportedReasoningEfforts": [{"reasoningEffort": e, "description": e} for e in ("medium", "max")]},
    {"id": "fake-hidden", "model": "fake-hidden", "displayName": "Fake Hidden", "description": "",
     "hidden": True, "isDefault": False, "defaultReasoningEffort": "low", "supportedReasoningEfforts": []},
]

AGY_MODELS = """Fetching available models...
fake-argon-medium\tFake Argon (Medium)
fake-argon-high\tFake Argon (High)
fake-argon-low\tFake Argon (Low)
fake-oss-medium\tFake OSS (Medium)
"""


def out(obj):
    sys.stdout.write(json.dumps(obj) + "\n")
    sys.stdout.flush()


def lines():
    for raw in sys.stdin:
        raw = raw.strip()
        if raw:
            yield json.loads(raw)


def picture_words(last):
    return " Here is the contact sheet: `sheet.png`." if "picture" in last else ""


def claude():
    session = "fake-session"
    turn = 0
    stream = lines()
    state = {"model": flag("--model", "default"), "effort": flag("--effort", "high"),
             "mode": flag("--permission-mode", "default")}

    def resolved():
        return next((m["resolvedModel"] for m in CLAUDE_MODELS if m["value"] == state["model"]), state["model"])

    def control(message):
        request = message["request"]
        kind = request["subtype"]
        body = None
        error = None
        if kind == "initialize":
            body = {"models": CLAUDE_MODELS, "current_permission_mode": state["mode"]}
        elif kind == "get_settings":
            body = {"effective": {}, "applied": {"model": resolved(), "effort": state["effort"]}}
        elif kind == "set_model":
            if request["model"] == "fake-refused":
                error = "fake-refused is not available on this plan"
            else:
                state["model"] = request["model"]
                body = {"model": resolved()}
        elif kind == "set_permission_mode":
            if request["mode"] == "bypassPermissions" and "--allow-dangerously-skip-permissions" not in ARGS:
                error = "the session was not launched to allow bypassPermissions"
            else:
                state["mode"] = request["mode"]
                body = {"mode": request["mode"]}
        elif kind == "apply_flag_settings":
            state["effort"] = request["settings"]["effortLevel"]
            if state["model"] == "fake-capped" and state["effort"] == "max":
                state["effort"] = "high"
            body = {}
        else:
            return
        reply = {"request_id": message["request_id"]}
        if error:
            reply.update(subtype="error", error=error)
        else:
            reply.update(subtype="success", response=body)
        out({"type": "control_response", "response": reply})

    def tool(tool_id, command, result, during=None):
        out({"type": "assistant", "message": {"id": f"p{tool_id}", "content": [
            {"type": "tool_use", "id": tool_id, "name": "Bash", "input": {"command": command}}]}})
        if during:
            during()
        out({"type": "user", "message": {"content": [
            {"type": "tool_result", "tool_use_id": tool_id, "content": result, "is_error": False}]}})

    def touch_sheet():
        # Time for the window to read the mention above before the file moves.
        time.sleep(0.6)
        status = os.stat("sheet.png")
        os.utime("sheet.png", (status.st_atime, status.st_mtime + 10))

    for message in stream:
        if message.get("type") == "control_request":
            control(message)
            continue
        if message.get("type") != "user":
            continue
        turn += 1
        text = message["message"]["content"][0]["text"]
        last = text.splitlines()[-1]
        out({"type": "system", "subtype": "init", "model": resolved(), "session_id": session})
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
        if "repeat" in last:
            tool(f"r{turn}", "open sheet.png", "showing ./sheet.png")
        if "rewrite" in last:
            out({"type": "assistant", "message": {"id": f"w{turn}", "content": [
                {"type": "text", "text": "first cut: sheet.png"}]}})
            tool(f"w{turn}", "render --again", "done", touch_sheet)
        if "listing" in last:
            tool(f"l{turn}", "ls", " ".join(f"s{n}.png" for n in range(9)))
        mid = f"n{turn}"
        reply = f"fake reply to: {last}"
        if decision:
            reply += f" (permission: {decision})"
        if "state?" in last:
            reply += f" [{resolved()}/{state['effort']}/{state['mode']}]"
        reply += picture_words(last)
        if "broken" in last:
            reply += " Here is `not-image.png`."
        if "unwritten" in last:
            reply += " I will write later.png next."
        if "repeat" in last or "rewrite" in last:
            reply += " Here is sheet.png."
        out({"type": "stream_event", "event": {"type": "message_start", "message": {"id": mid}}})
        half = len(reply) // 2
        parts = (reply[:half], reply[half:])
        if "half" in last:
            reply = "Here is sheet.png-old.png."
            parts = (reply[:len("Here is sheet.png")], reply[len("Here is sheet.png"):])
        for part in parts:
            if part is parts[1] and "half" in last:
                time.sleep(0.5)
            out({"type": "stream_event", "event": {"type": "content_block_delta", "index": 0,
                                                   "delta": {"type": "text_delta", "text": part}}})
        out({"type": "assistant", "message": {"id": mid, "content": [{"type": "text", "text": reply}]}})
        out({"type": "result", "subtype": "success", "is_error": False, "duration_ms": 1200,
             "total_cost_usd": 0.001, "result": reply, "session_id": session})


def antigravity():
    turn = 0
    if flag("--mode") == "auto":
        sys.stderr.write("warning: --mode auto is not available for this account; "
                         "continuing in the default mode.\n")
        sys.stderr.flush()
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
        if "state?" in last:
            skip = "--dangerously-skip-permissions" in ARGS
            reply += f" [{flag('--model', '-')}/{flag('--mode', '-')}/{'skip' if skip else 'ask'}]"
        reply += picture_words(last)
        out({"event": "step_update", "step_update": {"step_index": 2, "state": "ACTIVE",
             "step_type": "agent_response", "text_delta": reply}})
        out({"event": "result", "result": {"conversation_id": "fake-conversation", "status": "SUCCESS",
             "response": reply, "duration_seconds": 1.0, "usage": {"total_tokens": 42}}})


def codex():
    thread = None
    turn = 0
    pending = {}
    state = {"model": "fake-sol", "effort": "low", "access": "-"}
    for message in lines():
        method = message.get("method")
        if method == "initialize":
            out({"id": message["id"], "result": {"userAgent": "fake"}})
        elif method == "model/list":
            out({"id": message["id"], "result": {"data": CODEX_MODELS, "nextCursor": None}})
        elif method == "thread/start":
            thread = "fake-thread"
            params = message["params"]
            state["model"] = params.get("model") or state["model"]
            out({"id": message["id"], "result": {"thread": {"id": thread}, "model": state["model"],
                                                 "reasoningEffort": state["effort"]}})
        elif method == "turn/start":
            turn += 1
            params = message["params"]
            text = params["input"][0]["text"]
            last = text.splitlines()[-1]
            state["model"] = params.get("model") or state["model"]
            state["effort"] = params.get("effort") or state["effort"]
            state["access"] = "+".join([params.get("approvalPolicy", "-"), params.get("approvalsReviewer", "-"),
                                        params.get("sandboxPolicy", {}).get("type", "-")])
            if "state?" in last:
                last += f" [{state['model']}/{state['effort']}/{state['access']}]"
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
    if "viewed" in last:
        out({"method": "item/completed", "params": {"item": {"type": "imageView", "id": f"v{turn}",
             "path": os.path.abspath("sheet.png")}}})
    reply = f"fake reply to: {last}"
    if decision:
        reply += f" (permission: {decision})"
    reply += picture_words(last)
    out({"method": "item/agentMessage/delta", "params": {"itemId": f"a{turn}", "delta": reply}})
    out({"method": "item/completed", "params": {"item": {"type": "agentMessage", "id": f"a{turn}", "text": reply}}})
    out({"method": "thread/tokenUsage/updated", "params": {"tokenUsage": {"total": {"totalTokens": 42}}}})
    out({"method": "turn/completed", "params": {"turn": {"id": f"turn{turn}"}}})


def main():
    args = ARGS
    if args[:1] == ["models"]:
        sys.stdout.write(AGY_MODELS)
    elif "app-server" in args:
        codex()
    elif "--print=" in args:
        antigravity()
    else:
        claude()


if __name__ == "__main__":
    main()
