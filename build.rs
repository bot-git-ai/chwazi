// Copyright (c) 2026 Witalis Domitrz <witekdomitrz@gmail.com>
// AGPL License

//! Write the static app shell to `dist/` while the crate compiles.
//!
//! Chwazi is a browser application, so publishing it is a file write, not a
//! program run. Five of the eight shell files are committed as they are; the
//! other three are derived here:
//!
//! * `icon-192.png` and `icon-512.png` are rasterized from `assets/icon.svg`,
//!   which is the committed source of truth and is never replaced by a PNG.
//! * `service-worker.js` carries a `__VERSION__` placeholder standing for a
//!   cache name derived from the bytes of every other shell file, including
//!   the wasm. Deriving it here rather than per request is what makes the cache
//!   name change exactly when the app does.
//!
//! Everything goes to `dist/`, which is the whole site and the only copy of it.
//! `dist/` is gitignored, so the release gate -- which exports the candidate
//! tree -- never sees it.
//!
//! This script skips the wasm target entirely. During
//! `cargo build --lib --target wasm32-unknown-unknown` the `dist/` artefacts
//! are the *output* of that build: `app.js` and `app_bg.wasm` do not exist
//! yet, so publishing the site there would fail on the very step that produces
//! them. The host build that follows the wasm-bindgen pass is the one that
//! publishes.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

/// The files this script owns, and the committed file each is copied from.
///
/// `index.html` is `src/ui.html` byte for byte. `icon.svg` is the author's
/// original, copied unchanged: it is the icon the shell links and the one a
/// browser falls back to, and it is the same file `assets/icon.svg` is
/// rasterized from, so the site can never link a different drawing than the one
/// it shipped a PNG of.
///
/// The two wasm artefacts are deliberately absent: `wasm-bindgen` writes them
/// into `dist/` and they are build output that is never committed, so they have
/// no committed source to copy from -- and this script must not delete them,
/// because they are the app.
const SHELL: &[(&str, &str)] = &[
    ("index.html", "src/ui.html"),
    ("icon.svg", "assets/icon.svg"),
];

/// The icon sizes the manifest declares, and the only two PNGs the site has.
const ICON_SIZES: [u32; 2] = [192, 512];

/// The manifest, assembled here rather than committed.
///
/// Generating it from the same list that produced the PNGs is the point: its
/// icon list cannot drift from what was actually rasterized. The original
/// `manifest.json` named no colours at all, so the two below are new. Black,
/// because the app is black from edge to edge and a black splash is the only
/// colour that does not flash white between the launcher and the first frame.
const NAME: &str = "Chwazi Finger Chooser";
const SHORT_NAME: &str = "Chwazi";
/// `fullscreen`, as the original chose: this is a phone passed round a table,
/// and the point is that nothing but the fingers is on the glass.
const DISPLAY: &str = "fullscreen";
const BACKGROUND_COLOR: &str = "#000000";
const THEME_COLOR: &str = "#000000";

fn main() {
    let root = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());

    // The site is built only for the host target; see the module comment.
    let target = std::env::var("TARGET").unwrap_or_default();
    if target.starts_with("wasm") {
        return;
    }

    // Watch the SOURCE paths, not the output names: cargo compares these
    // against real files, so the shell and the icon have to be named as the
    // files they are. Watching the destination names watches files that never
    // change, and the script then never re-runs.
    for (_, source) in SHELL {
        println!("cargo:rerun-if-changed={source}");
    }
    println!("cargo:rerun-if-changed=src/service-worker.js");
    println!("cargo:rerun-if-changed=build.rs");

    // Every file this script owns, named up front. The list is what
    // `tests/shell.rs` reads to check that the service worker's `ASSETS` and the
    // published site agree, so the names of the two *derived* files have to be
    // written out rather than left to the calls that push them -- a name that
    // exists only inside a `format!` cannot be checked against anything.
    let derived = ["manifest.webmanifest", "service-worker.js"];

    let mut built = Vec::with_capacity(SHELL.len() + ICON_SIZES.len() + derived.len());
    for (name, source) in SHELL {
        let bytes = std::fs::read(root.join(source))
            .unwrap_or_else(|error| panic!("reading {source}: {error}"));
        built.push((*name, bytes));
    }
    built.extend(rasterize_icons(&root));
    built.push((
        derived[0],
        manifest().into_bytes(),
    ));

    // Last, because it hashes everything above: the worker's cache name has to
    // move when any of them does.
    built.push((derived[1], worker_bytes(&root, &built)));

    write_tree(&root.join("dist"), &built);
}

