"""The prompt registry: two registers x two specificities, plus the reformatters.

Each prompt is identified by an id and hashed, because §6 of
``docs/ASSIST_PROVIDER_CONTRACTS.md`` requires a benchmark result to name the
exact prompt that produced it. Editing a prompt string changes its sha256,
which changes every cell's request identity — that is the intent, not a
nuisance: an edited prompt invalidates the results it produced.

Two things are held constant across all four prompts so the axes stay
separable:

* the **content requested** is identical within a specificity level, and only
  the register (word choice, politeness, the smiley) differs;
* the **time frame** is declared identically everywhere, so a chunked
  condition and a whole-excerpt condition are asked for the same clock. What
  the model then does with that instruction is a measured outcome, not a
  design difference.
"""

from __future__ import annotations

from dataclasses import dataclass

from .bench_io import canonical_sha256

PROMPT_REGISTRY_VERSION = "musializer.mimo-bench-prompts/v1"

# The seven things every checklist prompt asks for, in one place, so the casual
# and strict variants cannot drift apart in content.
CHECKLIST_ITEMS: tuple[str, ...] = (
    "instrumentation: every instrument you can hear, and the timbre of each "
    "(attack, warmth, bite, breath, processing)",
    "tempo in BPM, the meter (for example 4/4, 3/4, 6/8), and how the groove sits "
    "against the grid",
    "key or tonal centre and its mode, plus any harmony worth naming (chord "
    "qualities, cadences, modal colour, dissonance)",
    "form: the sections you hear, each with the approximate second it starts",
    "lyrics: short quoted phrases only, each with the approximate second it is sung. "
    "Never a full passage",
    "sonic texture and production: stereo space, dynamics, density, reverb and "
    "delay character, compression, mix balance",
    "feel: the emotional character and how it moves across the excerpt",
)


def _numbered(items: tuple[str, ...]) -> str:
    return "\n".join(f"{index}. {item}" for index, item in enumerate(items, 1))


def _bulleted(items: tuple[str, ...]) -> str:
    return "\n".join(f"- {item}" for item in items)


STRICT_OPEN = """\
Describe this audio track in exquisite detail, so that a text-modality LLM can \
understand it. The reader cannot hear anything; your text is the only access it \
has to this audio. Be specific and falsifiable. Do not summarize; describe. Do \
not fabricate a beginning or an ending you were not given."""

CASUAL_OPEN = """\
hey :) i'm handing this bit of audio to a friend who can only read text, never \
hear anything — so could you just listen and tell me everything that's in there? \
describe it in exquisite detail so they can basically hear it through your words. \
go as deep as you like, be really specific about what's actually there rather than \
how it makes you feel in general. no need to invent an intro or an outro you \
weren't given, just take the piece as it comes."""

STRICT_CHECKLIST = f"""\
Describe this audio. Report the following, in this order. For any item you cannot \
determine, write the single word `unknown` rather than guessing.

{_numbered(CHECKLIST_ITEMS)}

Be specific and falsifiable. Do not add commentary outside these items. Do not \
fabricate a beginning or an ending you were not given."""

CASUAL_CHECKLIST = f"""\
hey :) would you have a listen to this and tell me about it? i'm after these \
things in particular — and if you genuinely can't tell for one of them just say \
`unknown`, that's much more useful to me than a guess:

{_bulleted(CHECKLIST_ITEMS)}

be as specific as you can about what's actually in there. and don't invent an \
intro or an outro you weren't given, just take it as it comes!"""


@dataclass(frozen=True)
class Prompt:
    id: str
    register: str        # "strict" | "casual"
    specificity: str     # "open" | "checklist"
    text: str

    @property
    def sha256(self) -> str:
        return canonical_sha256(self.text)


