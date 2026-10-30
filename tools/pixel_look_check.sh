#!/usr/bin/env bash
# Pixel/CRT filter and appearance moved from the retired toolkit inspector.
# Every launch uses the isolated, process-muted launcher. Control points come
# from pixel look:; writes are checked against the actual next-frame model.
set -euo pipefail
cd "$(dirname "$0")/.."
exec python3 tools/pixel_look_check.py "${1:-build/wx5-editor/gpu}"
