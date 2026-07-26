//! Content-addressed bundling for the `audio`, `images` and `fonts` categories.
//!
//! **Owner: Agent B.** Port of the asset-bundling paths in
//! `../musializer/src/project_io.c` (`musi_project_asset_category_directory`,
//! `project_bundle_paths`, `project_safe_extension`) and of the licence coupling
//! the caption face depends on.
//!
//! An asset lives at `<stem>.assets/<category>/<sha256><.ext>` beside the project
//! file, so the bytes are addressed by their content and two projects that import
//! the same file share one copy. This module computes those names and nothing
//! else: the copying, `fsync`, hard-link publication and collision handling in
//! `project_copy_asset_transaction` open files, so they belong to
//! `musializer-runtime` (Agent E owns atomic publication). The failure enums are
//! here because the *policy* they describe is this module's.
//!
//! The caption face is why this matters for correctness rather than tidiness: an
//! imported face is bundled **content-addressed together with its licence file**,
//! and a project whose face is `imported` without that asset — or which carries
//! the asset without the face being `imported` — is invalid. Captions must be
//! reproducible from the file alone.

use crate::project::model::{is_bundled_relative_path, FontAsset};
use crate::project::sha256;

/// Every category the bundle machinery will publish into
/// (`Musi_Project_Asset_Category`, `project_io.h:54-58`).
///
/// C defines "valid category" as "the directory lookup returned non-NULL"
/// (`project_io.c:1267-1270`) so that adding a category without adding its
/// directory is a compile-time-silent, runtime-rejected mistake. A Rust `match`
/// over this enum is exhaustive, so the mistake is a compile error instead — which
/// is the whole reason `musi_project_asset_category_valid` has no counterpart.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AssetCategory {
    Audio,
    Image,
    Font,
}

impl AssetCategory {
    pub const ALL: [AssetCategory; 3] = [
        AssetCategory::Audio,
        AssetCategory::Image,
        AssetCategory::Font,
    ];

    /// The `<stem>.assets/` subdirectory a category is stored under
    /// (`musi_project_asset_category_directory`, `project_io.c:1272-1281`).
    ///
    /// These strings are on disk in every existing project. Renaming one orphans
    /// every bundled asset of that kind.
    #[must_use]
    pub fn directory(self) -> &'static str {
        match self {
            AssetCategory::Audio => "audio",
            AssetCategory::Image => "images",
            AssetCategory::Font => "fonts",
        }
    }
}

/// How bundling failed (`Musi_Project_Bundle_Result`, `project_io.h:67-78`).
///
/// The granularity is the point. `Identity` means the copy did not hash to what was
/// promised; `Collision` means the content-addressed destination already holds
/// *different* bytes, which is either a hash collision or a corrupted bundle and in
/// both cases must not be overwritten. Folding those two into one "copy failed"
/// would lose the only signal that a bundle is damaged.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum BundleError {
    #[error("invalid bundle argument")]
    Argument,
    #[error("asset bundle path is too long or malformed")]
    Path,
    #[error("asset bundle directory could not be created")]
    Directory,
    #[error("source asset is missing or changed identity")]
    Source,
    #[error("asset could not be copied completely")]
    Copy,
    #[error("asset copy could not be made durable")]
    Sync,
    #[error("copied asset did not match its expected SHA-256")]
    Identity,
    #[error("content-addressed destination contains different data")]
    Collision,
    #[error("asset could not be published")]
    Publish,
}

/// Where one bundled asset goes (`project_bundle_paths`, `project_io.c:1283-1333`).
///
/// `stored` is what the `.musi` records — project-relative and portable. `runtime`
/// is where it actually is on this machine. `root` and `category_directory` are the
/// two directories that must exist first, in that order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BundlePaths {
    pub stored: String,
    pub runtime: String,
    pub root: String,
    pub category_directory: String,
}

