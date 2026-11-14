"""Projected cost of the matrix, printed before anything is sent.

Two numbers here are assumptions rather than measurements and are labelled as
such everywhere they appear:

* **Audio tokens per second.** OpenRouter bills audio as input tokens by
  duration, and the rate is not published per model. The projection is a
  *bracket* over a plausible range rather than a single number, so a run
  cannot come in above a figure the operator was shown. After the first live
  run the recorded ``usage`` gives the real rate and the bracket collapses;
  ``calibrate_from_usage`` does that arithmetic.
* **Output tokens per call.** A per-arm constant, chosen high. A free-text
  description is the expensive one.

Prices are per million tokens and are editable in one table, because they are
the operator's contract with the provider and not a property of this harness.
"""

from __future__ import annotations

import math
from dataclasses import dataclass
from typing import Any, Iterable, Sequence

from . import matrix, prompts, schema
from .bench_io import Track, track_by_slug

COST_MODEL_VERSION = "musializer.mimo-bench-cost/v1"

# (input $/M tokens, output $/M tokens). Audio input is billed at the input
# rate; that is the assumption these two rows encode.
MODEL_PRICES: dict[str, tuple[float, float]] = {
    "xiaomi/mimo-v2.5": (0.40, 2.00),
    "openai/gpt-4o-mini": (0.15, 0.60),
}

# The bracket. Low is roughly one token per 40 ms, high one per 20 ms.
AUDIO_TOKENS_PER_SECOND_LOW = 25.0
AUDIO_TOKENS_PER_SECOND_HIGH = 50.0

# Rough characters-per-token for prompt text. Only the prompt bodies use it,
# and they are a rounding error next to the audio.
CHARACTERS_PER_TOKEN = 4.0

OUTPUT_TOKENS_BY_ARM: dict[str, int] = {
    "S0": 1100,   # free prose, the longest answers
    "S1": 800,
    "S2a": 800,
    "S2b": 800,
    "S3": 800,
}

# A structured reformat has to carry the whole description into its prompt.
DESCRIPTION_TOKENS_ASSUMED = 1100


@dataclass(frozen=True)
class CallCost:
    call_index: int
    model: str
    kind: str
    audio_seconds: float
    text_input_tokens: int
    audio_input_tokens_low: int
    audio_input_tokens_high: int
    output_tokens: int

    def dollars(self, *, high: bool) -> float:
        prompt_price, completion_price = MODEL_PRICES.get(self.model, (0.0, 0.0))
        audio = self.audio_input_tokens_high if high else self.audio_input_tokens_low
        inputs = (audio + self.text_input_tokens) / 1_000_000.0 * prompt_price
        outputs = self.output_tokens / 1_000_000.0 * completion_price
        return inputs + outputs


def _text_tokens(text: str) -> int:
    return int(math.ceil(len(text) / CHARACTERS_PER_TOKEN))


def _prompt_tokens(cell: matrix.Cell, call: matrix.Call) -> int:
    if call.prompt_text_source == "reformat-turn2":
        # The whole prior turn is resent: the original prompt, the model's
        # description, and the reformat instruction.
        return (
            _text_tokens(prompts.prompt(cell.prompt_id).text)
            + DESCRIPTION_TOKENS_ASSUMED
            + _text_tokens(prompts.REFORMAT_INSTRUCTION)
            + _text_tokens(str(schema.DESCRIPTION_SCHEMA))
        )
    if call.prompt_text_source == "reformat-standalone":
        return (
            _text_tokens(prompts.REFORMAT_STANDALONE)
            + DESCRIPTION_TOKENS_ASSUMED
            + _text_tokens(str(schema.DESCRIPTION_SCHEMA))
        )
    body = _text_tokens(prompts.prompt(cell.prompt_id).text) + 80  # span header
    if call.structured:
        body += _text_tokens(str(schema.DESCRIPTION_SCHEMA))
    return body


