#!/usr/bin/env bash
#
# Runs the application with no window on the operator's screen and no sound in
# their headphones, and passes every argument through.
#
#   tools/headless_run.sh --treatment song.treatment.json --contact-sheet build/mv/sheet.png
#   tools/headless_run.sh --treatment song.treatment.json --encoder nvenc --render build/mv/draft.mp4
#   tools/headless_run.sh song.mp3 --scene atlas --still 73.5 build/mv/drop.png
#
# `tools/headless_check.sh` is the gate: it asserts things about a synthetic
# fixture. This is the tool: it renders whatever it is told to, for a caller
# that has no display of its own — a script, a batch job, or an agent directing
# a video (docs/MUSIC_VIDEO_CLI.md). The application needs an X display even to
# export offline, so "just run the binary" from such a caller either fails or,
# worse, opens a window on the desktop of whoever is logged in.
#
# The isolation is the gate's, for the gate's reasons (AGENTS.md, "Test audio
# must never reach the operator's headphones" and the WAYLAND_DISPLAY trap):
#
#   - a private Xvfb display, so nothing is drawn on a real session;
#   - WAYLAND_DISPLAY *set to a name that cannot resolve*. Never unset: libwayland
#     reads an absent variable as "wayland-0", the operator's own socket;
#   - PULSE_SERVER pointed at a path that cannot resolve;
#   - --mute on every launch, which zeroes raylib's output only: the decoded
#     samples, the analyzer input and the exported soundtrack are untouched;
#   - kdialog and zenity hidden from PATH, so nothing can summon a file dialog;
#   - a private XDG_CONFIG_HOME, so a batch run neither reads the operator's UI
#     preferences nor writes their recent-project list.
#
# Environment:
#   MZ_BIN          the binary to run. Default: whichever of target/release and
#                   target/debug was built last, so a stale release build is
#                   never preferred over the code just compiled.
#   MZ_GL_LAUNCH    the GL launcher. Default: `vglrun -d egl0` when VirtualGL is
#                   installed (GPU rendering), otherwise none (Mesa on the CPU,
#                   correct but several times slower). Set it empty to force CPU.
#   MZ_VGL_DEVICE   the VirtualGL device, default egl0.
#   MZ_SCREEN       the virtual screen, default 2560x1440x24.
#   MZ_STATE_DIR    where the private config lives, default build/headless-run.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if [ "$#" -eq 0 ] || [ "$1" = "--wrapper-help" ]; then
    sed -n '2,/^set -euo/p' "${BASH_SOURCE[0]}" | sed -e '$d' -e 's/^# \{0,1\}//'
    exit 0
fi

if ! command -v xvfb-run >/dev/null 2>&1; then
    echo "headless_run: xvfb-run is not installed (Debian/Ubuntu: apt install xvfb)" >&2
    exit 2
fi

BIN="${MZ_BIN:-}"
if [ -z "$BIN" ]; then
    RELEASE="$REPO_ROOT/target/release/musializer"
    DEBUG="$REPO_ROOT/target/debug/musializer"
    if [ -x "$RELEASE" ] && [ -x "$DEBUG" ]; then
        if [ "$DEBUG" -nt "$RELEASE" ]; then BIN="$DEBUG"; else BIN="$RELEASE"; fi
    elif [ -x "$RELEASE" ]; then
        BIN="$RELEASE"
    elif [ -x "$DEBUG" ]; then
        BIN="$DEBUG"
    else
        echo "headless_run: no binary; run 'cargo build --release' first" >&2
        exit 2
    fi
fi
if [ ! -x "$BIN" ]; then
    echo "headless_run: $BIN is not an executable" >&2
    exit 2
fi

# Resolved to an absolute path here, because the launch below runs with PATH
# reduced to the no-dialog directory and a bare `vglrun` under it is exit 127.
GL=()
if [ -n "${MZ_GL_LAUNCH+set}" ]; then
    # shellcheck disable=SC2206  # deliberate word splitting: it is a command line
    GL=(${MZ_GL_LAUNCH})
elif command -v vglrun >/dev/null 2>&1; then
    GL=("$(command -v vglrun)" -d "${MZ_VGL_DEVICE:-egl0}")
fi

STATE_DIR="${MZ_STATE_DIR:-$REPO_ROOT/build/headless-run}"
mkdir -p "$STATE_DIR/config" "$STATE_DIR/bin"

# The second guard, and the one that is easy to lose: the application finds
# kdialog/zenity by searching PATH, so a PATH without them is what makes a
# stray picker unreachable. Everything it legitimately spawns is linked in here
# by absolute path instead.
# The last four are what `vglrun`, itself a shell script, calls on its way up.
for tool in ffmpeg ffprobe python3 env sh bash dirname basename sed uname; do
    if resolved="$(command -v "$tool" 2>/dev/null)"; then
        ln -sf "$resolved" "$STATE_DIR/bin/$tool"
    fi
done

# stderr, so a caller capturing stdout gets the application's report and
# nothing else.
{
    echo "headless_run: bin $BIN"
    if [ "${#GL[@]}" -gt 0 ]; then
        echo "headless_run: gl  ${GL[*]}"
    else
        echo "headless_run: gl  none — Mesa on the CPU (install VirtualGL for the GPU)"
    fi
} >&2

exec xvfb-run -a -s "-screen 0 ${MZ_SCREEN:-2560x1440x24}" \
    env \
    WAYLAND_DISPLAY=musializer-headless-run-no-compositor \
    PULSE_SERVER=unix:/nonexistent/musializer-headless-run-no-audio \
    XDG_CONFIG_HOME="$STATE_DIR/config" \
    PATH="$STATE_DIR/bin" \
    "${GL[@]}" "$BIN" --mute "$@"