/// `project_safe_extension` (`project_io.c:1245-1265`).
///
/// The extension is lowercased and restricted to ASCII alphanumerics, because it
/// becomes part of a filename this code creates. Anything else — a dot file, a
/// multi-dot suffix with punctuation, an over-long tail — yields no extension at
/// all rather than a sanitized guess, and the asset is stored as a bare digest.
///
/// The returned string includes its leading dot, or is empty.
#[must_use]
pub fn safe_extension(path: &str) -> String {
    let name = match path.rfind(['/', '\\']) {
        Some(at) => &path[at + 1..],
        None => path,
    };
    let Some(dot) = name.rfind('.') else {
        return String::new();
    };
    if dot == 0 {
        // A leading dot is a hidden file, not an extension.
        return String::new();
    }
    let suffix = &name[dot..];
    if suffix.len() < 2 || suffix.len() >= 18 {
        return String::new();
    }
    let mut out = String::with_capacity(suffix.len());
    out.push('.');
    for byte in suffix[1..].bytes() {
        let lowered = byte.to_ascii_lowercase();
        if !lowered.is_ascii_alphanumeric() {
            return String::new();
        }
        out.push(lowered as char);
    }
    out
}

/// `project_bundle_paths` (`project_io.c:1283-1333`).
///
/// `project_path` must name a file with an extension: the stem is what
/// `<stem>.assets/` is built from, so a project path with no dot has no bundle and
/// this returns [`BundleError::Path`] rather than inventing one.
///
/// `sha256` is the *expected* digest of the source, and it becomes the filename.
/// This function does not read the file — verifying that promise is the runtime's
/// job, and it must do it before publication.
pub fn bundle_paths(
    project_path: &str,
    category: AssetCategory,
    source_path: &str,
    sha256_hex: &str,
) -> Result<BundlePaths, BundleError> {
    if project_path.is_empty() || source_path.is_empty() {
        return Err(BundleError::Argument);
    }
    if !sha256::is_hex_digest(sha256_hex) {
        return Err(BundleError::Argument);
    }
    let separator = project_path.rfind(['/', '\\']);
    let filename = match separator {
        Some(at) => &project_path[at + 1..],
        None => project_path,
    };
    let dot = filename
        .rfind('.')
        .filter(|at| *at > 0)
        .ok_or(BundleError::Path)?;
    let stem = &filename[..dot];

    let directory = match separator {
        None => ".",
        Some(0) => &project_path[..1],
        Some(at) => &project_path[..at],
    };
    let join = if directory.ends_with('/') || directory.ends_with('\\') {
        ""
    } else {
        "/"
    };

    let extension = safe_extension(source_path);
    let category_name = category.directory();
    let stored = format!("{stem}.assets/{category_name}/{sha256_hex}{extension}");
    let runtime = format!("{directory}{join}{stored}");
    let root = format!("{directory}{join}{stem}.assets");
    let category_directory = format!("{root}/{category_name}");
    Ok(BundlePaths {
        stored,
        runtime,
        root,
        category_directory,
    })
}

/// Why a caption face's bundle is not reproducible.
///
/// These are the shapes [`caption_font_bundle_paths`] refuses, separated from
/// [`BundleError`] because they are model faults rather than filesystem ones.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CaptionBundleError {
    #[error("the font asset is not internally valid")]
    Asset,
    #[error("the caption face is not stored inside the project's asset bundle")]
    OutsideBundle,
    #[error("the caption face and its licence disagree about whether one is bundled")]
    LicenceCoupling,
}