def call_cost(cell: matrix.Cell, call: matrix.Call) -> CallCost:
    audio_low = int(math.ceil(call.audio_seconds * AUDIO_TOKENS_PER_SECOND_LOW))
    audio_high = int(math.ceil(call.audio_seconds * AUDIO_TOKENS_PER_SECOND_HIGH))
    return CallCost(
        call_index=call.index,
        model=call.model,
        kind=call.kind,
        audio_seconds=call.audio_seconds,
        text_input_tokens=_prompt_tokens(cell, call),
        audio_input_tokens_low=audio_low,
        audio_input_tokens_high=audio_high,
        output_tokens=OUTPUT_TOKENS_BY_ARM.get(cell.shaping, 800),
    )


def cell_cost(cell: matrix.Cell, track: Track) -> list[CallCost]:
    return [call_cost(cell, call) for call in matrix.calls_for(cell, track)]


def project(repeats: int = matrix.DEFAULT_REPEATS) -> dict[str, Any]:
    """The whole matrix's projected cost, per cell and in total."""
    rows: list[dict[str, Any]] = []
    low_total = high_total = 0.0
    audio_seconds = 0.0
    calls = 0
    for cell in matrix.cells():
        track = track_by_slug(cell.track_slug)
        costs = cell_cost(cell, track)
        low = sum(cost.dollars(high=False) for cost in costs) * repeats
        high = sum(cost.dollars(high=True) for cost in costs) * repeats
        seconds = sum(cost.audio_seconds for cost in costs) * repeats
        low_total += low
        high_total += high
        audio_seconds += seconds
        calls += len(costs) * repeats
        rows.append({
            "cell": cell.id,
            "calls": len(costs) * repeats,
            "audio_seconds": seconds,
            "usd_low": low,
            "usd_high": high,
        })
    return {
        "cost_model_version": COST_MODEL_VERSION,
        "repeats": repeats,
        "assumptions": {
            "audio_tokens_per_second_low": AUDIO_TOKENS_PER_SECOND_LOW,
            "audio_tokens_per_second_high": AUDIO_TOKENS_PER_SECOND_HIGH,
            "characters_per_token": CHARACTERS_PER_TOKEN,
            "output_tokens_by_arm": dict(OUTPUT_TOKENS_BY_ARM),
            "description_tokens_assumed": DESCRIPTION_TOKENS_ASSUMED,
            "prices_usd_per_million": {
                model: {"input": prices[0], "output": prices[1]}
                for model, prices in MODEL_PRICES.items()
            },
        },
        "totals": {
            "calls": calls,
            "audio_seconds": audio_seconds,
            "usd_low": low_total,
            "usd_high": high_total,
        },
        "rows": rows,
    }


def calibrate_from_usage(records: Iterable[dict[str, Any]]) -> dict[str, Any]:
    """Recover the real audio token rate from stored responses.

    Uses only calls that carried audio and reported ``prompt_tokens``: the
    text part of those prompts is subtracted with the same estimate the
    projection used, so the residual over the chunk duration is the rate. It
    is an estimate of an estimate until the first live run, which is exactly
    why it is reported with its sample count.
    """
    samples: list[float] = []
    for record in records:
        if record.get("kind") != "audio":
            continue
        usage = (record.get("usage") or {})
        prompt_tokens = usage.get("prompt_tokens")
        seconds = record.get("audio_seconds")
        text_tokens = record.get("estimated_text_input_tokens")
        if not isinstance(prompt_tokens, (int, float)) or not seconds:
            continue
        residual = float(prompt_tokens) - float(text_tokens or 0)
        if residual <= 0:
            continue
        samples.append(residual / float(seconds))
    if not samples:
        return {"samples": 0, "audio_tokens_per_second": None}
    samples.sort()
    middle = len(samples) // 2
    median = (samples[middle] if len(samples) % 2
              else (samples[middle - 1] + samples[middle]) / 2.0)
    return {
        "samples": len(samples),
        "audio_tokens_per_second": median,
        "minimum": samples[0],
        "maximum": samples[-1],
    }


