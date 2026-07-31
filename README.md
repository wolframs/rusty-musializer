# Rusty Musializer

A Rust rewrite of [Musializer](https://github.com/tsoding/musializer), a music
visualiser. This repository is a rewrite of a personal fork of that project, which
is kept frozen and used as a behavioural oracle rather than as a starting point.

Linux-first, hobby scope, agent-driven.

## What it does

Opens an audio file, plays it, and draws one of ten visualiser scenes reacting to
a real-time spectrum analysis of it. Projects are saved as `.musi` files that
round-trip with the C original. Video is exported through FFmpeg as an external
executable.

Scenes: Spectrum, Pulse Field, Orbital Lattice, ASCII Field, Song Atlas, Spectral
Terrarium, Constellation, Cadence, Loom, Pentagram Orbits.

Beyond drawing, the application carries a tuning inspector with per-scene settings
and a preset library, a route editor mapping analysis values (RMS, peak, spectral
flux, beat phase, individual bands) onto scene parameters, a lyrics editor with
caption typography and font import, a manual event timeline, and an export panel.

## Building

```sh
cargo build
cargo run -- path/to/song.mp3
```

No environment setup is needed. raylib 5.5 is vendored under `vendor/` and built
by the `raylib-5-5-link` crate; fonts and shaders are embedded in the binary
rather than read from disk at runtime.

FFmpeg is required at runtime for video export only, as an external executable.

## Checking it

```sh
tools/verify.sh            # everything that can check itself, in order
tools/verify.sh --quick    # same, minus the headless capture
```

That runs formatting, clippy, the unit tests, thirteen differential harnesses
against the frozen C, and a headless capture gate on a private Xvfb display.

The differential harnesses are the substance of the correctness argument. Each one
compiles the relevant `.c` from the frozen C repository, runs the same inputs
through both implementations, and compares the outputs value by value — currently
around 1.2 million compared values across the analyzer, settings tables, route
evaluation and persistence, event merging, the assist state machine, the preset
store, the Song Atlas map, ASCII art, `.musi` I/O in both directions, timeline and
workspace layout, and the beat tracker.

The harnesses need the frozen C repository present at `../musializer`. Without it
they are skipped; everything else still runs.

## Layout

```
crates/
  raylib-5-5-link/    builds and links raylib 5.5 from vendor/
  musializer-core/    no raylib: analysis, scene contracts, model, layout
  musializer-runtime/ raylib, the audio bridge, processes, filesystem edges
  musializer-app/     the binary, CLI, scene drawing, UI
vendor/               upstream raylib source and a bindgen header shim
resources/            first-party shaders; third-party fonts with their licences
docs/                 CLI grammar, settings tables, schemas, environment variables
packaging/linux/      desktop entry and MIME package templates
tools/                verification scripts, the headless gate, the launcher
```

`musializer-core` is deliberately free of raylib handles, OS process handles,
global mutable state and filesystem side effects. That constraint is what makes
the layout, analysis and state-machine code testable without a window.

## Notes

`AGENTS.md` is the working guide: architecture decisions, the `unsafe` inventory,
the differential testing method, and an explicit list of what is deliberately not
built. `REWRITE_PLAN.md` carries the plan and a running record of what actually
happened, which is not always the same thing.

Deliberately not built: microphone capture, hot reload, Windows and macOS.

## Licence

First-party code is this repository's own. Third-party components keep their own
licences: raylib under `vendor/raylib-5.5/`, and Space Grotesk, Alegreya and
Font Awesome under `resources/fonts/`, each with its licence file alongside.
