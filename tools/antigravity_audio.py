#!/usr/bin/env python3
"""Explicitly invoked audio research through Google's authenticated ACP server.

No playback, API keys, project edits, or client tools. Reuses the selected
profile through the official executable without reading/copying its credentials.
Network access occurs only in ask(); discovery is filesystem-only.
"""
from __future__ import annotations

import argparse
import asyncio
import base64
import hashlib
import json
import re
import os
from pathlib import Path
import subprocess
import tempfile
import time


PROMPT = """Listen to the attached short audio clip directly. Do not use tools.
Transcribe only audible sung/spoken words, including repetitions and backing
vocals. Never invent production directions as lyrics. Do not infer words from
file names or a song you remember. Return JSON only:
{"lines":[{"text":"exact words heard", "start_seconds":0.0,
"end_seconds":1.0,"uncertain":false}],"notes":[]}
Times are relative to this clip, with zero at its first sample. Split at natural
lyric phrases. Start is first audible vocal phoneme; end is last audible vocal
phoneme, including sustained vowels but excluding reverb/instrumental tails.
If a phrase is clipped, say so in notes. Mark uncertain words honestly. An
instrumental clip has an empty lines array. Listen to the audio; do not estimate
timings from reading speed. No Markdown fences or surrounding prose.
"""


MODELS = ("gemini-3.8-flash-high", "gemini-3.8-flash-medium", "gemini-3.8-flash-low")


def discover(*, server=None, harness=None, profile=None, environ=None) -> dict:
    """Resolve official T3 runtime paths without launching or reading tokens.

    Explicit paths win and fail closed. Multiple authenticated profiles require
    a selection; the helper never guesses which account the user intended.
    """
    env = os.environ if environ is None else environ
    home = Path(env.get("HOME", ""))
    if not home.is_absolute():
        raise ValueError("Antigravity discovery needs HOME or explicit paths")
    selected = {}
    for key, value in (("server", server), ("harness", harness), ("profile", profile)):
        value = value or env.get("MUSIALIZER_ANTIGRAVITY_" + key.upper())
        if value:
            selected[key] = Path(value).expanduser().resolve()
    if "server" not in selected:
        runtime = home / ".t3/tools/antigravity-acp/linux-x64"
        active = runtime / "active.json"
        if not active.is_file() or active.stat().st_size > 4096:
            raise ValueError("Install the Antigravity ACP runtime in T3 Code, or set antigravity_server")
        release = json.loads(active.read_text()).get("releaseId", "")
        if not re.fullmatch(r"[a-zA-Z0-9._-]+", release) or release in (".", ".."):
            raise ValueError("Invalid T3 Antigravity release identifier")
        selected["server"] = runtime / "versions" / release / "agy_acp_server.par"
    selected.setdefault("harness", selected["server"].parent / "localharness_external")
    if "profile" not in selected:
        profiles = sorted((home / ".t3/userdata/providers/antigravity").glob("*/antigravity-acp/acp_token.json"))
        if len(profiles) != 1:
            raise ValueError("Select antigravity_profile: expected exactly one authenticated T3 Antigravity profile")
        selected["profile"] = profiles[0].parent.parent
    for key in ("server", "harness"):
        if not selected[key].is_file() or not os.access(selected[key], os.X_OK):
            raise ValueError(f"Antigravity {key} is not executable: {selected[key]}")
    if not (selected["profile"] / "antigravity-acp/acp_token.json").is_file():
        raise ValueError("Selected Antigravity profile has no saved sign-in; authenticate it in T3 Code")
    return selected


def inventory(**kwargs) -> dict:
    """A local discovery fact, never a claim that OAuth is still valid."""
    try:
        paths = discover(**kwargs)
        return {"state": "available", "path": str(paths["server"]),
                "version": None, "model_sha256": None, "model_path": None, "gpu_ready": None,
                "language_support": "Audio transcription; model checked at Start",
                "remediation": "Runtime and saved sign-in found; authentication is checked only when you start an audio job.",
                "paths": {key: str(value) for key, value in paths.items()}}
    except (OSError, ValueError) as error:
        return {"state": "unavailable", "path": None, "version": None,
                "model_sha256": None, "model_path": None, "gpu_ready": None, "language_support": "Audio transcription",
                "remediation": str(error)}