PROMPTS: dict[str, Prompt] = {
    prompt.id: prompt
    for prompt in (
        Prompt("strict-open", "strict", "open", STRICT_OPEN),
        Prompt("casual-open", "casual", "open", CASUAL_OPEN),
        Prompt("strict-checklist", "strict", "checklist", STRICT_CHECKLIST),
        Prompt("casual-checklist", "casual", "checklist", CASUAL_CHECKLIST),
    )
}

PROMPT_IDS: tuple[str, ...] = (
    "strict-open", "casual-open", "strict-checklist", "casual-checklist",
)


def prompt(prompt_id: str) -> Prompt:
    if prompt_id not in PROMPTS:
        raise KeyError(f"unknown prompt id: {prompt_id}")
    return PROMPTS[prompt_id]


# ---------------------------------------------------------------------------
# The time-frame header
# ---------------------------------------------------------------------------

TIME_FRAME_POLICY = "absolute-offset-declared"


def span_header(
    *, chunk_index: int, chunk_count: int, start_seconds: float, end_seconds: float,
) -> str:
    """Declared identically for every cell, including the single-chunk ones.

    Every timestamp the model is asked for is in *source* seconds, and the
    chunk's own offset is stated. A chunked condition therefore has the
    information it needs to answer on the same clock as the whole-excerpt
    condition; whether it uses it is scored (``time_frame_obeyed``) rather
    than assumed.
    """
    if chunk_count == 1:
        position = "This is the whole excerpt."
    else:
        position = f"This is chunk {chunk_index + 1} of {chunk_count}."
    return (
        f"[{position} It is an excerpt of a longer track. In the source track it "
        f"runs from {start_seconds:.2f} s to {end_seconds:.2f} s. Every timestamp "
        f"you give must be in seconds from the start of the source track, so this "
        f"audio begins at {start_seconds:.2f} s, not at 0.]"
    )


def compose(prompt_id: str, header: str) -> str:
    """The exact user text sent for one chunk: header first, then the prompt."""
    return f"{header}\n\n{prompt(prompt_id).text}"


# ---------------------------------------------------------------------------
# Turn 2: reformatting a free description into the schema
# ---------------------------------------------------------------------------

REFORMAT_INSTRUCTION = """\
Now format that description as JSON matching the supplied schema. Use only what \
the description already says: copy values across, do not add observations, and do \
not drop any that fit a field. Where the description gives no value for a field, \
use null for a nullable field, an empty array for a list, and an empty string for \
a text field. Keep every timestamp exactly as the description states it."""

REFORMAT_STANDALONE = """\
Below is one listener's written description of a piece of audio you cannot hear. \
Convert it to JSON matching the supplied schema. Use only what the description \
says: copy values across, do not add observations, and do not drop any that fit a \
field. Where the description gives no value for a field, use null for a nullable \
field, an empty array for a list, and an empty string for a text field. Keep every \
timestamp exactly as the description states it.

DESCRIPTION:
{description}"""

ELIDED_AUDIO_NOTE = (
    "[The audio itself is omitted from this turn. Work only from the description "
    "you gave above.]"
)


def reformat_standalone(description: str) -> str:
    return REFORMAT_STANDALONE.format(description=description)


REFORMAT_SHA256 = canonical_sha256(
    {"turn2": REFORMAT_INSTRUCTION, "standalone": REFORMAT_STANDALONE}
)


__all__ = [
    "CASUAL_CHECKLIST",
    "CASUAL_OPEN",
    "CHECKLIST_ITEMS",
    "ELIDED_AUDIO_NOTE",
    "PROMPTS",
    "PROMPT_IDS",
    "PROMPT_REGISTRY_VERSION",
    "Prompt",
    "REFORMAT_INSTRUCTION",
    "REFORMAT_SHA256",
    "REFORMAT_STANDALONE",
    "STRICT_CHECKLIST",
    "STRICT_OPEN",
    "TIME_FRAME_POLICY",
    "compose",
    "prompt",
    "reformat_standalone",
    "span_header",
]