/// The two bundle entries an imported caption face needs: the face, and its licence
/// if one travels with it.
///
/// This exists so no caller can bundle a face and forget its licence. Copying a
/// face into `<stem>.assets/fonts/` and handing someone the project is
/// redistribution, and the OFL asks that its text travel with the bytes
/// (`project.h:150-155`). A face imported from the user's own disk legitimately has
/// no licence — but then it has *no* licence fields at all, and the caller gets
/// `None` for the second entry rather than a half-filled one.
pub fn caption_font_bundle_paths(
    project_path: &str,
    font: &FontAsset,
    face_source_path: &str,
    licence_source_path: Option<&str>,
) -> Result<(BundlePaths, Option<BundlePaths>), CaptionBundleError> {
    if !font.is_valid() {
        return Err(CaptionBundleError::Asset);
    }
    if font.has_licence() != licence_source_path.is_some() {
        return Err(CaptionBundleError::LicenceCoupling);
    }
    let face = bundle_paths(
        project_path,
        AssetCategory::Font,
        face_source_path,
        &font.sha256,
    )
    .map_err(|_| CaptionBundleError::Asset)?;
    if !is_bundled_relative_path(&face.stored) {
        return Err(CaptionBundleError::OutsideBundle);
    }
    let licence = match licence_source_path {
        None => None,
        Some(source) => {
            let paths = bundle_paths(
                project_path,
                AssetCategory::Font,
                source,
                &font.licence_sha256,
            )
            .map_err(|_| CaptionBundleError::Asset)?;
            if !is_bundled_relative_path(&paths.stored) {
                return Err(CaptionBundleError::OutsideBundle);
            }
            Some(paths)
        }
    };
    Ok((face, licence))
}

/// The bytes ceiling on an imported caption face
/// (`CAPTION_IMPORTED_FONT_BYTE_LIMIT`, `plug.c:369`, mirrored by
/// `font-import-v1.schema.json:44-49`).
///
/// Bounded so a redirect to something enormous fails before it fills a disk.
pub const IMPORTED_FONT_BYTE_LIMIT: u64 = 33_554_432;

/// What `tools/google_fonts.py` writes after retrieving one caption face
/// (`schemas/font-import-v1.schema.json`).
///
/// Reading the JSON is Agent E's (it supervises the child and reads a bounded
/// manifest); the *coupling rules* are here, because they are what decides whether
/// a [`FontAsset`] may be built from a download.
///
/// The asymmetry with [`FontAsset`] is deliberate and load-bearing. An imported
/// *download* must carry a licence — closed enum, both digests mandatory — whereas
/// a project-bundled face may have come from the user's own disk and may
/// legitimately carry none. A face whose licence could not be retrieved is refused
/// rather than bundled without it, because copying it into a shareable project is
/// redistribution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontImportManifest {
    pub family: String,
    pub source: String,
    pub font_path: String,
    pub font_sha256: String,
    pub font_bytes: u64,
    pub licence_path: String,
    pub licence_sha256: String,
    pub licence_name: String,
}

/// The three licence names `google/fonts` sorts a family into
/// (`font-import-v1.schema.json:59-66`).
///
/// A closed enum, taken from the directory the family lives in and **not** guessed
/// from the licence text.
pub const FONT_IMPORT_LICENCE_NAMES: [&str; 3] = ["OFL-1.1", "Apache-2.0", "UFL-1.0"];

/// The family pattern (`font-import-v1.schema.json:23-29`):
/// `^[A-Za-z0-9][A-Za-z0-9 '+.-]*$`, 1..128 bytes.
///
/// Note what it permits beyond a `stable_name`: space, apostrophe, plus, dot and
/// hyphen — because that is how Google Fonts spells family names. It is a display
/// label only; the bytes are identified by their digest.
#[must_use]
pub fn is_font_import_family(value: &str) -> bool {
    if value.is_empty() || value.len() > 128 {
        return false;
    }
    let bytes = value.as_bytes();
    if !bytes[0].is_ascii_alphanumeric() {
        return false;
    }
    bytes.iter().all(|byte| {
        byte.is_ascii_alphanumeric() || matches!(byte, b' ' | b'\'' | b'+' | b'.' | b'-')
    })
}