def child_environment(profile: Path, harness: Path) -> dict[str, str]:
    blocked = ("API_KEY", "TOKEN", "SECRET", "PASSWORD", "CREDENTIAL",
               "GOOGLE_CLOUD", "GCLOUD", "CLOUDSDK", "GOOGLE_GENAI",
               "AGY_ACP_CCPA", "AGY_ACP_ENABLE_OAUTH")
    env = {k: v for k, v in os.environ.items()
           if not any(marker in k.upper() for marker in blocked)}
    env.update(GEMINI_HOME=str(profile), AGY_ACP_FORCE_FILE_STORAGE="1",
               ANTIGRAVITY_HARNESS_PATH=str(harness), BROWSER="/bin/true",
               ELECTRON_RUN_AS_NODE="1", PYTHONUNBUFFERED="1")
    return env


class Client:
    def __init__(self, process: asyncio.subprocess.Process):
        self.process = process
        self.counter = 0
        self.events: list[dict] = []
        self.messages: list[str] = []
        self.received_bytes = 0

    async def write(self, value: dict) -> None:
        self.process.stdin.write(json.dumps(value).encode() + b"\n")
        await self.process.stdin.drain()

    async def call(self, method: str, params: dict) -> dict:
        self.counter += 1
        request_id = self.counter
        await self.write(dict(jsonrpc="2.0", id=request_id,
                              method=method, params=params))
        while True:
            raw = await self.process.stdout.readline()
            if not raw:
                raise RuntimeError(f"ACP exited during {method}")
            self.received_bytes += len(raw)
            if self.received_bytes > 4 * 1024 * 1024:
                raise RuntimeError("ACP response exceeds 4 MiB")
            try:
                message = json.loads(raw)
            except ValueError:
                continue
            if not isinstance(message, dict):
                raise RuntimeError("ACP message is not an object")
            if "method" in message and "id" in message:
                if message["method"] == "session/request_permission":
                    reply = {"result": {"outcome": {"outcome": "cancelled"}}}
                else:
                    reply = {"error": {"code": -32601,
                                      "message": "Client tools disabled"}}
                await self.write(dict(jsonrpc="2.0", id=message["id"], **reply))
                self.events.append({"denied_tool": message["method"]})
            elif message.get("method") == "session/update":
                update = message.get("params", {}).get("update", {})
                self.events.append(update)
                if (method == "session/prompt"
                        and message.get("params", {}).get("sessionId") == params.get("sessionId")
                        and update.get("sessionUpdate") == "agent_message_chunk"):
                    content = update.get("content", {})
                    if content.get("type") == "text":
                        self.messages.append(content.get("text", ""))
            elif message.get("id") == request_id:
                if "error" in message:
                    raise RuntimeError(str(message["error"]))
                return message.get("result", {})


class AntigravityUsageLimit(RuntimeError):
    """The provider requires a quota reset before another audio request."""


class AntigravityNoResponse(RuntimeError):
    """The request ended without a transcript or a usable provider status."""


def completed_response(result: dict, messages: list[str]) -> str:
    """Provider status prose is not a malformed transcript to resend audio for."""
    if result.get("stopReason") != "end_turn":
        raise RuntimeError(f"ACP did not complete the audio request: {result.get('stopReason')}")
    response = "".join(messages)
    if not response.strip():
        raise AntigravityNoResponse("Antigravity ended the audio request without a response. "
                           "No transcript was produced; retry later to resume completed clips.")
    if response.lstrip().startswith("Usage Limit Reached") and "quota" in response.lower():
        reset = re.search(r"Your limit will reset in ([0-9a-zA-Z ,]+)\.", response)
        detail = f" It reports a reset in {reset.group(1)}." if reset else ""
        raise AntigravityUsageLimit("Antigravity usage quota reached." + detail +
                           " Completed clips are preserved; retry after the reset.")
    return response


