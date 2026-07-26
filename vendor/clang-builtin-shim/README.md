# Compiler-provided headers for libclang

Five headers so `bindgen` can parse raylib's headers on this machine.

## Why this exists

`raylib-sys` runs `bindgen` in its build script, and `bindgen` parses C through
`libclang`. Ubuntu's `libclang1-21` package ships `libclang.so` **without
clang's resource directory** — the place clang keeps the headers a *compiler*
must provide rather than the C library: `stdarg.h`, `stdbool.h`, `stddef.h`,
`float.h`, `stdalign.h`. Everything else raylib includes (`stdio.h`,
`string.h`, `math.h`, `stdlib.h`) comes from glibc in `/usr/include` and
resolves fine.

Without these, the build fails at:

```
binding/../raylib/src/raylib.h:88:10: fatal error: 'stdarg.h' file not found
```

## Why not just point at GCC's copy

`BINDGEN_EXTRA_CLANG_ARGS=-I/usr/lib/gcc/x86_64-linux-gnu/15/include` also
works, and was the first thing tried. It hardcodes a GCC major version, so the
next `gcc` upgrade breaks the build with an error that does not mention GCC.
These five files are ~60 lines total, version-independent, and defined in terms
of the `__builtin_*` and `__*_TYPE__` intrinsics every remotely modern compiler
provides, so nothing here goes stale.

Installing `clang` or `libclang-dev` would also fix it, and is the better answer
on a machine where that is convenient — it needs root, which an unattended
session does not have.

## Why it cannot simply be switched off

`raylib-sys` has a `nobuild` feature that skips building raylib (which is what
`crates/raylib-5-5-link` does instead) and a separate `bindgen` feature. In
principle `bindgen` can be turned off and pregenerated bindings supplied through
`RAYLIB_BINDGEN_LOCATION`. In practice the safe `raylib` crate 5.5.1 declares
`raylib-sys` **without** `default-features = false`
(`raylib-5.5.1/Cargo.toml:87-88`), and Cargo unifies features across the graph,
so `bindgen` is on whenever the `raylib` crate is in the tree. Setting
`default-features = false` on our own `raylib-sys` entry does not undo it.

That was verified, not assumed: `cargo tree -e features -p raylib-sys` shows
`bindgen feature "default"` regardless.

## How it is wired up

`.cargo/config.toml` puts this directory on `CPATH`, which clang honours as an
implicit include search path.

`CPATH` also affects GCC, which would let these minimal headers shadow GCC's
real ones while compiling raylib itself — a genuinely bad outcome, since these
are only complete enough to satisfy a header parse. So
`crates/raylib-5-5-link/build.rs` removes `CPATH` from its own environment
before invoking the C compiler. The shim reaches libclang and nothing else.

## Scope

These are **only** for parsing declarations. They are not a C standard library
and must not be put in front of a real compilation of first-party code.
