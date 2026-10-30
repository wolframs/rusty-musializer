# Embedded third-party notices

`musializer_core::credits` embeds these files, the repository's MIT licence,
three `resources/fonts/` OFL notices, the vendored raylib licence, and the
adapted egui bridge's MIT licence. About → Licence reads only embedded text.

The named crate notices were copied verbatim from the corresponding package
in `Cargo.lock` and its Cargo registry source. `locked-dependencies.txt`
collects the packaged licence/notice files of the Linux normal, build and test
dependency graph (`cargo tree --locked --target x86_64-unknown-linux-gnu
--prefix none -e normal,build,dev`). Build/test-only entries are conservative
attribution, not a claim that their executables are bundled. Each section
records the exact package version, declared SPDX expression and repository.
Regenerate this bundle when changing the lockfile; retain the original notices.

The registry omits the monorepo licences for some packages. Their notices were
retrieved from these original repositories on 2026-10-30:

- egui/ecolor/emath/epaint 0.35.0: https://github.com/emilk/egui/blob/0.35.0/LICENSE-MIT
- raylib/raylib-sys 6.0.0 bindings: https://github.com/raylib-rs/raylib-rs/blob/master/LICENSE.md
- AccessKit: https://github.com/AccessKit/accesskit/blob/main/LICENSE-MIT
- profiling: https://github.com/aclysma/profiling/blob/master/LICENSE-MIT

`Hack-Regular.txt`, `OFL.txt`, `UFL.txt`, and
`emoji-icon-font-mit-license.txt` come from `epaint_default_fonts` 0.35.0's
`fonts/` directory. Noto Emoji and Ubuntu Light's copyright/designer names
were read from the bundled font files' `name` tables and prepended to their
licence notices. No copyright line was inferred.

`raylib-components.txt` preserves the licence-bearing comment blocks from the
vendored raylib external headers plus GLFW's licence and contributor list.
This deliberately includes notices for optional vendored components too.
The app icon derives from Musializer's MIT-covered logo (`tools/make_rust_logo.py`);
the top-bar mark is procedural. No outside logo artwork is added to About.
User-imported/Google Fonts downloads are external assets, not bundled fonts.

External tool licences are identified separately in the credits table; the tools,
model weights, agent programs and remote services are not shipped in this binary.
FFmpeg's licence depends on its build options. MMS_FA is CC-BY-NC-4.0 per
https://docs.pytorch.org/audio/stable/generated/torchaudio.pipelines.MMS_FA.html .
Python uses PSF-2.0, NumPy BSD-3-Clause, Whisper MIT, and the external
PyTorch/TorchAudio installations their upstream BSD-style/BSD-2-Clause terms.
Sources: https://docs.python.org/3/license.html ,
https://numpy.org/doc/stable/license.html ,
https://github.com/openai/whisper/blob/main/LICENSE ,
https://github.com/pytorch/pytorch/blob/main/LICENSE , and
https://github.com/pytorch/audio/blob/main/LICENSE .
Configured agent/provider terms vary and are deliberately not assigned a licence.
