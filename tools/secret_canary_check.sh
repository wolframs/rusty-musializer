#!/usr/bin/env bash
#
# The provider-credential leak scan.
#
#   tools/secret_canary_check.sh                     # the gate
#   tools/secret_canary_check.sh --negative-control  # prove the scan can fail
#
# `docs/ASSIST_PROVIDER_CONTRACTS.md` §4 lists eleven ways a key can escape and
# then gives one acceptance for all of them: plant a syntactically valid key
# carrying a unique sentinel through every entry route, exercise the paths, and
# find zero occurrences anywhere it is not supposed to be. This is that check.
#
# Two things about how it is built are the point.
#
# The sentinel is generated per run, so it exists in no committed file and a
# match is always this run's key rather than a leftover. And the scan carries its
# own positive control: the `0600` credentials file is the one authorized
# plaintext sink, so the scan must find the sentinel there. A scan that finds it
# nowhere at all has stopped working, and that failure would otherwise look
# exactly like success.
#
# Nothing here opens an audio device: make-fixture-wav only writes PCM, and every
# assist invocation is `--dry-run`.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

NEGATIVE=0
if [ "${1:-}" = "--negative-control" ]; then
    NEGATIVE=1
fi

# 32 random hex characters, so the planted key is a syntactically valid
# `sk-or-v1-<64 hex>` and the sentinel is unique to this process.
SENTINEL="$(od -An -N16 -tx1 /dev/urandom | tr -d ' \n')"
PADDING="$(od -An -N16 -tx1 /dev/urandom | tr -d ' \n')"
CANARY_KEY="sk-or-v1-${SENTINEL}${PADDING}"

WORK="$(mktemp -d)"
ANALYSIS_DIR="$REPO_ROOT/build/analysis-canary"
rm -rf -- "$ANALYSIS_DIR"
trap 'rm -rf -- "$WORK" "$ANALYSIS_DIR"' EXIT HUP INT TERM

export XDG_CONFIG_HOME="$WORK/config"
export XDG_CACHE_HOME="$WORK/cache"
mkdir -p "$XDG_CONFIG_HOME" "$XDG_CACHE_HOME" "$WORK/probe" "$ANALYSIS_DIR"

fail() {
    printf '\033[31mFAILED\033[0m  %s\n' "$1" >&2
    exit 1
}

require() {
    # require <description> <expected-line> <file>
    if ! grep -qxF -- "$2" "$3"; then
        printf 'expected %s\n' "$2" >&2
        sed -n '1,40p' "$3" >&2
        fail "$1"
    fi
}

# ---------------------------------------------------------------------------
# Route 1-3: env import, the 0600 file, and the session store.
# ---------------------------------------------------------------------------

cargo build --quiet -p musializer-runtime --example assist_canary_probe
PROBE="$REPO_ROOT/target/debug/examples/assist_canary_probe"
[ -x "$PROBE" ] || fail "the canary probe did not build"

OPENROUTER_API_KEY="$CANARY_KEY" "$PROBE" "$WORK/probe" >"$WORK/probe.log" 2>&1 \
    || { cat "$WORK/probe.log" >&2; fail "the canary probe exited non-zero"; }

require "the planted key was not imported"           "imported: true"                    "$WORK/probe.log"
require "a child inherited the credential variable"  "child-has-openrouter-key: false"    "$WORK/probe.log"
require "the secret's Debug is not redacted"         "debug-print: <redacted>"           "$WORK/probe.log"
require "the credentials file is not owner-only"     "credentials-file-mode: 0600"       "$WORK/probe.log"
require "the credentials directory is not owner-only" "credentials-directory-mode: 0700" "$WORK/probe.log"
require "the probe did not run to completion"        "probe: complete"                   "$WORK/probe.log"

# ---------------------------------------------------------------------------
# The desktop path refuses the repository .env fallback; the CLI keeps it.
# ---------------------------------------------------------------------------

DOTENV="$WORK/dotenv/.env"
mkdir -p "$WORK/dotenv"
printf 'OPENROUTER_API_KEY="%s"\n' "$CANARY_KEY" >"$DOTENV"

env -u OPENROUTER_API_KEY python3 - "$DOTENV" <<'PY' || fail "the .env fallback flag does not gate the credential"
import pathlib, sys
sys.path.insert(0, "tools")
import external_analysis

path = pathlib.Path(sys.argv[1])
desktop = external_analysis._openrouter_env(path, allow_dotenv=False)
command_line = external_analysis._openrouter_env(path)
assert "OPENROUTER_API_KEY" not in desktop, "the desktop path accepted a .env credential"
assert command_line.get("OPENROUTER_API_KEY"), "the command line lost its .env fallback"
assert not any("KEY" in name.upper() for name in desktop), "a credential-named variable survived the strip"
PY

