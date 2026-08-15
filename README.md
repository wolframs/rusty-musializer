<p align="center">
  <img src="resources/logo/logo.svg" width="128" alt="Musializer logo">
</p>

<h1 align="center">Musializer</h1>

<p align="center">
  <strong>Turn a song into a visual performance.</strong><br>
  A Linux-first music visualization studio, built in Rust for musicians,
  motion artists, live-visuals tinkerers, and people who enjoy seeing signal
  processing become pictures.
</p>

Musializer listens to an audio track, extracts its changing shape, and gives you
ten reactive scenes to direct. You can tune a scene by hand, route parts of the
sound into its controls, arrange scene changes on a timeline, author timed
lyrics, and render the result to video.

This is the canonical, actively developed Musializer. It began as a careful Rust
port of an earlier C application; that codebase is now frozen history and serves
only as a provenance oracle for behavior worth preserving. The product, file
format, and creative direction live here.

## What can you make with it?

- **Shape sound into motion.** Spectrum, Pulse Field, Orbital Lattice, ASCII
  Field, Song Atlas, Spectral Terrarium, Constellation, Cadence, Loom, and
  Pentagram Orbits each respond to the same track in a different visual language.
- **Direct the arrangement.** Split a song into scene segments, retarget them,
  capture per-segment tuning, place events, and keep the whole performance
  aligned on one zoomable timeline.
- **Make the music drive the controls.** Route RMS, peak, spectral flux, beat
  phase, time, or any of 104 analyzer bands into scene parameters with tunable
  curves and ranges.
- **Treat lyrics as typography.** Time and stack cues, import a typeface, choose
  anchors and backing styles, then add audio-reactive glow, hue, shadow, and
  plate effects.
- **Render for the actual canvas.** Export deterministic MP4 video through
  FFmpeg in landscape, square, or vertical formats. High and Master modes use a
  linear-light supersampling resolve so fine bright detail survives the trip.
- **Ask for evidence, not magic.** Optional Assist workflows can measure a
  track, propose scene structure, and align authored lyrics. Candidates remain
  reviewable and inert until you apply them, and local/remote data boundaries
  are explicit.

## First run

You need a Rust toolchain (Rust 1.80 or newer) and an ordinary Linux C build
toolchain. raylib 5.5 is vendored and built with the project; fonts and shaders
are embedded in the executable, so the app does not depend on the directory it
was launched from.

```sh
git clone https://github.com/wolframs/rusty-musializer.git
cd rusty-musializer
cargo run --release -- path/to/song.mp3
```

FFmpeg is optional for playback and editing, and required only when you render
video. To install a per-user desktop entry and `.musi` file association:

```sh
tools/install-linux-launcher.sh
```

Once the window opens, a useful first session is:

1. Choose a scene and open **Tune**.
2. Change a few controls, or map one to an analyzer source with the route editor.
3. Add scene splits along the timeline and give each segment its own look.
4. Open **Lyrics** to add timed captions and typography.
5. Open **Export**, choose a canvas and quality, then render an MP4.

Projects save as `.musi` files and carry their authored state and verified asset
references. The CLI can also render without an editing session:

```sh
cargo run --release -- \
  --project performance.musi \
  --render performance.mp4 \
  --resolution 1080x1920 \
  --fps 30 \
  --quality master
```

Run `cargo run --release -- --help` for the complete command-line surface.

## Assisted analysis, with the boundary visible

Musializer keeps model frameworks out of the renderer. First-party Python tools
under [`tools/`](tools/) supervise measured analysis, FFmpeg, whisper.cpp, local
alignment, Codex, and explicitly authorized OpenRouter requests. Their result
crosses into the application through a small validated artifact rather than
mutating a live project behind your back.

The local **Scene changes** workflow needs Python 3.10+, NumPy, and FFmpeg. Timed
lyrics additionally need whisper.cpp and either authored lyrics or Codex; hosted
modes require explicit OpenRouter authorization. Check the current machine
without starting a model or making a network request:

```sh
python3 tools/musializer_doctor.py
python3 tools/musializer_doctor.py --require local_lyrics
```

The [Assist pipeline](docs/ASSIST_PIPELINE.md) explains validation, staging, lyric
timing, and trust boundaries. The lower-level discovery paths and privacy notes
live in [`tools/ANALYSIS_ADAPTERS.md`](tools/ANALYSIS_ADAPTERS.md).

## Why the internals are worth exploring

Musializer is a creative tool, but its engineering argument is unusually
concrete:

- **Pure decisions, effectful edges.** `musializer-core` owns analysis, model,
  layout, and UI policy without raylib, filesystem, or process handles. Rendering
  and operating-system effects sit in named outer layers.
- **One frame contract.** Preview and offline export consume the same project
  lanes, scene plan, routes, settings, and timestamps rather than maintaining two
  interpretations of a song.
- **Measured compatibility.** Thirteen differential harnesses compare retained
  behavior against the frozen C oracle over roughly 1.2 million values. The
  oracle is evidence, not the roadmap.
- **Real rendering gates.** A private Xvfb run exercises the application, captures
  consequential UI states, and checks report lines and pixels. Test playback is
  process-muted and isolated from the operator's audio session without changing
  the PCM being analyzed.
- **No cwd-shaped runtime.** First-party shaders and fonts are embedded. Project
  assets are content-addressed and verified.

Start with the [architecture guide](docs/CODE_ARCHITECTURE.md) for the important
data flows. The [generated code map](docs/CODE_MAP.md) is the faster “where is
that?” index; regenerate it after moving or adding Rust modules with:

```sh
tools/code_map.py
```

## Developing

The common loop is intentionally small:

```sh
cargo fmt --check
cargo test                 # headless: no window or audio device
cargo clippy --all-targets
tools/verify.sh --quick    # repository gate without visual capture
tools/verify.sh            # full gate, including private-Xvfb evidence
```

Read [`AGENTS.md`](AGENTS.md) before changing rendering, audio, `unsafe` code, or
the verification harnesses. It records the safety invariants and the negative-
control method behind the tests. [`FEATURE_PARITY_PLAN.md`](FEATURE_PARITY_PLAN.md)
is the sole live product/completion queue; `REWRITE_PLAN.md` is historical
evidence, not a second backlog.

Agent-authored human listening checks can use the local
[`tools/listening-lab`](tools/listening-lab/) Vite workspace. It serves declared
local audio through zoomable waveforms, preserves exact time across blind A/B
switches, and appends each feedback revision to a gitignored JSONL log.

For navigation:

- [`docs/README.md`](docs/README.md) — documentation index and authority map
- [`docs/CODE_ARCHITECTURE.md`](docs/CODE_ARCHITECTURE.md) — ownership and data flows
- [`docs/CODE_MAP.md`](docs/CODE_MAP.md) — generated source/target inventory
- [`docs/PHASE0_INVENTORY.md`](docs/PHASE0_INVENTORY.md) — formats, CLI, schemas, and environment contracts
- [`tools/ANALYSIS_ADAPTERS.md`](tools/ANALYSIS_ADAPTERS.md) — optional analysis dependencies and privacy boundaries

## Scope and status

The application builds and runs; all ten scenes, project open/save, timeline
editing, caption authoring, Assist staging, and FFmpeg export are real. It remains
a Linux-first project. Microphone capture, hot reload, and non-Linux platforms
are deliberate exclusions for now; current product gaps are tracked openly in
the [feature-parity plan](FEATURE_PARITY_PLAN.md).

## License

First-party code is available under the [MIT License](LICENSE). Vendored raylib
and the bundled Space Grotesk, Alegreya, and Font Awesome resources retain their
own license files beside their source or assets.
