//! Builds raylib 5.5 from the source vendored at `vendor/raylib-5.5/` and emits
//! the link directives for it.
//!
//! Why this crate exists: `raylib-sys` is used in `nobuild` mode, so it
//! generates bindings but neither builds nor links a library. This crate is the
//! other half. Keeping it separate means the raylib version is ours, not a side
//! effect of the binding crate's version — `raylib-sys` 5.5.1 actually vendors
//! raylib **5.6-dev**, which is exactly the coupling the plan wanted to avoid.
//!
//! The compile flags mirror `../musializer/src_build/nob_linux.c:98-150` so the
//! library under the Rust renderer is the same one the parity oracle was built
//! against:
//!
//! ```text
//! cc -ggdb -DPLATFORM_DESKTOP -D_GLFW_X11 -fPIC -DSUPPORT_FILEFORMAT_FLAC=1 \
//!    -I<src>/external/glfw/include -c <module>.c
//! ar -crs libraylib.a <objects>
//! ```
//!
//! The oracle links only `-lm -ldl -lpthread` alongside the static library
//! (`nob_linux.c:74,86-90`): GLFW 3.4 under `_GLFW_X11` `dlopen`s libX11 and
//! the GL driver at runtime rather than linking them, so no X11/GL link flag is
//! needed here either.

use std::path::PathBuf;

/// The translation units the oracle compiles, in its order.
/// (`../musializer/src_build/nob_stage2.c:10-19`)
const RAYLIB_MODULES: &[&str] = &[
    "rcore",
    "raudio",
    "rglfw",
    "rmodels",
    "rshapes",
    "rtext",
    "rtextures",
    "utils",
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
        .expect("crate lives at <workspace>/crates/raylib-5-5-link");
    let raylib_src = workspace_root.join("vendor/raylib-5.5/src");

    assert!(
        raylib_src.join("raylib.h").is_file(),
        "vendored raylib source missing at {}. It is copied from \
         ../musializer/thirdparty/raylib-5.5/ — upstream source, not a build artifact.",
        raylib_src.display()
    );

    // rcore.c textually includes the GLFW platform backend, so a change there
    // has to invalidate the rcore object. The oracle tracks the same edge for
    // the same reason (nob_stage2.c:21-32).
    println!("cargo:rerun-if-changed=build.rs");
    for module in RAYLIB_MODULES {
        println!("cargo:rerun-if-changed={}/{module}.c", raylib_src.display());
    }
    println!(
        "cargo:rerun-if-changed={}/platforms/rcore_desktop_glfw.c",
        raylib_src.display()
    );

    let mut build = cc::Build::new();
    build
        .define("PLATFORM_DESKTOP", None)
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
    // this from DEP_RAYLIB_INCLUDE thanks to `links = "raylib"`.
    println!("cargo:include={}", raylib_src.display());
    println!("cargo:version=5.5");
}