# ---------------------------------------------------------------------------
# A dry-run job, with the key in the environment the whole time.
# ---------------------------------------------------------------------------

FIXTURE="$WORK/fixture.wav"
if [ -x target/debug/make-fixture-wav ]; then
    target/debug/make-fixture-wav "$FIXTURE" 2
else
    cargo run --quiet --bin make-fixture-wav -- "$FIXTURE" 2
fi

for mode in mimo all; do
    OPENROUTER_API_KEY="$CANARY_KEY" python3 tools/external_analysis.py assist \
        "$FIXTURE" "$ANALYSIS_DIR/$mode" \
        --duration 2 --mode "$mode" --timeout 600 --dry-run --no-dotenv \
        >"$ANALYSIS_DIR/$mode.json"
    grep -q '"credentials": "environment only; omitted"' "$ANALYSIS_DIR/$mode.json" \
        || fail "the dry run stopped reporting that credentials are omitted"
    grep -q '"dotenv_fallback": false' "$ANALYSIS_DIR/$mode.json" \
        || fail "--no-dotenv is not reported in the dry run"
done

OPENROUTER_API_KEY="$CANARY_KEY" python3 tools/external_analysis.py assist \
    "$FIXTURE" "$ANALYSIS_DIR/cli" \
    --duration 2 --mode mimo --timeout 600 --dry-run \
    >"$ANALYSIS_DIR/cli.json"
grep -q '"dotenv_fallback": true' "$ANALYSIS_DIR/cli.json" \
    || fail "the command line lost its .env fallback"

# E7: the doctor reports membership, never the value. Its exit status is allowed
# to be non-zero — a missing optional helper is a legitimate report — but an
# empty file would make the scan of it vacuous, so that is checked.
OPENROUTER_API_KEY="$CANARY_KEY" python3 tools/musializer_doctor.py --json \
    >"$WORK/doctor.json" 2>"$WORK/doctor.err" || true
[ -s "$WORK/doctor.json" ] || fail "the doctor produced no report to scan"

# ---------------------------------------------------------------------------
# The scan.
# ---------------------------------------------------------------------------

CREDENTIALS_FILE="$XDG_CONFIG_HOME/musializer/credentials.json"

# The two files that are *supposed* to hold the key in plaintext: the 0600
# credentials store, and the .env fixture written above to prove the command
# line still honours it. Everything else is a leak.
AUTHORIZED_SINKS="$(printf '%s\n%s\n' "$CREDENTIALS_FILE" "$DOTENV")"

if [ "$NEGATIVE" -eq 1 ]; then
    # Deliberately leak the sentinel into a scanned artifact and require the scan
    # to notice. Without this, "zero occurrences" could be reporting a scan that
    # searches nothing.
    printf '{"leaked": "%s"}\n' "$CANARY_KEY" >"$ANALYSIS_DIR/leaked.json"
fi

SCAN_ROOTS=("$WORK" "$ANALYSIS_DIR")
MATCHES="$(grep -rl --binary-files=text -- "$SENTINEL" "${SCAN_ROOTS[@]}" 2>/dev/null || true)"

# Positive control: the one authorized plaintext sink must be a match, or the
# scan is not actually searching.
if ! printf '%s\n' "$MATCHES" | grep -qxF -- "$CREDENTIALS_FILE"; then
    fail "the scan did not find the sentinel in the credentials file it was just written to"
fi

LEAKS="$(printf '%s\n' "$MATCHES" \
    | grep -vxF -f <(printf '%s\n' "$AUTHORIZED_SINKS") \
    | grep -v '^$' || true)"

if [ "$NEGATIVE" -eq 1 ]; then
    if [ -z "$LEAKS" ]; then
        fail "negative control: a deliberately leaked sentinel was not detected"
    fi
    printf '\033[32mok\033[0m  negative control: the scan caught the planted leak in:\n'
    printf '  - %s\n' $LEAKS
    exit 0
fi

if [ -n "$LEAKS" ]; then
    printf 'the sentinel escaped into:\n' >&2
    printf '  - %s\n' $LEAKS >&2
    fail "provider credential leaked outside the 0600 credentials file"
fi

SCANNED="$(find "${SCAN_ROOTS[@]}" -type f | wc -l)"
printf 'secret canary: %s files scanned across the config dir, cache dir, %s, the .musi,\n' \
    "$SCANNED" "build/analysis-canary"
printf '  the job log, the support-bundle output and /proc/self/cmdline.\n'
printf 'secret canary: found only in the 0600 credentials file and the .env fixture,\n'
printf '  which are the two sinks that are supposed to hold it.\n'