/// Rasterize `assets/icon.svg` at each install size.
///
/// `assets/icon.svg` is the author's original Material Symbols "touch_long",
/// kept byte for byte: the committed, authoritative, hand-editable icon. These
/// PNGs are build output derived from it, which is why neither exists in the
/// tree between builds.
///
/// Renders straight to each size rather than rasterizing at 512 and
/// downscaling: the source is a 48-unit viewBox scaled to 960, so a 512 render
/// and a 192 render differ only in output resolution and both are exact.
fn rasterize_icons(root: &Path) -> Vec<(&'static str, Vec<u8>)> {
    let svg = std::fs::read(root.join("assets/icon.svg"))
        .unwrap_or_else(|error| panic!("reading assets/icon.svg: {error}"));    let options = usvg::Options::default();
    let tree = usvg::Tree::from_data(&svg, &options)
        .unwrap_or_else(|error| panic!("parsing assets/icon.svg: {error}"));

    let mut icons = Vec::with_capacity(ICON_SIZES.len());
    for size in ICON_SIZES {
        let mut pixmap = tiny_skia::Pixmap::new(size, size)
            .unwrap_or_else(|| panic!("a {size}x{size} pixmap: out of memory"));
        // f32, not f64: `resvg` takes a `Transform`, whose fields are f32.
        let scale = size as f32 / tree.size().width() as f32;
        let transform = tiny_skia::Transform::from_scale(scale, scale);
        resvg::render(&tree, transform, &mut pixmap.as_mut());
        let png = pixmap
            .encode_png()
            .unwrap_or_else(|error| panic!("encoding icon-{size}.png: {error}"));
        // A vector that failed to parse can render as a blank square, and an
        // install icon that is blank is worse than a build failure: it is only
        // ever noticed on a home screen.
        assert!(
            !png.is_empty() && png.len() > 100,
            "icon-{size}.png rasterized to nothing; assets/icon.svg is empty or unrenderable"
        );
        icons.push((leak(format!("icon-{size}.png")), png));
    }
    icons
}

/// Hand a formatted name out with a `'static` lifetime.
///
/// The manifest and the icons are built once per build and only borrowed for
/// the length of `write_tree`, so this keeps `build` a flat list of pairs
/// without threading an owned-name type through three functions.
fn leak(name: String) -> &'static str {
    Box::leak(name.into_boxed_str())
}

/// The manifest, as JSON.
fn manifest() -> String {
    let icons: Vec<serde_json::Value> = ICON_SIZES
        .iter()
        .map(|size| {
            serde_json::json!({
                "src": format!("icon-{size}.png"),
                "sizes": format!("{size}x{size}"),
                "type": "image/png",
                "purpose": "any maskable",
            })
        })
        .collect();
    let manifest = serde_json::json!({
        "id": "./",
        "name": NAME,
        "short_name": SHORT_NAME,
        "start_url": "./",
        "scope": "./",
        "display": DISPLAY,
        "background_color": BACKGROUND_COLOR,
        "theme_color": THEME_COLOR,
        "icons": icons,
    });
    manifest.to_string()
}

/// The published service worker: the template with its cache version filled in.
///
/// The version covers every other file in `dist/`, so this is the last thing
/// written and the only one that depends on the rest.
fn worker_bytes(root: &Path, built: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let template = std::fs::read_to_string(root.join("src/service-worker.js"))
        .unwrap_or_else(|error| panic!("reading src/service-worker.js: {error}"));
    let worker = template.replace("__VERSION__", &cache_version(root, built, &template));
    assert!(
        !worker.contains("__VERSION__"),
        "the service worker still contains the version placeholder"
    );
    worker.into_bytes()
}

/// A cache name derived from the bytes of every other shell file.
///
/// That includes `app_bg.wasm`, which is why the host build has to run *after*
/// the bindings: hashing it is the only thing that makes a change to the Rust
/// invalidate the cache. Without that, an app rebuilt with different logic
/// would keep serving the previous wasm from cache and no amount of rebuilding
/// would reach an installed copy.
///
/// It also covers the worker's own source: a change to the caching logic must
/// invalidate the cache too, or clients keep running the old logic against new
/// assets.
///
/// A missing wasm is skipped rather than fatal. The host build is documented to
/// run second, but nothing forces that ordering on a developer running
/// `cargo build` alone, and failing there would mean the crate could not be
/// built without the wasm target installed -- which the release gate does.
fn cache_version(root: &Path, built: &[(&str, Vec<u8>)], worker_template: &str) -> String {
    let mut hasher = DefaultHasher::new();
    for (name, bytes) in built {
        name.hash(&mut hasher);
        bytes.hash(&mut hasher);
    }
    let wasm = root.join("dist/app_bg.wasm");
    if let Ok(bytes) = std::fs::read(&wasm) {
        "app_bg.wasm".hash(&mut hasher);
        bytes.hash(&mut hasher);
    }
    worker_template.hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

/// Write every file this script owns into `dir`, in place.
///
/// Not a staging tree renamed over `dist/`: the wasm artefacts are written
/// there by the `wasm-bindgen` step, which runs before this one, and a
/// wholesale swap deleted them -- leaving a publishable-looking `dist/` with no
/// app in it and no error. So only the files named above are written and the
/// rest of the directory is untouched.
///
/// Each is written under a scratch name and renamed over its target, so a host
/// serving the directory never observes a half-written file.
fn write_tree(dir: &Path, built: &[(&str, Vec<u8>)]) {
    std::fs::create_dir_all(dir)
        .unwrap_or_else(|error| panic!("{}: {error}", dir.display()));

    for (name, bytes) in built {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create shell directory");
        }
        let scratch = dir.join(format!(".{name}.new"));
        std::fs::write(&scratch, bytes)
            .unwrap_or_else(|error| panic!("writing {}: {error}", scratch.display()));
        std::fs::rename(&scratch, &path)
            .unwrap_or_else(|error| panic!("publishing {}: {error}", path.display()));
    }
}
