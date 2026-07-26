# Fixtures

Everything here is synthetic. Nothing here came from user audio, generated
video, an analysis cache, a `.env`, or a build artifact.

Provenance is recorded against the frozen C repository at
`../musializer`, commit `9300af942bd00d8c85fc4e3c8c02cf2b6356764f`
(`9300af9`, branch `master`). See `docs/PHASE0_INVENTORY.md` sections 8 and 10
for the full details behind every claim below.

## `musi/` — intentionally empty

**There are no `.musi` fixtures to copy.** The frozen C repository contains zero
`.musi` files outside `build/`, and zero are checked into git
(`git ls-files | grep -i musi` returns only source and packaging files whose
*names* contain "musi"). `.gitignore:1-2`, `:10-11`, `:25` in that repo ignore
`music/`, `*.wav`, `*.mp4`, and `build/` precisely so that no audio and no
generated project is ever committed.

The C suite's compatibility fixtures are built **inline in C**, not loaded from
files: `tests/test_project_io.c:25-84` builds a maximal in-memory project,
serializes it with the real serializer, then textually deletes blocks from the
resulting JSON to synthesize an older document. Port the technique, not a file.
The inventory tabulates all sixteen of those tests and the exact compatibility
property each one pins.

The only real `.musi` in that tree is `build/ui-review/demo.musi`, produced at
runtime by `tools/ui_fixture.sh` driving the application itself with
`--save-project` (so the fixture exercises the real import/save path). It lives
under `build/` and is therefore out of bounds for copying. Regenerate it in the
C tree with `tools/ui_fixture.sh` if a real `.musi` is ever needed.

## Synthetic audio — regenerate, do not copy

No audio files are copied here. The C generators in
`tests/audio_fixtures.c` and `tests/audio_fixtures.h` are specified
waveform-by-waveform in `docs/PHASE0_INVENTORY.md` section 8.4, precisely
enough to reimplement bit-identically in Rust: six generators (`silence`,
`sine`, `sweep`, `impulse`, `stereo_imbalance`, `beat`), interleaved `f32`
samples, no RNG, no seed, no windowing, no WAV writing, and the f64→f32 phase
pipeline and per-frame `fmod` wrap spelled out because their rounding is
observable.

The concrete parameter values the C suite uses — and therefore the expected
values a Rust port must reproduce — are listed there too, sourced from
`tests/test_main.c:6-37` and `tests/test_song_atlas_map.c:27`, `:78-79`.

Two further generators outside `audio_fixtures.c` are also specified, because
they use different numerics: the f32-tau 44.1 kHz sine in
`tests/test_audio_analyzer.c:9-15`, and the only WAV writer and only Hann
window in the tree, `tests/adapters/test_measured_analysis.py:24-44`.

The headless capture fixture's 40 s stereo WAV is likewise a recipe, not a
file: the exact `ffmpeg aevalsrc` expression from `tools/ui_fixture.sh:23-29`
is quoted in inventory section 10.1.

## Not copied, by policy

`build/`, `music/`, any `.mp3` / `.wav` / `.mp4`, `.env`, and analysis caches
are never copied into this repository.
