#!/usr/bin/env python3
"""Static checks on the capture scripts' process isolation.

Two defects this repository has actually shipped, both invisible until something
spawned a GUI child, and both of which put a file dialog on the operator's real
desktop:

1. **`WAYLAND_DISPLAY` unset instead of set to an unresolvable name.**
   `wl_display_connect(NULL)` reads the variable and falls back to a hardcoded
   `"wayland-0"` when it is missing — which is exactly what the operator's socket
   is called. `env -u WAYLAND_DISPLAY` is therefore not weaker isolation than
   doing nothing; it is identical to doing nothing. This shipped at 46 call
   sites.

2. **An `env` option after an assignment.** `env FOO=bar -u BAZ cmd` runs `-u` as
   the command and exits 127. The capture then "fails to produce a frame" and
   reads like an application bug.

Neither is caught by `bash -n`, by any test, or by a capture — a broken guard is
only visible when a GUI child is spawned, and only a handful of captures do that.

Usage: tools/capture_isolation_lint.py [FILE ...]
"""

import re
import sys
from pathlib import Path

DEFAULT_FILES = ["tools/headless_check.sh", "tools/lyric_lane_capture.sh"]
ASSIGNMENT = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*=")


def logical_lines(text):
    """Yield (line_number, joined_command) with backslash continuations joined."""
    lines = text.split("\n")
    index = 0
    while index < len(lines):
        start = index
        joined = lines[index]
        while joined.rstrip().endswith("\\") and index + 1 < len(lines):
            index += 1
            joined = joined.rstrip()[:-1] + " " + lines[index].strip()
        yield start + 1, joined
        index += 1


def check(path):
    problems = []
    for number, command in logical_lines(Path(path).read_text()):
        stripped = command.strip()
        if stripped.startswith("#"):
            continue

        if re.search(r"\benv\b[^|;#]*?-u\s+WAYLAND_DISPLAY\b", command):
            problems.append(
                (number, "clears WAYLAND_DISPLAY instead of setting it to an "
                         "unresolvable name; libwayland reads an absent variable "
                         "as \"wayland-0\", the operator's own socket")
            )

        match = re.search(r"\benv\b(.*)", command)
        if match:
            seen_assignment = False
            for token in match.group(1).split():
                if ASSIGNMENT.match(token):
                    seen_assignment = True
                elif seen_assignment:
                    if token.startswith("-") and re.fullmatch(r"-[a-zA-Z-]+", token):
                        problems.append(
                            (number, f"env option {token!r} follows an assignment; "
                                     "env will run it as the command and exit 127")
                        )
                    break
    return problems


def main(argv):
    files = argv[1:] or DEFAULT_FILES
    failed = 0
    for path in files:
        if not Path(path).exists():
            print(f"missing: {path}", file=sys.stderr)
            failed += 1
            continue
        for number, reason in check(path):
            print(f"{path}:{number}: {reason}", file=sys.stderr)
            failed += 1
    if failed:
        print(
            f"\nFAIL: {failed} capture-isolation problem(s). See the traps in "
            "AGENTS.md; a guard that does nothing is worse than no guard, "
            "because it reads as protection.",
            file=sys.stderr,
        )
        return 1
    print(f"ok  capture isolation: {len(files)} script(s) clean")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
