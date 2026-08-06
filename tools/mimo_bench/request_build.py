"""Request construction. Pure: takes bytes and text, returns a payload dict.

Nothing here reads a credential, opens a socket, or touches the filesystem.
``redact`` replaces the base64 audio with its sha256 so a request body can be
printed, stored beside the result and diffed — the same discipline
``tools/mimo_openrouter.py`` already uses for its ``--dry-run``.

Every payload carries an ``identity`` companion (returned alongside, never
sent) with the model, prompt id and hash, schema version and hash, chunk span
and audio hash. That is what makes a stored result reproducible under §6 of
``docs/ASSIST_PROVIDER_CONTRACTS.md``.
"""

from __future__ import annotations

import base64
import hashlib
from typing import Any

from . import matrix, prompts, schema
from .bench_io import canonical_sha256

OPENROUTER_URL = "https://openrouter.ai/api/v1/chat/completions"
DEFAULT_TIMEOUT = 600.0

# Sent on every request. `require_parameters` makes a provider that cannot do
# structured output refuse rather than silently return prose, which would
# otherwise show up as a schema failure attributed to the model.
PROVIDER_POLICY: dict[str, Any] = {
    "allow_fallbacks": True,
    "require_parameters": True,
}


def _audio_part(audio_bytes: bytes) -> dict[str, Any]:
    return {
        "type": "input_audio",
        "input_audio": {
            "data": base64.b64encode(audio_bytes).decode("ascii"),
            "format": matrix.AUDIO_FORMAT,
        },
    }


def _identity(
    *,
    cell: matrix.Cell,
    call: matrix.Call,
    prompt_id: str,
    prompt_text: str,
    audio_sha256: str | None,
) -> dict[str, Any]:
    return {
        "matrix_version": matrix.MATRIX_VERSION,
        "prompt_registry_version": prompts.PROMPT_REGISTRY_VERSION,
        "cell": cell.id,
        "block": ",".join(cell.blocks),
        "shaping": cell.shaping,
        "chunking": cell.chunking_id,
        "chunk_index": call.chunk.index if call.chunk else None,
        "chunk_count": call.chunk.count if call.chunk else None,
        "chunk_start_seconds": call.chunk.start_seconds if call.chunk else None,
        "chunk_end_seconds": call.chunk.end_seconds if call.chunk else None,
        "audio_seconds": call.audio_seconds,
        "audio_sha256": audio_sha256,
        "prompt_id": prompt_id,
        "prompt_sha256": canonical_sha256(prompt_text),
        "time_frame_policy": prompts.TIME_FRAME_POLICY,
        "schema_version": schema.SCHEMA_VERSION if call.structured else None,
        "schema_sha256": schema.SCHEMA_SHA256 if call.structured else None,
        "requested_model": call.model,
        "temperature": matrix.TEMPERATURE,
        "turn": call.turn,
        "probe_run": call.probe_run,
        "kind": call.kind,
    }


def build_listen_request(
    cell: matrix.Cell, call: matrix.Call, audio_bytes: bytes,
) -> tuple[dict[str, Any], dict[str, Any]]:
    """Arms S0 and S1: one chunk of audio plus the cell's prompt."""
    if call.chunk is None:
        raise ValueError("a listening call must carry a chunk span")
    header = prompts.span_header(
        chunk_index=call.chunk.index,
        chunk_count=call.chunk.count,
        start_seconds=call.chunk.start_seconds,
        end_seconds=call.chunk.end_seconds,
    )
    text = prompts.compose(cell.prompt_id, header)
    payload: dict[str, Any] = {
        "model": call.model,
        "messages": [{
            "role": "user",
            "content": [{"type": "text", "text": text}, _audio_part(audio_bytes)],
        }],
        "provider": dict(PROVIDER_POLICY),
        "temperature": matrix.TEMPERATURE,
    }
    if call.structured:
        payload["response_format"] = schema.response_format()
    identity = _identity(
        cell=cell, call=call, prompt_id=cell.prompt_id, prompt_text=text,
        audio_sha256=hashlib.sha256(audio_bytes).hexdigest(),
    )
    identity["estimated_text_input_tokens"] = int(len(text) / 4) + 1
    return payload, identity


