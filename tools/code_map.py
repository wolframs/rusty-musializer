#!/usr/bin/env python3
"""Generate the repository's deterministic, linkable code map."""

from __future__ import annotations

import argparse
import ast
import difflib
import json
import re
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
OUTPUT = ROOT / "docs" / "CODE_MAP.md"


@dataclass(frozen=True)
class RustFile:
    path: Path
    lines: int
    tests: int
    summary: str


EXTERNAL_MODULE = re.compile(
    r"^\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+([A-Za-z_][A-Za-z0-9_]*)\s*;",
    re.MULTILINE,
)


def cargo_metadata() -> dict[str, object]:
    result = subprocess.run(
        ["cargo", "metadata", "--no-deps", "--format-version", "1"],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    )
    return json.loads(result.stdout)


def clean_prose(text: str) -> str:
    text = re.sub(r"\[([^]]+)]\([^)]+\)", r"\1", text)
    text = re.sub(r"\s+", " ", text).strip()
    return text.replace("|", "\\|")


def rust_summary(lines: list[str], path: Path) -> str:
    paragraphs: list[list[str]] = [[]]
    saw_doc = False
    for line in lines:
        match = re.match(r"\s*//!\s?(.*)$", line)
        if match:
            saw_doc = True
            content = match.group(1).strip()
            if content:
                paragraphs[-1].append(content)
            elif paragraphs[-1]:
                paragraphs.append([])
            continue
        if saw_doc:
            break
        if line.strip() and not line.startswith("#!"):
            break
    for paragraph in paragraphs:
        if paragraph:
            return clean_prose(" ".join(paragraph))
    raise ValueError(f"Rust source lacks a leading //! module summary: {path.relative_to(ROOT)}")


def rust_file(path: Path) -> RustFile:
    text = path.read_text(encoding="utf-8")
    lines = text.splitlines()
    tests = len(re.findall(r"#\s*\[\s*(?:[A-Za-z_][\w]*::)*test\s*]", text))
    return RustFile(path, len(lines), tests, rust_summary(lines, path))


def child_module(current: Path, name: str) -> Path:
    if current.name in {"lib.rs", "main.rs", "mod.rs"} or current.parent.name in {
        "bin",
        "examples",
    }:
        base = current.parent
    else:
        base = current.parent / current.stem
    candidates = [base / f"{name}.rs", base / name / "mod.rs"]
    present = [candidate for candidate in candidates if candidate.is_file()]
    if len(present) == 1:
        return present[0]
    readable = ", ".join(str(candidate.relative_to(ROOT)) for candidate in candidates)
    if not present:
        raise ValueError(
            f"{current.relative_to(ROOT)} declares mod {name}; expected one of {readable}"
        )
    raise ValueError(
        f"{current.relative_to(ROOT)} declares ambiguous mod {name}; both {readable} exist"
    )


def reachable_rust_sources(package: dict[str, object]) -> list[Path]:
    package_dir = Path(package["manifest_path"]).parent
    roots = [Path(target["src_path"]) for target in package["targets"]]
    pending = list(roots)
    reachable: set[Path] = set()
    while pending:
        current = pending.pop()
        if current in reachable:
            continue
        reachable.add(current)
        source = current.read_text(encoding="utf-8")
        pending.extend(child_module(current, name) for name in EXTERNAL_MODULE.findall(source))

    present = set(package_dir.rglob("*.rs"))
    orphans = sorted(present - reachable)
    if orphans:
        paths = ", ".join(str(path.relative_to(ROOT)) for path in orphans)
        raise ValueError(f"orphan Rust source not reachable from a Cargo target: {paths}")
    return sorted(reachable)


def markdown_link(path: Path, label: str | None = None) -> str:
    relative = path.relative_to(ROOT).as_posix()
    return f"[{label or relative}](../{relative})"


