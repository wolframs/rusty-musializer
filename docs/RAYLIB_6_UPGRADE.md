# raylib 6.0 upgrade

The application builds upstream raylib **6.0** using the version-neutral
`crates/raylib-link` crate, with `raylib` and `raylib-sys` pinned to **6.0.0**
in `nobuild` mode. This retains ownership of our C build and audio diagnostic.
The workspace Rust minimum is 1.92 because egui 0.35 requires it; raylib's
bindings independently require 1.88.

## Provenance

- Upstream release: https://github.com/raysan5/raylib/releases/tag/6.0
- Tag commit: `dbc56a87da87d973a9c5baa4e7438a9d20121d28`.
- Archive: https://github.com/raysan5/raylib/archive/refs/tags/6.0.tar.gz
- Archive SHA-256: `2b3ee1e2120c7a0796b33062c7e9a694dd8a8caa56a96319ac8c8ecf54a90d0b`.
- `vendor/raylib-6.0` retains upstream `LICENSE` and `src/` only.
- Before the local diagnostic declaration, the vendored `raylib.h` matched
  the header bundled by `raylib-sys` 6.0.0 byte for byte. The bindings and C
  library therefore share the upstream public structure/function definitions.

## Preserved local changes

The only source patch in the old vendor tree (commit `7099741`) was ported to
6.0: the saturating `AUDIO.Buffer.streamUnderruns` counter, reset on audio
initialization, increment at an unfilled stream buffer half, and
`GetAudioStreamUnderrunCount` with the audio mutex around reads. Its public
header declaration is retained. This measures output starvation separately
from analyzer-ring drops. Comparing the imported release tree against the
vendored tree finds changes only in `raudio.c` and `raylib.h`.

The build keeps GLFW/X11 and `SUPPORT_FILEFORMAT_FLAC=1`, with process-local
mute/isolation required for tests. The platform macro is now explicitly
`PLATFORM_DESKTOP_GLFW`; `utils.c` was absorbed into `rcore.c` upstream and is
removed from the module list. Cargo now watches the entire source tree for
header/transitive-source changes. `links = "musializer_raylib"` avoids collision
with raylib-sys 6's `links = "raylib"`; the emitted static library is still
`raylib`. The clang builtin-header shim remains scoped away from GCC.

## Rust compatibility

- `nobuild` in raylib-rs 6 hides its safe camera module. A small by-value
  perspective-camera adapter in runtime draw constructs the upstream FFI
  camera; scene APIs still receive the same camera values.
- Font wrappers expose unsafe `AsRawMut` instead of safe `AsMut`. Our font
  abstraction delegates that ownership-preservation contract to its variants.
- `LoadFontData` now returns an actual glyph count through an output pointer,
  omitting unavailable codepoints. SDF allocation, atlas generation, cleanup
  and final ownership use the returned count rather than the requested count.
  A zero-glyph result is freed and refused before atlas creation, whose zero
  count otherwise means 95 and would overread an empty allocation.
- Audio wave export now returns `Result`; errors retain the existing staging
  cleanup behavior. 3D circle names and draw callback arity were updated.
- `SUPPORT_IMAGE_GENERATION` enables the safe image helper API used by export
  and the egui renderer; the vendored C configuration also enables it.

## Verification

`cargo test -p musializer-runtime --lib` passed: 187 tests, one intentionally
ignored. This includes real process/filesystem checks and runs without an audio
device. The imported source diff has exactly the two diagnostic-patched files.
The main workstream's final release and private, muted visual/UI checks cover
integration. Compilation alone does not establish renderer parity: the 6.0
release includes fullscreen/DPI changes and font rasterizer behavior changes,
so the existing headless framebuffer and export evidence remain necessary.

A native CPU-only probe against the compiled C archive verified that requesting
`A` and U+10FFFF from Alegreya returns one glyph, and requesting U+10FFFF alone
returns zero. The uninitialized audio diagnostic returns zero. No window or
audio device was opened. The runtime camera test pins every position, target,
up vector, FOV and projection field passed into the C camera. Active tool and
test references were searched: no runtime path/check still names the removed
vendor directory; historical archive references remain historical.

The upstream clipboard-image helper references X11 directly. Current callers
use clipboard text, and normal Rust linking discards unused sections; a standalone
C probe likewise needs `--gc-sections` or explicit `-lX11`. If clipboard-image
support is added, the link contract must include X11 rather than assuming every
GLFW-related X11 symbol is dynamically resolved.

## Transport semantic regression caught by the full gate

raylib 6 changed both halves of our seek transaction: `PlayAudioBuffer` resets
`framesProcessed` and marks both halves empty (`raudio.c:664`), while
`UpdateMusicStream` refuses stopped streams (`raudio.c:1893`). The previous
stop/seek/update/play ordering therefore reset every requested playhead to zero.
This broke clip In/Out, still selection, and lyric/scene captures at a requested
time despite successful compilation and clip export.

Initial playback now starts, pauses, primes the buffers, and resumes only when
requested. Seeking stops, starts and pauses, then seeks and primes while parked;
the analysis ring and scene clocks reset while paused, followed by resume only
if playback was previously active. No subsequent Play call erases the seek.

The existing gate assertions were retained. Independent private Xvfb checks,
using `--mute` and an invalid per-process PulseAudio socket, now report exact
clip In at 2.000 s, clip Out at 5.000 s, and a still at 4.000 s/frame120 of240.
Evidence is in `build/raylib-upgrade/seek-qa/`; the prior failure was zero for
all three. The full gate rerun is required to cover their downstream visual
comparisons and live lyric cues.

Final integration evidence: `build/egui-headless-final.log` passes the rendering,
framebuffer, caption and export checks after the transport fix. Its only remaining
failure was a stale recovery-test click, corrected and verified through the full
recovery round trip at `build/raylib-upgrade/recovery-qa/result.log`. The full
headless script was not repeated after that test-only correction.
`build/egui-release-final.log` records the final optimized build.
