#!/usr/bin/env python3
"""The argv the application hands the helper, checked against the real parser.

Protocol-map row 9 (`docs/archive/ASSIST_AUDIT_2026-08-29.md` §C): "helper argv
+ env, Rust->Py, no version field — Rust pins argv against a **fake helper**; no
Python test accepts the real flag set." `AssistJob::start`
(`crates/musializer-runtime/src/process/assist.rs`) composes the command line
and `the_helper_receives_the_oracles_argv` pins it — against a three-line script
that writes down what it was given. That test cannot notice a flag
`tools/external_analysis.py` no longer has: the fake helper accepts everything,
and the real one would exit 2 before doing any work, which the user sees as a
job that died instantly behind a generic toast.

What is pinned here is **acceptance, not order**. A stage that reorders the
composition, or adds a conditional flag, should not have to edit an expectation
in two languages — but a stage that adds a flag to the Rust side and not to the
helper must fail, which is what `test_every_flag_the_runtime_sends_is_listed`
is for: the list below is checked against the Rust source that sends it.

Pure: no subprocess, no network, no audio device.
"""

from __future__ import annotations

import contextlib
import io
import re
import sys
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))

import external_analysis  # noqa: E402


ASSIST_RS = ROOT / "crates" / "musializer-runtime" / "src" / "process" / "assist.rs"

# Every flag `AssistJob::start` can put on the command line, with a value this
# parser must take for it. The unconditional ones first, then the conditional
# ones in the order the Rust composes them.
#
# `--duration`'s value is the nine-decimal form the Rust test pins as "nine
# decimals, as the C formats it"; `--timeout`'s is the literal 2400 the Rust
# hard-codes. Both are here as the exact strings that cross, because a parser
# that took the flag and refused its value would fail the same way.
UNCONDITIONAL = [
    ("--duration", "40.500000000"),
    ("--mode", "mimo"),
    ("--bridge", "/tmp/musializer/analysis.bridge.tsv"),
    ("--timeout", "2400"),
    ("--new-process-group", None),
    ("--no-dotenv", None),
]
CONDITIONAL = [
    ("--zdr", None),                                        # mode.uses_model()
    ("--lyrics-file", "/tmp/musializer/sheet.txt"),         # an explicit sheet
    ("--execution-snapshot", "/tmp/musializer/route.json"),  # a resolved graph
    ("--whisper-bin", "/opt/whisper/whisper-cli"),          # local runtimes
    ("--whisper-model", "/opt/whisper/ggml-large.bin"),
    ("--align-python", "/opt/align/bin/python3"),
    ("--codex-bin", "/home/example/.local/npm-global/bin/codex"),
]

POSITIONALS = ["assist", "/tmp/track.wav", "/tmp/musializer"]


def argv(flags: list[tuple[str, str | None]]) -> list[str]:
    line = list(POSITIONALS)
    for flag, value in flags:
        line.append(flag)
        if value is not None:
            line.append(value)
    return line


class HelperArgv(unittest.TestCase):
    def parse(self, line: list[str]):
        # `parse_args` calls `parser.error` — which is `SystemExit(2)` — for an
        # unknown flag, a missing value or a rejected choice. Letting it out of
        # the helper is the whole failure this file is about, so the test says
        # so rather than asserting on a return value.
        try:
            return external_analysis.build_parser().parse_args(line)
        except SystemExit as exit_code:  # pragma: no cover - the failure path
            self.fail(f"the helper refused the runtime's argv: exit {exit_code.code}")

    def test_the_unconditional_argv_is_accepted(self) -> None:
        args = self.parse(argv(UNCONDITIONAL))
        self.assertEqual(args.command, "assist")
        self.assertEqual(args.mode, "mimo")
        self.assertEqual(args.duration, 40.5)
        self.assertEqual(args.timeout, 2400)
        self.assertTrue(args.new_process_group)
        self.assertTrue(args.no_dotenv)

    def test_the_widest_argv_is_accepted(self) -> None:
        args = self.parse(argv(UNCONDITIONAL + CONDITIONAL))
        self.assertTrue(args.zdr)
        self.assertEqual(args.lyrics_file, Path("/tmp/musializer/sheet.txt"))
        self.assertEqual(args.execution_snapshot,
                         Path("/tmp/musializer/route.json"))
        self.assertEqual(args.codex_bin,
                         "/home/example/.local/npm-global/bin/codex")

    def test_each_conditional_flag_is_accepted_on_its_own(self) -> None:
        # One at a time, so a helper that dropped exactly one flag names it
        # instead of failing the widest case with no clue which.
        for flag, value in CONDITIONAL:
            with self.subTest(flag=flag):
                self.parse(argv(UNCONDITIONAL + [(flag, value)]))

    def test_every_mode_the_runtime_can_send_is_a_choice(self) -> None:
        # `--mode` is the one flag whose *value* is a closed set on both sides:
        # `AssistMode::argument`'s four tokens against argparse's `choices`.
        modes = set(re.findall(r'=> "(lyrics|sections|mimo|all)"',
                               ASSIST_RS.read_text(encoding="utf-8")))
        self.assertEqual(modes, {"lyrics", "sections", "mimo", "all"})
        for mode in sorted(modes):
            with self.subTest(mode=mode):
                line = argv([("--mode", mode) if flag == "--mode" else (flag, value)
                             for flag, value in UNCONDITIONAL])
                self.assertEqual(self.parse(line).mode, mode)

    def refuse(self, line: list[str]) -> int:
        """Parse expecting a refusal. argparse prints its usage to stderr on the
        way out, which is noise in a suite the shell gate reads."""
        with self.assertRaises(SystemExit) as refused:
            with contextlib.redirect_stderr(io.StringIO()):
                external_analysis.build_parser().parse_args(line)
        return refused.exception.code

    def test_an_unknown_flag_is_refused(self) -> None:
        self.assertEqual(
            self.refuse(argv(UNCONDITIONAL + [("--transcribe-harder", "1")])), 2)

    def test_an_unknown_mode_is_refused(self) -> None:
        self.assertEqual(self.refuse(argv(
            [("--mode", "everything") if flag == "--mode" else (flag, value)
             for flag, value in UNCONDITIONAL])), 2)

    def test_a_missing_required_flag_is_refused(self) -> None:
        # `--duration` and `--mode` are `required=True`; the Rust sends both
        # unconditionally, so their absence is a composition defect rather than
        # a user error, and it has to be loud on this side too.
        for dropped in ("--duration", "--mode"):
            with self.subTest(dropped=dropped):
                self.assertEqual(self.refuse(argv(
                    [pair for pair in UNCONDITIONAL if pair[0] != dropped])), 2)

    def test_every_flag_the_runtime_sends_is_listed(self) -> None:
        """The list above, checked against the source that composes the argv.

        Read as text, the way `tools/support_bundle_check.sh` reads the support
        manifest: a hand-kept second copy is the defect, so the check consults
        the first copy rather than a transcription of it.
        """
        source = ASSIST_RS.read_text(encoding="utf-8")
        start = source.index("pub fn start(spec: &AssistSpec<'_>")
        end = source.index("let child = command.spawn()", start)
        composed = set(re.findall(r'"(--[a-z-]+)"', source[start:end]))
        listed = {flag for flag, _ in UNCONDITIONAL + CONDITIONAL}
        self.assertEqual(composed, listed)


if __name__ == "__main__":
    unittest.main()