def first_party_summary(path: Path) -> str:
    text = path.read_text(encoding="utf-8")
    if path.suffix == ".py":
        try:
            return clean_prose(ast.get_docstring(ast.parse(text)) or "") or "Python helper"
        except SyntaxError:
            return "Python helper (syntax currently invalid)"
    if path.suffix == ".sh":
        comments: list[str] = []
        for line in text.splitlines()[1:]:
            stripped = line.strip()
            if stripped.startswith("#"):
                content = stripped.lstrip("#").strip()
                if content:
                    comments.append(content)
                elif comments:
                    break
            elif comments:
                break
        return clean_prose(" ".join(comments)) or "Shell helper"
    return path.suffix.lstrip(".").upper() or "support file"


def format_count(value: int) -> str:
    return f"{value:,}"


def generate() -> str:
    metadata = cargo_metadata()
    packages = {package["id"]: package for package in metadata["packages"]}
    workspace = [packages[package_id] for package_id in metadata["workspace_members"]]
    workspace.sort(key=lambda package: package["name"])

    rust_by_package: dict[str, list[RustFile]] = {}
    all_rust: list[RustFile] = []
    for package in workspace:
        files = [rust_file(path) for path in reachable_rust_sources(package)]
        rust_by_package[package["name"]] = files
        all_rust.extend(files)

    out = [
        "# Project code map",
        "",
        "> Generated by [`tools/code_map.py`](../tools/code_map.py) from Cargo metadata,",
        "> the source tree, and each Rust file's leading `//!` module documentation.",
        "> Run `tools/code_map.py` to repair it; `tools/code_map.py --check` detects drift.",
        "",
        "Use this as the fast path from a responsibility to its code. The",
        "[architecture guide](CODE_ARCHITECTURE.md) explains ownership and data flow; this",
        "file answers what exists *right now*. It deliberately contains no timestamp, so a",
        "regeneration with unchanged inputs is byte-for-byte stable.",
        "",
        "## Workspace at a glance",
        "",
        "| Crate | Purpose | Cargo targets | Rust files | Lines | Tests |",
        "| --- | --- | --- | ---: | ---: | ---: |",
    ]
    for package in workspace:
        package_dir = Path(package["manifest_path"]).parent
        files = rust_by_package[package["name"]]
        targets = ", ".join(
            f"`{target['name']}` ({'/'.join(target['kind'])})" for target in package["targets"]
        )
        out.append(
            "| "
            + markdown_link(package_dir / "Cargo.toml", f"`{package['name']}`")
            + f" | {clean_prose(package.get('description') or '')} | {targets} | "
            + f"{format_count(len(files))} | {format_count(sum(item.lines for item in files))} | "
            + f"{format_count(sum(item.tests for item in files))} |"
        )

    out.extend(
        [
            "",
            "Dependency direction: `musializer-app` → `musializer-runtime` → raylib,",
            "while both outer crates depend on the raylib-free `musializer-core`.",
            "`raylib-5-5-link` builds the exact vendored C library used by the application.",
            "",
            "## Cargo entry points",
            "",
            "| Target | Kind | Entry point |",
            "| --- | --- | --- |",
        ]
    )
    for package in workspace:
        for target in sorted(package["targets"], key=lambda item: (item["kind"], item["name"])):
            source = Path(target["src_path"])
            out.append(
                f"| `{target['name']}` | `{'/'.join(target['kind'])}` | "
                f"{markdown_link(source, source.relative_to(ROOT).as_posix())} |"
            )

    out.extend(["", "## Rust source map", ""])
    for package in workspace:
        files = rust_by_package[package["name"]]
        out.extend(
            [
                f"### `{package['name']}`",
                "",
                "| Source | Lines | Tests | Module responsibility |",
                "| --- | ---: | ---: | --- |",
            ]
        )
        for item in files:
            relative_to_package = item.path.relative_to(Path(package["manifest_path"]).parent)
            out.append(
                f"| {markdown_link(item.path, f'`{relative_to_package.as_posix()}`')} | "
                f"{format_count(item.lines)} | {format_count(item.tests)} | {item.summary} |"
            )
        out.append("")

    out.extend(
        [
            "## Navigation hotspots",
            "",
            "These are the largest Rust files, not automatically refactoring candidates. They",
            "are listed because they are the places where a reader benefits most from starting",
            "with the module documentation and searching for a narrow symbol before scrolling.",
            "",
            "| Source | Lines | Tests |",
            "| --- | ---: | ---: |",
        ]
    )
    for item in sorted(all_rust, key=lambda source: (-source.lines, source.path.as_posix()))[:15]:
        out.append(
            f"| {markdown_link(item.path, f'`{item.path.relative_to(ROOT).as_posix()}`')} | "
            f"{format_count(item.lines)} | {format_count(item.tests)} |"
        )

    out.extend(
        [
            "",
            "## Non-Rust boundaries",
            "",
            "These first-party files are easy to miss in a Cargo-only view. Vendored code,",
            "generated build output, fixtures, and media assets are intentionally excluded.",
            "",
            "### Verification and analysis tools",
            "",
            "| Tool | Role |",
            "| --- | --- |",
        ]
    )
    tool_files = sorted(
        path
        for path in (ROOT / "tools").iterdir()
        if path.is_file() and path.suffix in {".py", ".sh"}
    )
    for path in tool_files:
        out.append(f"| {markdown_link(path, f'`tools/{path.name}`')} | {first_party_summary(path)} |")

    out.extend(
        [
            "",
            "### Differential and policy tests",
            "",
            "| Test source | Kind |",
            "| --- | --- |",
        ]
    )
    test_files = sorted(
        path
        for path in (ROOT / "tests").rglob("*")
        if path.is_file() and path.suffix in {".c", ".h", ".py"}
    )
    for path in test_files:
        out.append(f"| {markdown_link(path, f'`{path.relative_to(ROOT).as_posix()}`')} | `{path.suffix.lstrip('.') or 'file'}` |")

    contracts: list[tuple[str, Path]] = []
    for directory, pattern in (("schemas", "*.json"), ("prompts", "*"), ("resources/shaders", "*")):
        base = ROOT / directory
        if base.exists():
            contracts.extend(
                (directory, path) for path in sorted(base.glob(pattern)) if path.is_file()
            )
    out.extend(
        [
            "",
            "### Schemas, prompts, and shaders",
            "",
            "| Boundary | File |",
            "| --- | --- |",
        ]
    )
    for boundary, path in contracts:
        out.append(
            f"| `{boundary}` | {markdown_link(path, f'`{path.relative_to(ROOT).as_posix()}`')} |"
        )

    out.extend(
        [
            "",
            "## Keeping the map honest",
            "",
            "`tools/code_map.py` regenerates this file atomically. Generation fails if a Rust",
            "source file has no leading `//!` summary, turning module-level orientation into a",
            "maintained interface. `tools/verify.sh` runs the check form before expensive tests.",
            "",
            "When a responsibility changes, update the owning module's leading documentation",
            "along with the code, then regenerate. Hand-written architecture and product history",
            "remain in their focused documents rather than being copied into this inventory.",
            "",
        ]
    )
    return "\n".join(out)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--check",
        action="store_true",
        help="exit non-zero and show a compact diff when docs/CODE_MAP.md is stale",
    )
    args = parser.parse_args()

    try:
        generated = generate()
    except (OSError, ValueError, subprocess.CalledProcessError, json.JSONDecodeError) as error:
        print(f"code map generation failed: {error}", file=sys.stderr)
        return 2

    if args.check:
        current = OUTPUT.read_text(encoding="utf-8") if OUTPUT.exists() else ""
        if current == generated:
            print(f"code map is current: {OUTPUT.relative_to(ROOT)}")
            return 0
        print(
            f"{OUTPUT.relative_to(ROOT)} is stale; run tools/code_map.py to repair it",
            file=sys.stderr,
        )
        diff = difflib.unified_diff(
            current.splitlines(),
            generated.splitlines(),
            fromfile=str(OUTPUT.relative_to(ROOT)),
            tofile="generated",
            lineterm="",
            n=2,
        )
        for index, line in enumerate(diff):
            if index >= 80:
                print("... diff truncated ...", file=sys.stderr)
                break
            print(line, file=sys.stderr)
        return 1

    temporary = OUTPUT.with_suffix(".md.tmp")
    temporary.write_text(generated, encoding="utf-8")
    temporary.replace(OUTPUT)
    print(f"wrote {OUTPUT.relative_to(ROOT)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