async def _ask(args: argparse.Namespace, clip: bytes, prompt: str) -> dict:
    env = child_environment(args.profile, args.harness)
    with tempfile.TemporaryDirectory(prefix="musializer-antigravity-") as cwd:
        process = await asyncio.create_subprocess_exec(
            str(args.server), "--uid=", stdin=asyncio.subprocess.PIPE,
            stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.DEVNULL,
            env=env, cwd=cwd, limit=4 * 1024 * 1024)
        client = Client(process)
        try:
            initialized = await client.call("initialize", {
                "protocolVersion": 1,
                "clientCapabilities": {"fs": {"readTextFile": False,
                                                "writeTextFile": False},
                                       "terminal": False},
                "clientInfo": {"name": "musializer-lyrics-assist",
                               "version": "1"}})
            if initialized.get("agentInfo", {}).get("name") != "antigravity-acp":
                raise RuntimeError("Not the official Antigravity ACP agent")
            if not initialized.get("agentCapabilities", {}).get(
                    "promptCapabilities", {}).get("audio"):
                raise RuntimeError("Server does not advertise audio input")
            await client.call("authenticate", {"methodId": "oauth-personal"})
            session = await client.call("session/new", {"cwd": cwd, "mcpServers": []})
            available = {m["modelId"] for m in session.get("models", {}).get(
                "availableModels", [])}
            if args.model not in available:
                raise RuntimeError(f"Requested model unavailable: {args.model}")
            sid = session["sessionId"]
            selection = await client.call("session/set_config_option", {
                "sessionId": sid, "configId": "model", "value": args.model})
            selected = next((option.get("currentValue") for option in selection.get("configOptions", [])
                             if option.get("id") == "model"), None)
            if selected != args.model:
                raise RuntimeError("Antigravity did not confirm the exact selected model")
            result = await client.call("session/prompt", {
                "sessionId": sid, "prompt": [
                    {"type": "text", "text": prompt},
                    {"type": "audio", "mimeType": "audio/wav",
                     "data": base64.b64encode(clip).decode()}]})
            response = completed_response(result, client.messages)
            return {"agent": initialized["agentInfo"], "model": args.model,
                    "selection": selection, "session_id": sid,
                    "result": result, "response": response,
                    "events": client.events}
        finally:
            if process.returncode is None:
                process.terminate()
                try:
                    await asyncio.wait_for(process.wait(), 3)
                except asyncio.TimeoutError:
                    process.kill()
                    await process.wait()


async def ask(args: argparse.Namespace, clip: bytes, prompt: str) -> dict:
    # wait_for also works in the supported Python 3.10 runtime. On timeout it
    # waits for _ask's finally block to terminate and reap its ACP child.
    try:
        return await asyncio.wait_for(_ask(args, clip, prompt), args.timeout)
    except asyncio.TimeoutError as error:
        raise RuntimeError(
            f"Antigravity audio request did not finish within {args.timeout:g} seconds. "
            "Retry the job to resume its completed clips.") from error


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("audio", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--server", type=Path, required=True)
    parser.add_argument("--harness", type=Path, required=True)
    parser.add_argument("--profile", type=Path, required=True)
    parser.add_argument("--model", default="gemini-3.8-flash-high")
    parser.add_argument("--start", type=float, default=0)
    parser.add_argument("--seconds", type=float, default=12)
    parser.add_argument("--timeout", type=float, default=180)
    parser.add_argument("--prompt-file", type=Path)
    args = parser.parse_args()
    if not 0 < args.seconds <= 30 or args.start < 0:
        parser.error("Use a positive clip of at most 30 seconds and nonnegative start")
    prompt = args.prompt_file.read_text() if args.prompt_file else PROMPT
    # Decode into a pipe: real PCM, no output device, no metadata sent upstream.
    clip = subprocess.check_output([
        "ffmpeg", "-v", "error", "-i", str(args.audio), "-ss", str(args.start),
        "-t", str(args.seconds), "-vn", "-ac", "1", "-ar", "16000",
        "-map_metadata", "-1", "-f", "wav", "pipe:1"], timeout=60)
    started = time.monotonic()
    report = asyncio.run(ask(args, clip, prompt))
    report.update(audio_sha256=hashlib.sha256(args.audio.read_bytes()).hexdigest(),
                  clip_sha256=hashlib.sha256(clip).hexdigest(),
                  clip_start_seconds=args.start, clip_duration_seconds=args.seconds,
                  prompt=prompt, runtime_seconds=time.monotonic() - started)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    temporary = args.output.with_suffix(".tmp")
    temporary.write_text(json.dumps(report, indent=2) + "\n")
    temporary.replace(args.output)
    print(report["response"])


if __name__ == "__main__":
    main()
