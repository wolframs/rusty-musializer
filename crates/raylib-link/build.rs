//! Builds the pinned upstream raylib 6.0 source with our audio diagnostic patch.
//! raylib-sys uses nobuild, so this crate supplies the actual C library.
//! See docs/RAYLIB_6_UPGRADE.md for provenance and integration contracts.

use std::path::PathBuf;

/// raylib 6.0 translation units (utils was folded into rcore).
const RAYLIB_MODULES: &[&str] = &[
    "rcore",
    "raudio",
    "rglfw",
    "rmodels",
    "rshapes",
    "rtext",
    "rtextures",
];

fn main() {
    // `.cargo/config.toml` puts `vendor/clang-builtin-shim` on CPATH so bindgen's
    // libclang can find the compiler-provided headers Ubuntu's libclang1 package
    // omits. GCC honours CPATH too, and those shim headers are only complete
    // enough to satisfy a header *parse* — letting them shadow GCC's real
    // `stddef.h` while compiling 15 MB of raylib would be a bad trade. Drop it
    // here so the shim reaches libclang and nothing else.
    std::env::remove_var("CPATH");

    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let workspace_root = manifest
        .parent()
        .and_then(|p| p.parent())
        .expect("crate lives at <workspace>/crates/raylib-link");
    let raylib_src = workspace_root.join("vendor/raylib-6.0/src");

    assert!(
        raylib_src.join("raylib.h").is_file(),
        "vendored raylib source missing at {}. See docs/RAYLIB_6_UPGRADE.md.",
        raylib_src.display()
    );

    // rcore.c textually includes the GLFW platform backend, so a change there
    // has to invalidate the rcore object. Track all headers and bundled sources.
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={}", raylib_src.display());
    for module in RAYLIB_MODULES {
        println!("cargo:rerun-if-changed={}/{module}.c", raylib_src.display());
    }
    println!(
        "cargo:rerun-if-changed={}/platforms/rcore_desktop_glfw.c",
        raylib_src.display()
    );

    let mut build = cc::Build::new();
    build
        .define("PLATFORM_DESKTOP_GLFW", None)
        .define("_GLFW_X11", None)
        .define("SUPPORT_FILEFORMAT_FLAC", "1")
        .include(raylib_src.join("external/glfw/include"))
        .include(&raylib_src)
        .flag_if_supported("-fPIC")
        // raylib and its bundled decoders are third-party code that compiles
        // with warnings under -Wall. They are not ours to fix; silence them so
        // a real warning in first-party code stays visible.
        .warnings(false)
        .extra_warnings(false);

    for module in RAYLIB_MODULES {
        build.file(raylib_src.join(format!("{module}.c")));
    }

    // Emits `cargo:rustc-link-lib=static=raylib` and the search path.
    build.compile("raylib");

    for lib in ["m", "dl", "pthread"] {
        println!("cargo:rustc-link-lib=dylib={lib}");
    }

    // Consumers that need the headers (bindgen, a hand-rolled shim) can read
    // this from DEP_MUSIALIZER_RAYLIB_INCLUDE thanks to `links = "musializer_raylib"`.
    println!("cargo:include={}", raylib_src.display());
    println!("cargo:version=6.0");
}