def recorded_cost(record: dict[str, Any]) -> float:
    """What one stored call actually cost, in USD.

    OpenRouter reports ``usage.cost``; that number is preferred. When it is
    missing the tokens are priced with ``MODEL_PRICES``, which on the first
    live call (2026-11-14) agreed with ``usage.cost`` to the cent. Reasoning
    tokens are inside ``completion_tokens`` and bill at the output rate, which
    is the reason the token-only projection above undershoots.
    """
    usage = record.get("usage") or {}
    reported = usage.get("cost")
    if isinstance(reported, (int, float)) and not isinstance(reported, bool):
        return float(reported)
    identity = record.get("identity") or {}
    model = identity.get("requested_model") or record.get("model_served") or ""
    prompt_price, completion_price = MODEL_PRICES.get(str(model), (0.0, 0.0))
    prompt_tokens = usage.get("prompt_tokens") or 0
    completion_tokens = usage.get("completion_tokens") or 0
    return (float(prompt_tokens) * prompt_price
            + float(completion_tokens) * completion_price) / 1_000_000.0


class Budget:
    """A hard spending ceiling checked before every call, from recorded usage.

    ``reserve`` is what the next call is allowed to cost in the worst case seen
    so far for its model (never less than its projection), so a call is only
    sent while ``spent + reserve`` stays inside the ceiling.
    """

    def __init__(self, ceiling_usd: float, records: Iterable[dict[str, Any]] = ()):
        self.ceiling_usd = float(ceiling_usd)
        self.spent_usd = 0.0
        self.calls = 0
        self.worst_by_model: dict[str, float] = {}
        for record in records:
            self.add(record)

    def add(self, record: dict[str, Any]) -> None:
        spent = recorded_cost(record)
        self.spent_usd += spent
        self.calls += 1
        model = str((record.get("identity") or {}).get("requested_model") or "")
        self.worst_by_model[model] = max(self.worst_by_model.get(model, 0.0), spent)

    def reserve_for(self, cell: matrix.Cell, call: matrix.Call) -> float:
        projected = call_cost(cell, call).dollars(high=True)
        return max(projected, self.worst_by_model.get(call.model, 0.0))

    def refusal(self, cell: matrix.Cell, call: matrix.Call) -> str | None:
        reserve = self.reserve_for(cell, call)
        if self.spent_usd + reserve > self.ceiling_usd:
            return (f"budget: {self.spent_usd:.4f} USD spent over {self.calls} "
                    f"recorded call(s); the next call may cost up to "
                    f"{reserve:.4f} USD, which would pass the "
                    f"{self.ceiling_usd:.2f} USD ceiling")
        return None


def format_projection(projection: dict[str, Any]) -> str:
    lines: list[str] = []
    totals = projection["totals"]
    lines.append(
        f"projected cost, {projection['repeats']} repeats, "
        f"{totals['calls']} calls, {totals['audio_seconds']:.0f} audio seconds")
    lines.append(
        f"  audio billed at {AUDIO_TOKENS_PER_SECOND_LOW:g}-"
        f"{AUDIO_TOKENS_PER_SECOND_HIGH:g} tokens/s (assumption, bracketed)")
    width = max(len(str(row["cell"])) for row in projection["rows"])
    lines.append(f"  {'cell'.ljust(width)}  calls  audio_s      USD low     USD high")
    for row in projection["rows"]:
        lines.append(
            f"  {str(row['cell']).ljust(width)}  {row['calls']:5d}  "
            f"{row['audio_seconds']:7.0f}  {row['usd_low']:11.4f}  {row['usd_high']:11.4f}")
    lines.append(
        f"  {'TOTAL'.ljust(width)}  {totals['calls']:5d}  "
        f"{totals['audio_seconds']:7.0f}  {totals['usd_low']:11.4f}  "
        f"{totals['usd_high']:11.4f}")
    return "\n".join(lines)


def price_table(models: Sequence[str] | None = None) -> dict[str, tuple[float, float]]:
    if models is None:
        return dict(MODEL_PRICES)
    return {model: MODEL_PRICES[model] for model in models if model in MODEL_PRICES}


__all__ = [
    "AUDIO_TOKENS_PER_SECOND_HIGH",
    "AUDIO_TOKENS_PER_SECOND_LOW",
    "Budget",
    "CHARACTERS_PER_TOKEN",
    "COST_MODEL_VERSION",
    "CallCost",
    "DESCRIPTION_TOKENS_ASSUMED",
    "MODEL_PRICES",
    "OUTPUT_TOKENS_BY_ARM",
    "calibrate_from_usage",
    "call_cost",
    "cell_cost",
    "format_projection",
    "price_table",
    "project",
    "recorded_cost",
]
