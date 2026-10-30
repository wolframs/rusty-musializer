//! Source-audited attribution. All notices travel inside the executable.
//! Registry notices are copied verbatim under resources/licenses; no runtime I/O.

#[derive(Clone, Copy, Debug)]
pub struct Credit {
    pub group: &'static str,
    pub name: &'static str,
    pub author: &'static str,
    pub license: Option<&'static str>,
    pub urls: &'static [&'static str],
    pub notice: Option<&'static str>,
    pub dependency: Option<&'static str>,
    pub font: Option<&'static str>,
}

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const CREDITS: &[Credit] = &[
    Credit {
        group: "Original project",
        name: "Musializer",
        author: "Alexey Kutepov (Tsoding) and the Musializer contributors",
        license: Some("MIT"),
        urls: &["https://github.com/tsoding/musializer"],
        notice: Some(include_str!("../../../LICENSE")),
        dependency: None,
        font: None,
    },
    Credit {
        group: "Scene adapted with permission",
        name: "Phosphor Dream / dreamscape.py",
        author: "Digi",
        license: Some("Adapted with permission"),
        urls: &[
            "https://github.com/digi-the-robot",
            "https://x.com/digi_dot_exe",
            "https://linktr.ee/digi_the_robot",
        ],
        notice: None,
        dependency: None,
        font: None,
    },
    Credit {
        group: "Libraries",
        name: "raylib 6.0",
        author: "Ramon Santamaria (@raysan5)",
        license: Some("Zlib"),
        urls: &["https://www.raylib.com"],
        notice: Some(include_str!("../../../vendor/raylib-6.0/LICENSE")),
        dependency: None,
        font: None,
    },
    Credit {
        group: "Libraries",
        name: "egui 0.35.0",
        author: "Emil Ernerfeldt",
        license: Some("MIT OR Apache-2.0"),
        urls: &["https://github.com/emilk/egui"],
        notice: Some(include_str!("../../../resources/licenses/egui-MIT.txt")),
        dependency: Some("egui"),
        font: None,
    },
    Credit {
        group: "Libraries",
        name: "raylib 6.0.0",
        author: "raylib-rs team",
        license: Some("Zlib"),
        urls: &["https://github.com/raylib-rs/raylib-rs"],
        notice: Some(include_str!("../../../resources/licenses/raylib-rs.txt")),
        dependency: Some("raylib"),
        font: None,
    },
    Credit {
        group: "Libraries",
        name: "raylib-sys 6.0.0",
        author: "raylib-rs team",
        license: Some("Zlib"),
        urls: &["https://github.com/raylib-rs/raylib-rs"],
        notice: Some(include_str!("../../../resources/licenses/raylib-rs.txt")),
        dependency: Some("raylib-sys"),
        font: None,
    },
    Credit {
        group: "Libraries",
        name: "thiserror 2.0.19",
        author: "David Tolnay",
        license: Some("MIT OR Apache-2.0"),
        urls: &["https://github.com/dtolnay/thiserror"],
        notice: Some(include_str!("../../../resources/licenses/thiserror.txt")),
        dependency: Some("thiserror"),
        font: None,
    },
    Credit {
        group: "Libraries",
        name: "anyhow 1.0.104",
        author: "David Tolnay",
        license: Some("MIT OR Apache-2.0"),
        urls: &["https://github.com/dtolnay/anyhow"],
        notice: Some(include_str!("../../../resources/licenses/anyhow.txt")),
        dependency: Some("anyhow"),
        font: None,
    },
    Credit {
        group: "Libraries",
        name: "serde 1.0.229",
        author: "Erick Tryzelaar, David Tolnay",
        license: Some("MIT OR Apache-2.0"),
        urls: &["https://github.com/serde-rs/serde"],
        notice: Some(include_str!("../../../resources/licenses/serde.txt")),
        dependency: Some("serde"),
        font: None,
    },
    Credit {
        group: "Libraries",
        name: "serde_json 1.0.151",
        author: "Erick Tryzelaar, David Tolnay",
        license: Some("MIT OR Apache-2.0"),
        urls: &["https://github.com/serde-rs/json"],
        notice: Some(include_str!("../../../resources/licenses/serde_json.txt")),
        dependency: Some("serde_json"),
        font: None,
    },
    Credit {
        group: "Libraries",
        name: "sha2 0.10.9",
        author: "RustCrypto Developers",
        license: Some("MIT OR Apache-2.0"),
        urls: &["https://github.com/RustCrypto/hashes"],
        notice: Some(include_str!("../../../resources/licenses/sha2.txt")),
        dependency: Some("sha2"),
        font: None,
    },
    Credit {
        group: "Libraries",
        name: "tempfile 3.27.0 (tests only)",
        author: "Steven Allen, The Rust Project Developers, Ashley Mannix, Jason White",
        license: Some("MIT OR Apache-2.0"),
        urls: &["https://github.com/Stebalien/tempfile"],
        notice: Some(include_str!("../../../resources/licenses/tempfile.txt")),
        dependency: Some("tempfile"),
        font: None,
    },
    Credit {
        group: "Libraries",
        name: "clap 4.6.4",
        author: "The clap developers",
        license: Some("MIT OR Apache-2.0"),
        urls: &["https://github.com/clap-rs/clap"],
        notice: Some(include_str!("../../../resources/licenses/clap.txt")),
        dependency: Some("clap"),
        font: None,
    },
    Credit {
        group: "Libraries",
        name: "alacritty_terminal 0.26.0",
        author: "Christian Duerr, Joe Wilm",
        license: Some("Apache-2.0"),
        urls: &["https://github.com/alacritty/alacritty"],
        notice: Some(include_str!(
            "../../../resources/licenses/alacritty_terminal.txt"
        )),
        dependency: Some("alacritty_terminal"),
        font: None,
    },
    Credit {
        group: "Libraries",
        name: "cc 1.4.0 (build only)",
        author: "Alex Crichton",
        license: Some("MIT OR Apache-2.0"),
        urls: &["https://github.com/rust-lang/cc-rs"],
        notice: Some(include_str!("../../../resources/licenses/cc.txt")),
        dependency: Some("cc"),
        font: None,
    },
    Credit {
        group: "Libraries",
        name: "egui-raylib 0.1.3 (adapted bridge)",
        author: "Yoppez",
        license: Some("MIT"),
        urls: &["https://github.com/Yoppez/egui-raylib"],
        notice: Some(include_str!(
            "../../musializer-app/src/ui/egui_backend/LICENSE"
        )),
        dependency: None,
        font: None,
    },
    Credit {
        group: "Libraries",
        name: "Additional dependency notices",
        author: "Contributors named in the embedded notices",
        license: Some("Per component"),
        urls: &["https://crates.io"],
        notice: Some(include_str!(
            "../../../resources/licenses/locked-dependencies.txt"
        )),
        dependency: None,
        font: None,
    },
    Credit {
        group: "Libraries",
        name: "raylib component notices (GLFW, miniaudio, stb, dr_libs and others)",
        author: "Authors named in the embedded component notices",
        license: Some("Per component; see notices"),
        urls: &["https://github.com/raysan5/raylib"],
        notice: Some(include_str!(
            "../../../resources/licenses/raylib-components.txt"
        )),
        dependency: None,
        font: None,
    },
    Credit {
        group: "Fonts",
        name: "Alegreya",
        author: "The Alegreya Project Authors",
        license: Some("OFL-1.1"),
        urls: &["https://github.com/huertatipografica/Alegreya"],
        notice: Some(include_str!("../../../resources/fonts/OFL.txt")),
        dependency: None,
        font: Some("Alegreya-Regular.ttf"),
    },
    Credit {
        group: "Fonts",
        name: "Space Grotesk",
        author: "The Space Grotesk Project Authors",
        license: Some("OFL-1.1"),
        urls: &["https://github.com/floriankarsten/space-grotesk"],
        notice: Some(include_str!(
            "../../../resources/fonts/SpaceGrotesk-OFL.txt"
        )),
        dependency: None,
        font: Some("SpaceGrotesk-Regular.otf"),
    },
    Credit {
        group: "Fonts",
        name: "Font Awesome 4",
        author: "Dave Gandy",
        license: Some("OFL-1.1"),
        urls: &["https://fontawesome.com"],
        notice: Some(include_str!("../../../resources/fonts/FontAwesome-OFL.txt")),
        dependency: None,
        font: Some("FontAwesome.otf"),
    },
    Credit {
        group: "Fonts",
        name: "Hack (egui fallback)",
        author: "Source Foundry Authors; Bitstream Inc.; DejaVu contributors",
        license: Some("MIT / Bitstream Vera / public domain"),
        urls: &["https://github.com/source-foundry/Hack"],
        notice: Some(include_str!("../../../resources/licenses/Hack-Regular.txt")),
        dependency: None,
        font: None,
    },
    Credit {
        group: "Fonts",
        name: "Noto Emoji (egui fallback)",
        author: "Google Inc.; Monotype Design Team",
        license: Some("OFL-1.1"),
        urls: &["https://github.com/googlefonts/noto-emoji"],
        notice: Some(include_str!("../../../resources/licenses/OFL.txt")),
        dependency: None,
        font: None,
    },
    Credit {
        group: "Fonts",
        name: "Ubuntu Light (egui fallback)",
        author: "Canonical Ltd.; Dalton Maag Ltd",
        license: Some("Ubuntu-font-1.0"),
        urls: &["https://design.ubuntu.com/font"],
        notice: Some(include_str!("../../../resources/licenses/UFL.txt")),
        dependency: None,
        font: None,
    },
    Credit {
        group: "Fonts",
        name: "emoji-icon-font (egui fallback)",
        author: "John Slegers",
        license: Some("MIT"),
        urls: &["https://github.com/jslegers/emoji-icon-font"],
        notice: Some(include_str!(
            "../../../resources/licenses/emoji-icon-font-mit-license.txt"
        )),
        dependency: None,
        font: None,
    },
    Credit {
        group: "Optional tools",
        name: "FFmpeg / ffprobe — not bundled",
        author: "The FFmpeg developers",
        license: Some("LGPL-2.1-or-later; build options may enable GPL"),
        urls: &["https://ffmpeg.org/legal.html"],
        notice: None,
        dependency: None,
        font: None,
    },
    Credit {
        group: "Optional tools",
        name: "Python — not bundled",
        author: "Python Software Foundation",
        license: Some("PSF-2.0"),
        urls: &["https://www.python.org"],
        notice: None,
        dependency: None,
        font: None,
    },
    Credit {
        group: "Optional tools",
        name: "NumPy — not bundled",
        author: "NumPy developers",
        license: Some("BSD-3-Clause"),
        urls: &["https://numpy.org"],
        notice: None,
        dependency: None,
        font: None,
    },
    Credit {
        group: "Optional tools",
        name: "whisper.cpp — not bundled",
        author: "The ggml authors",
        license: Some("MIT"),
        urls: &["https://github.com/ggml-org/whisper.cpp"],
        notice: None,
        dependency: None,
        font: None,
    },
    Credit {
        group: "Optional tools",
        name: "Whisper models — not bundled",
        author: "OpenAI",
        license: Some("MIT"),
        urls: &["https://github.com/openai/whisper"],
        notice: None,
        dependency: None,
        font: None,
    },
    Credit {
        group: "Optional tools",
        name: "PyTorch / TorchAudio — not bundled",
        author: "PyTorch contributors",
        license: Some("PyTorch BSD-style; TorchAudio BSD-2-Clause"),
        urls: &["https://pytorch.org/audio"],
        notice: None,
        dependency: None,
        font: None,
    },
    Credit {
        group: "Optional tools",
        name: "MMS_FA model — not bundled",
        author: "Meta",
        license: Some("CC-BY-NC-4.0"),
        urls: &["https://pytorch.org/audio/stable/generated/torchaudio.pipelines.MMS_FA.html"],
        notice: None,
        dependency: None,
        font: None,
    },
    Credit {
        group: "Optional tools",
        name: "Demucs (research stem controls) — not bundled",
        author: "Meta / Demucs contributors",
        license: Some("MIT"),
        urls: &["https://github.com/facebookresearch/demucs"],
        notice: None,
        dependency: None,
        font: None,
    },
    Credit {
        group: "Optional tools",
        name: "Codex / Claude Code / Antigravity — not bundled",
        author: "OpenAI / Anthropic / Google",
        license: None,
        urls: &["https://developers.openai.com/codex"],
        notice: None,
        dependency: None,
        font: None,
    },
    Credit {
        group: "Optional tools",
        name: "Remote Assist models (configured provider) — not bundled",
        author: "Provider selected in Assist",
        license: None,
        urls: &["https://openrouter.ai"],
        notice: None,
        dependency: None,
        font: None,
    },
    Credit {
        group: "Inspired by",
        name: "Le Biniou feedback family (ideas only)",
        author: "Le Biniou contributors",
        license: None,
        urls: &["https://biniou.net"],
        notice: None,
        dependency: None,
        font: None,
    },
    Credit {
        group: "Inspired by",
        name: "WebGL Fluid demo (ideas only)",
        author: "WebGL Fluid demo authors",
        license: None,
        urls: &["https://webdev.teknium.io/d/gl/demos/webgl-fluid-ink/"],
        notice: None,
        dependency: None,
        font: None,
    },
    Credit {
        group: "Inspired by",
        name: "Glyph Lattice visual reference (ideas only)",
        author: "Modal",
        license: None,
        urls: &["https://modal.com"],
        notice: None,
        dependency: None,
        font: None,
    },
    Credit {
        group: "Inspired by",
        name: "Clawd flower character (procedural homage)",
        author: "thebes",
        license: None,
        urls: &["https://x.com/voooooogel"],
        notice: None,
        dependency: None,
        font: None,
    },
    Credit {
        group: "Inspired by",
        name: "Stable Fluids method",
        author: "Jos Stam",
        license: None,
        urls: &["https://doi.org/10.1145/311535.311548"],
        notice: None,
        dependency: None,
        font: None,
    },
    Credit {
        group: "Inspired by",
        name: "Six-ribbon geometry study (visual reference)",
        author: "OpenAI",
        license: None,
        urls: &["https://openai.com/brand/"],
        notice: None,
        dependency: None,
        font: None,
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::BTreeSet, fs, path::Path};

    #[test]
    fn every_workspace_direct_dependency_has_exactly_one_credit() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut manifests = vec![root.join("Cargo.toml")];
        for member in fs::read_dir(root.join("crates")).unwrap() {
            let manifest = member.unwrap().path().join("Cargo.toml");
            if manifest.exists() {
                manifests.push(manifest);
            }
        }
        let internal = [
            "musializer-core",
            "musializer-app",
            "musializer-runtime",
            "raylib-link",
        ];
        let mut expected = BTreeSet::new();
        for manifest in manifests {
            let mut dependencies = false;
            for line in fs::read_to_string(manifest).unwrap().lines() {
                let line = line.trim();
                if line.starts_with('[') {
                    dependencies = line.contains("dependencies]");
                } else if dependencies && !line.starts_with('#') {
                    if let Some((key, _)) = line.split_once('=') {
                        let name = key.trim().trim_end_matches(".workspace");
                        if !internal.contains(&name) {
                            expected.insert(name.to_owned());
                        }
                    }
                }
            }
        }
        let actual: Vec<_> = CREDITS.iter().filter_map(|c| c.dependency).collect();
        let unique: BTreeSet<_> = actual.iter().map(|s| s.to_string()).collect();
        assert_eq!(
            actual.len(),
            unique.len(),
            "duplicate dependency attribution"
        );
        assert_eq!(unique, expected, "workspace dependency attribution drift");
    }

    #[test]
    fn every_locked_registry_package_has_an_embedded_notice() {
        use sha2::{Digest, Sha256};
        let lock = include_str!("../../../Cargo.lock");
        let bundle = CREDITS
            .iter()
            .find(|c| c.name == "Additional dependency notices")
            .unwrap()
            .notice
            .unwrap();
        let expected: BTreeSet<String> = lock
            .split("[[package]]")
            .skip(1)
            .filter(|block| {
                block
                    .lines()
                    .any(|line| line.starts_with("source = \"registry+"))
            })
            .map(|block| {
                let field = |key: &str| {
                    block
                        .lines()
                        .find_map(|line| line.strip_prefix(key))
                        .unwrap()
                        .trim_matches('"')
                };
                format!("{} {}", field("name = "), field("version = "))
            })
            .collect();
        let rows: Vec<_> = bundle
            .lines()
            .filter_map(|line| line.strip_prefix("Package: "))
            .map(str::to_owned)
            .collect();
        let actual: BTreeSet<_> = rows.iter().cloned().collect();
        assert_eq!(rows.len(), actual.len(), "duplicate package notice");
        assert_eq!(actual, expected, "locked dependency notices are incomplete");
        assert!(
            bundle.contains(&format!(
                "Cargo.lock SHA-256: {:x}",
                Sha256::digest(lock.as_bytes())
            )),
            "refresh notices after Cargo.lock changes"
        );
    }

    #[test]
    fn every_embedded_project_font_has_exactly_one_credit() {
        let fonts = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources/fonts");
        let expected: BTreeSet<_> = fs::read_dir(fonts)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| {
                matches!(
                    p.extension()
                        .and_then(|e| e.to_str())
                        .map(str::to_ascii_lowercase)
                        .as_deref(),
                    Some("ttf" | "otf" | "ttc" | "otc" | "woff" | "woff2" | "bdf" | "fnt")
                )
            })
            .map(|p| p.file_name().unwrap().to_str().unwrap().to_owned())
            .collect();
        let actual: Vec<_> = CREDITS.iter().filter_map(|c| c.font).collect();
        let unique: BTreeSet<_> = actual.iter().map(|s| s.to_string()).collect();
        assert_eq!(actual.len(), unique.len(), "duplicate font attribution");
        assert_eq!(unique, expected, "embedded font attribution drift");
    }

    #[test]
    fn redistribution_notices_and_digis_requested_handles_are_present() {
        for c in CREDITS
            .iter()
            .filter(|c| matches!(c.group, "Original project" | "Libraries" | "Fonts"))
        {
            assert!(
                c.notice.is_some_and(|text| text.len() > 500),
                "missing notice: {}",
                c.name
            );
        }
        let digi = CREDITS.iter().find(|c| c.author == "Digi").unwrap();
        assert_eq!(
            digi.urls,
            [
                "https://github.com/digi-the-robot",
                "https://x.com/digi_dot_exe",
                "https://linktr.ee/digi_the_robot"
            ]
        );
        assert!(CREDITS
            .iter()
            .find(|c| c.font == Some("Alegreya-Regular.ttf"))
            .unwrap()
            .notice
            .unwrap()
            .starts_with("Copyright 2011 The Alegreya Project Authors"));
    }
}