impl FontImportManifest {
    /// Every field bound in the schema, checked.
    ///
    /// The manifest "is a description of a download, never a warrant for it"
    /// (`font-import-v1.schema.json:5`): the application re-computes both digests
    /// against the files on disk before either is trusted. This function checks the
    /// description; the runtime checks the files.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        is_font_import_family(&self.family)
            && self.source.starts_with("https://fonts.gstatic.com/")
            && !self.font_path.is_empty()
            && sha256::is_hex_digest(&self.font_sha256)
            && (1..=IMPORTED_FONT_BYTE_LIMIT).contains(&self.font_bytes)
            && !self.licence_path.is_empty()
            && sha256::is_hex_digest(&self.licence_sha256)
            && FONT_IMPORT_LICENCE_NAMES.contains(&self.licence_name.as_str())
    }

    /// The [`FontAsset`] a validated download becomes once bundled.
    ///
    /// `stored_font_path` and `stored_licence_path` are the project-relative paths
    /// [`caption_font_bundle_paths`] produced, because the asset records where the
    /// bytes ended up, not where they came from.
    pub fn to_font_asset(
        &self,
        stored_font_path: &str,
        stored_licence_path: &str,
    ) -> Option<FontAsset> {
        if !self.is_valid() {
            return None;
        }
        let asset = FontAsset {
            path: stored_font_path.to_owned(),
            sha256: self.font_sha256.clone(),
            family: self.family.clone(),
            licence_path: stored_licence_path.to_owned(),
            licence_sha256: self.licence_sha256.clone(),
            licence_name: self.licence_name.clone(),
        };
        // A download always carries a licence, so the bundled asset must too.
        (asset.is_valid() && asset.has_licence()).then_some(asset)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest_of(text: &str) -> String {
        sha256::digest_hex(text.as_bytes())
    }

    #[test]
    fn every_category_has_its_own_directory() {
        assert_eq!(AssetCategory::Audio.directory(), "audio");
        assert_eq!(AssetCategory::Image.directory(), "images");
        assert_eq!(AssetCategory::Font.directory(), "fonts");
        let mut seen: Vec<&str> = AssetCategory::ALL
            .iter()
            .map(|category| category.directory())
            .collect();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(
            seen.len(),
            AssetCategory::ALL.len(),
            "no two share a directory"
        );
    }

    #[test]
    fn an_asset_is_addressed_by_its_content_beside_the_project() {
        let sha = digest_of("song");
        let paths = bundle_paths(
            "/home/user/show.musi",
            AssetCategory::Audio,
            "/media/Song Name.WAV",
            &sha,
        )
        .unwrap();
        assert_eq!(paths.stored, format!("show.assets/audio/{sha}.wav"));
        assert_eq!(
            paths.runtime,
            format!("/home/user/show.assets/audio/{sha}.wav")
        );
        assert_eq!(paths.root, "/home/user/show.assets");
        assert_eq!(paths.category_directory, "/home/user/show.assets/audio");
        assert!(
            is_bundled_relative_path(&paths.stored),
            "the stored path must be re-savable by the editor"
        );
    }

    #[test]
    fn the_stored_path_is_identical_for_identical_content() {
        let sha = digest_of("same bytes");
        let first = bundle_paths("/a/show.musi", AssetCategory::Font, "/x/Face.TTF", &sha).unwrap();
        let second =
            bundle_paths("/a/show.musi", AssetCategory::Font, "/y/other.ttf", &sha).unwrap();
        assert_eq!(first.stored, second.stored, "content addressing, not name");
    }

    #[test]
    fn a_project_path_without_a_stem_has_no_bundle() {
        let sha = digest_of("x");
        for project in ["/home/user/noextension", "/home/user/.hidden", "show"] {
            assert_eq!(
                bundle_paths(project, AssetCategory::Audio, "/x/a.wav", &sha).unwrap_err(),
                BundleError::Path,
                "{project}"
            );
        }
    }

    #[test]
    fn bundle_paths_refuse_a_digest_that_is_not_one() {
        for sha in [
            "",
            &"a".repeat(63),
            &"a".repeat(65),
            &digest_of("x").to_uppercase(),
            "not a digest at all, but the right length.......................",
        ] {
            assert_eq!(
                bundle_paths("/a/show.musi", AssetCategory::Audio, "/x/a.wav", sha).unwrap_err(),
                BundleError::Argument,
                "{sha}"
            );
        }
    }

    #[test]
    fn a_project_in_the_working_directory_bundles_beside_itself() {
        let sha = digest_of("x");
        let paths = bundle_paths("show.musi", AssetCategory::Image, "art.PNG", &sha).unwrap();
        assert_eq!(paths.stored, format!("show.assets/images/{sha}.png"));
        assert_eq!(paths.runtime, format!("./show.assets/images/{sha}.png"));
        assert_eq!(paths.root, "./show.assets");
    }

    #[test]
    fn extensions_are_lowercased_and_anything_odd_is_dropped() {
        assert_eq!(safe_extension("/x/song.WAV"), ".wav");
        assert_eq!(safe_extension("song.mp3"), ".mp3");
        assert_eq!(safe_extension("archive.tar.gz"), ".gz");
        assert_eq!(safe_extension("song"), "");
        assert_eq!(safe_extension(".hidden"), "");
        assert_eq!(safe_extension("song."), "");
        assert_eq!(
            safe_extension("song.wav "),
            "",
            "a space is not alphanumeric"
        );
        assert_eq!(safe_extension("song.wa-v"), "");
        assert_eq!(safe_extension("song.wav/notafile"), "");
        assert_eq!(safe_extension(&format!("song.{}", "x".repeat(20))), "");
    }

    fn font_asset(licensed: bool) -> FontAsset {
        FontAsset {
            path: "show.assets/fonts/face.ttf".into(),
            sha256: digest_of("face"),
            family: "Some Family".into(),
            licence_path: if licensed {
                "show.assets/fonts/OFL.txt".into()
            } else {
                String::new()
            },
            licence_sha256: if licensed {
                digest_of("licence")
            } else {
                String::new()
            },
            licence_name: if licensed {
                "OFL-1.1".into()
            } else {
                String::new()
            },
        }
    }

    #[test]
    fn a_licensed_face_bundles_both_files_together() {
        let font = font_asset(true);
        let (face, licence) = caption_font_bundle_paths(
            "/home/user/show.musi",
            &font,
            "/downloads/Face.ttf",
            Some("/downloads/OFL.txt"),
        )
        .unwrap();
        let licence = licence.expect("the licence travels with the face");
        assert_eq!(
            face.stored,
            format!("show.assets/fonts/{}.ttf", font.sha256)
        );
        assert_eq!(
            licence.stored,
            format!("show.assets/fonts/{}.txt", font.licence_sha256)
        );
        assert_eq!(face.category_directory, licence.category_directory);
    }

    #[test]
    fn a_face_from_the_users_own_disk_may_carry_no_licence() {
        let font = font_asset(false);
        let (_, licence) =
            caption_font_bundle_paths("/a/show.musi", &font, "/x/face.ttf", None).unwrap();
        assert!(licence.is_none());
    }

    #[test]
    fn a_face_and_its_licence_may_not_disagree_about_whether_one_exists() {
        // Licence fields present but no source file offered.
        assert_eq!(
            caption_font_bundle_paths("/a/show.musi", &font_asset(true), "/x/face.ttf", None)
                .unwrap_err(),
            CaptionBundleError::LicenceCoupling
        );
        // A source file offered for a font asset that records no licence.
        assert_eq!(
            caption_font_bundle_paths(
                "/a/show.musi",
                &font_asset(false),
                "/x/face.ttf",
                Some("/x/OFL.txt")
            )
            .unwrap_err(),
            CaptionBundleError::LicenceCoupling
        );
    }

    #[test]
    fn an_internally_invalid_font_asset_is_never_bundled() {
        let mut font = font_asset(true);
        font.licence_name = String::new();
        assert_eq!(
            caption_font_bundle_paths("/a/show.musi", &font, "/x/face.ttf", Some("/x/OFL.txt"))
                .unwrap_err(),
            CaptionBundleError::Asset
        );
    }

    fn manifest() -> FontImportManifest {
        FontImportManifest {
            family: "Space Grotesk".into(),
            source: "https://fonts.gstatic.com/s/spacegrotesk/v1/font.ttf".into(),
            font_path: "job/font.ttf".into(),
            font_sha256: digest_of("face"),
            font_bytes: 120_000,
            licence_path: "job/OFL.txt".into(),
            licence_sha256: digest_of("licence"),
            licence_name: "OFL-1.1".into(),
        }
    }

    #[test]
    fn a_font_import_manifest_is_bounded_field_by_field() {
        assert!(manifest().is_valid());
        type Breaker = fn(&mut FontImportManifest);
        let breakers: Vec<(&str, Breaker)> = vec![
            ("empty family", |manifest| {
                manifest.family = String::new();
            }),
            ("family starting with punctuation", |manifest| {
                manifest.family = "-Nope".into();
            }),
            ("family with a slash", |manifest| {
                manifest.family = "A/B".into();
            }),
            ("family too long", |manifest| {
                manifest.family = "a".repeat(129);
            }),
            ("source off host", |manifest| {
                manifest.source = "https://example.com/font.ttf".into();
            }),
            ("source not https", |manifest| {
                manifest.source = "http://fonts.gstatic.com/font.ttf".into();
            }),
            ("no font path", |manifest| {
                manifest.font_path = String::new();
            }),
            ("bad font digest", |manifest| {
                manifest.font_sha256 = "nope".into();
            }),
            ("zero bytes", |manifest| manifest.font_bytes = 0),
            ("past the byte limit", |manifest| {
                manifest.font_bytes = IMPORTED_FONT_BYTE_LIMIT + 1;
            }),
            ("no licence path", |manifest| {
                manifest.licence_path = String::new();
            }),
            ("no licence digest", |manifest| {
                manifest.licence_sha256 = String::new();
            }),
            ("licence name off the enum", |manifest| {
                manifest.licence_name = "MIT".into();
            }),
        ];
        for (name, break_it) in breakers {
            let mut candidate = manifest();
            break_it(&mut candidate);
            assert!(!candidate.is_valid(), "accepted {name}");
        }
        // Exactly at the byte limit is legal.
        let mut at_limit = manifest();
        at_limit.font_bytes = IMPORTED_FONT_BYTE_LIMIT;
        assert!(at_limit.is_valid());
        // And the family pattern really does allow these.
        for family in [
            "Noto Sans",
            "Rock 3D",
            "M PLUS 1p",
            "Alegreya",
            "Zen Old Mincho",
        ] {
            let mut candidate = manifest();
            candidate.family = family.into();
            assert!(candidate.is_valid(), "rejected {family}");
        }
    }

    #[test]
    fn a_download_becomes_a_font_asset_only_with_both_stored_paths() {
        let manifest = manifest();
        let asset = manifest
            .to_font_asset("show.assets/fonts/face.ttf", "show.assets/fonts/OFL.txt")
            .expect("a validated download with both paths");
        assert!(asset.is_valid());
        assert!(asset.has_licence());
        assert_eq!(asset.family, "Space Grotesk");
        assert_eq!(asset.licence_name, "OFL-1.1");

        // Dropping the licence path would produce a face whose terms are unstated
        // even though the download had them. Refused.
        assert!(manifest
            .to_font_asset("show.assets/fonts/face.ttf", "")
            .is_none());
        // And an invalid manifest never becomes an asset at all.
        let mut broken = manifest.clone();
        broken.licence_name = "MIT".into();
        assert!(broken
            .to_font_asset("show.assets/fonts/face.ttf", "show.assets/fonts/OFL.txt")
            .is_none());
    }
}