def build_second_turn_request(
    cell: matrix.Cell,
    call: matrix.Call,
    description: str,
    audio_bytes: bytes | None,
) -> tuple[dict[str, Any], dict[str, Any]]:
    """Arms S2a and S2b: the same model reshapes its own prose.

    Chat completions are stateless, so a "second turn" is a whole new request
    that replays the conversation. S2a replays it including the audio part —
    which pays for the audio a second time — and S2b replays only the text.
    Measuring that difference is the point of having both.
    """
    if call.chunk is None:
        raise ValueError("a second-turn call must carry the chunk span it follows")
    header = prompts.span_header(
        chunk_index=call.chunk.index,
        chunk_count=call.chunk.count,
        start_seconds=call.chunk.start_seconds,
        end_seconds=call.chunk.end_seconds,
    )
    first_text = prompts.compose(cell.prompt_id, header)
    if audio_bytes is None:
        first_content: Any = [
            {"type": "text", "text": first_text},
            {"type": "text", "text": prompts.ELIDED_AUDIO_NOTE},
        ]
        audio_sha = None
    else:
        first_content = [{"type": "text", "text": first_text}, _audio_part(audio_bytes)]
        audio_sha = hashlib.sha256(audio_bytes).hexdigest()
    payload: dict[str, Any] = {
        "model": call.model,
        "messages": [
            {"role": "user", "content": first_content},
            {"role": "assistant", "content": description},
            {"role": "user", "content": prompts.REFORMAT_INSTRUCTION},
        ],
        "provider": dict(PROVIDER_POLICY),
        "temperature": matrix.TEMPERATURE,
        "response_format": schema.response_format(),
    }
    identity = _identity(
        cell=cell, call=call, prompt_id=f"{cell.prompt_id}+reformat-turn2",
        prompt_text=first_text + prompts.REFORMAT_INSTRUCTION, audio_sha256=audio_sha,
    )
    identity["description_sha256"] = canonical_sha256(description)
    identity["audio_resent"] = audio_bytes is not None
    identity["reformat_sha256"] = prompts.REFORMAT_SHA256
    identity["estimated_text_input_tokens"] = int(
        (len(first_text) + len(description) + len(prompts.REFORMAT_INSTRUCTION)) / 4) + 1
    return payload, identity


def build_reformatter_request(
    cell: matrix.Cell, call: matrix.Call, description: str,
) -> tuple[dict[str, Any], dict[str, Any]]:
    """Arm S3: a separate, cheap, text-only model converts the prose.

    No audio, no conversation history, and no knowledge of the source: the
    reformatter sees exactly the description text and the schema. That is what
    makes the determinism probe meaningful — every one of its runs has a
    byte-identical input.
    """
    text = prompts.reformat_standalone(description)
    payload: dict[str, Any] = {
        "model": call.model,
        "messages": [{"role": "user", "content": text}],
        "provider": dict(PROVIDER_POLICY),
        "temperature": matrix.TEMPERATURE,
        "response_format": schema.response_format(),
    }
    identity = _identity(
        cell=cell, call=call, prompt_id="reformat-standalone", prompt_text=text,
        audio_sha256=None,
    )
    identity["description_sha256"] = canonical_sha256(description)
    identity["reformat_sha256"] = prompts.REFORMAT_SHA256
    identity["estimated_text_input_tokens"] = int(len(text) / 4) + 1
    return payload, identity


def redact(payload: dict[str, Any]) -> dict[str, Any]:
    """A printable copy: base64 audio replaced by its own sha256 and length."""
    import json

    copy = json.loads(json.dumps(payload))
    for message in copy.get("messages", []):
        content = message.get("content")
        if not isinstance(content, list):
            continue
        for part in content:
            if isinstance(part, dict) and part.get("type") == "input_audio":
                data = part["input_audio"].get("data") or ""
                raw = base64.b64decode(data) if data else b""
                part["input_audio"]["data"] = (
                    f"<base64 omitted; {len(raw)} bytes; "
                    f"sha256={hashlib.sha256(raw).hexdigest()}>")
    return copy


def request_dump(
    payload: dict[str, Any], identity: dict[str, Any],
) -> dict[str, Any]:
    return {
        "dry_run": True,
        "url": OPENROUTER_URL,
        "timeout_seconds": DEFAULT_TIMEOUT,
        "headers": {
            "Authorization": "Bearer <redacted; read from OPENROUTER_API_KEY at send time>",
            "Content-Type": "application/json",
        },
        "identity": identity,
        "payload": redact(payload),
    }


def completion_text(response: dict[str, Any]) -> str:
    """The assistant text, tolerating both string and content-part shapes."""
    try:
        content = response["choices"][0]["message"]["content"]
    except (KeyError, IndexError, TypeError) as error:
        raise ValueError("response carries no completion") from error
    if isinstance(content, str):
        return content.strip()
    if isinstance(content, list):
        return "".join(
            part.get("text", "") for part in content if isinstance(part, dict)
        ).strip()
    raise ValueError("completion content is not text")


def response_metadata(response: dict[str, Any]) -> dict[str, Any]:
    """What §6 calls observed provenance: what actually served the request."""
    return {
        "response_id": response.get("id"),
        "model_served": response.get("model"),
        "provider_served": response.get("provider"),
        "created": response.get("created"),
        "usage": response.get("usage"),
        "finish_reason": (
            (response.get("choices") or [{}])[0].get("finish_reason")
            if isinstance(response.get("choices"), list) else None
        ),
    }


__all__ = [
    "DEFAULT_TIMEOUT",
    "OPENROUTER_URL",
    "PROVIDER_POLICY",
    "build_listen_request",
    "build_reformatter_request",
    "build_second_turn_request",
    "completion_text",
    "redact",
    "request_dump",
    "response_metadata",
]
