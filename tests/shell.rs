// Copyright (c) 2026 Witalis Domitrz <witekdomitrz@gmail.com>
// AGPL License

//! Invariants of the app shell source, and of what is and is not committed.
//!
//! Everything here reads committed files. `dist/` is build output and is
//! gitignored, so the release gate — which exports the candidate tree — never
//! has it, and cannot build it either: that needs the wasm target and a pinned
//! `wasm-bindgen` CLI. A test asserting on `dist/` would therefore run only in a
//! developer's checkout, which is exactly where it is least likely to catch
//! anything, so those assertions are gone rather than skipped.
//!
//! What covers the built output is running the two build steps, in the order
//! AGENTS.md gives them. `.github/workflows/build.yml` does exactly that on
//! every push and then inspects what came out, so the assertions about the
//! *shape* of the site are made here against the sources that produce it.

use std::path::Path;

/// The repository root.
fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// The app shell, as committed.
///
/// `build.rs` copies this into `dist/index.html` byte for byte, so asserting
/// on it asserts on exactly what gets published.
fn shell() -> String {
    let path = root().join("src/ui.html");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("reading {}: {error}", path.display()))
}

/// The service worker template, as committed.
fn worker() -> String {
    std::fs::read_to_string(root().join("src/service-worker.js"))
        .expect("the committed worker template")
}

/// Tracked file names, or `None` outside a checkout.
///
/// The release gate exports the candidate as a bare directory with no `.git`,
/// so there is no index to ask. Callers decide what that means.
fn tracked_files() -> Option<String> {
    let inside = std::process::Command::new("git")
        .args(["rev-parse", "--git-dir"])
        .current_dir(root())
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false);
    if !inside {
        return None;
    }
    let output = std::process::Command::new("git")
        .args(["ls-files"])
        .current_dir(root())
        .output()
        .expect("git ls-files");
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// The shell is the app, and the app is wasm. A hand-written ABI would mean the
/// chooser no longer shared the library the unit tests cover.
#[test]
fn the_page_loads_generated_bindings_not_a_manual_wasm_abi() {
    let page = shell();
    assert!(page.contains("<!DOCTYPE html>"), "the shell must be a document");
    assert!(page.contains("<script type=\"module\">"), "a module script");
    assert!(
        page.contains("import('./app.js')"),
        "the page must load the generated bindings"
    );
    assert_eq!(
        page.matches("<script").count(),
        1,
        "exactly one script tag:\n{page}"
    );
    for obsolete in [
        "instantiateStreaming",
        "wasm.exports",
        "addEventListener",
        "requestAnimationFrame",
        "getElementById('main').getContext",
    ] {
        assert!(
            !page.contains(obsolete),
            "{obsolete} in the static shell: the page must not do what Rust does"
        );
    }
    // The original's viewport is load-bearing: a second finger must never reach
    // the browser's pinch-zoom, because that finger is choosing a winner.
    assert!(
        page.contains("user-scalable=no"),
        "the page must keep the original viewport"
    );
    // And the touch rule needs the CSS as well as the listener, because a
    // browser that honours `touch-action` never fires the scroll it would
    // otherwise have to cancel.
    assert!(
        page.contains("touch-action: none"),
        "touch-action must keep the page from scrolling under a finger"
    );
}

/// The canvas is the entire interface, so it has to exist, be labelled, and the
/// draw's state has to reach a screen reader some other way.
#[test]
fn the_page_is_one_labelled_canvas_with_a_live_status() {
    let page = shell();
    assert!(page.contains("id=\"main\""), "the canvas must be #main");
    assert!(page.contains("<canvas"), "the app draws into a canvas");
    assert!(
        page.contains("aria-label"),
        "a canvas with no label is invisible to a screen reader"
    );
    assert!(
        page.contains("id=\"status\""),
        "the draw's state must be announced somewhere"
    );
    assert!(
        page.contains("aria-live") || page.contains("role=\"status\""),
        "that announcement must be live, not just present"
    );
    // The failure UI exists in the shell and is hidden, so a wasm that will not
    // load says so instead of showing a black rectangle.
    assert!(page.contains("id=\"error\""), "a loader failure message");
    assert!(
        page.contains("hidden"),
        "which starts hidden, so it never costs the app a pixel"
    );
    assert!(
        page.contains("<noscript>"),
        "and there is a message for a browser with no JavaScript at all"
    );
}

/// The site is mounted under an arbitrary prefix, so every URL in it is
/// relative. One build, any subdirectory.
#[test]
fn the_shell_is_mountable_anywhere() {
    let page = shell();
    assert!(
        !page.contains("http://") && !page.contains("https://"),
        "an absolute URL would break the site outside its own origin"
    );
    assert!(
        page.contains("./app.js"),
        "bindings must be referenced relatively"
    );
    assert!(page.contains("manifest.webmanifest"), "a manifest");
    assert!(
        page.contains("./icon.svg"),
        "the committed SVG is the icon the shell links"
    );
    assert!(
        !page.contains("user-select: none") || page.contains("touch-action"),
        "a finger drag must not select text or scroll"
    );
}

/// The service worker is a committed template with exactly one placeholder, and
/// `build.rs` substitutes it. A template with no placeholder would mean the
/// cache never invalidates; a second one would mean the substitution is not the
/// only edit.
#[test]
fn the_service_worker_template_has_exactly_one_placeholder() {
    let worker = worker();
    assert_eq!(
        worker.matches("__VERSION__").count(),
        1,
        "the template must carry exactly one version placeholder"
    );
    assert!(
        worker.contains("new URL('./', self.location.href)"),
        "the worker must resolve its cache from its own location"
    );
    // A `skipWaiting()` call would swap the wasm under a live tab, which for an
    // app whose entire state is one wasm module means half old code drawing over
    // half new code. Checked as a *call*, not as the bare word, so this file is
    // still allowed to explain why it does not make one.
    assert!(
        !worker.contains("skipWaiting("),
        "an update must wait for the old tab to close"
    );
    assert!(
        worker.contains("clients.claim()"),
        "and then take over the pages it now covers"
    );
}

/// Everything the worker precaches has to exist, or `caches.addAll` rejects the
/// *whole* install on one 404 and the app silently loses offline support. The
/// symptom is "offline is broken", with nothing in the build output, no failed
/// request in the obvious place, and a favicon 404 that looks cosmetic and is
/// not. Several apps in this family have shipped a `dist/` missing one file.
///
/// The list is read out of the worker's own `ASSETS` array rather than written
/// out here, because a second copy of the list is exactly how the two drift
/// apart: the build drops a file, the test still asserts the old eight, and
/// nothing compares them. Deriving the expectation from the template asks the
/// only question that matters — does the build publish everything the worker
/// asks for?
#[test]
fn every_precached_file_is_published_by_the_build() {
    let assets = worker_assets(&worker());
    assert!(
        !assets.is_empty(),
        "the ASSETS list could not be read out of the worker"
    );

    let published = published_files();
    for asset in &assets {
        // `'./'` is the scope root, which is `index.html` on disk.
        let name = if asset == "./" { "index.html" } else { asset };
        assert!(
            published.iter().any(|file| file == name),
            "the service worker precaches {asset:?}, which the build does not publish; \
             caches.addAll rejects the whole install on one 404 (published: {published:?})"
        );
    }

    // The other direction: a published file the worker does not cache is only
    // fetched from the network, so it is not an error — but the shell's own
    // files all should be cached, so a name dropped from the worker is caught
    // here rather than being a silent no-op.
    for file in ["app.js", "app_bg.wasm", "manifest.webmanifest"] {
        assert!(
            assets.iter().any(|asset| asset == file),
            "{file} is published but not precached"
        );
    }
}

/// The `ASSETS` entries from a service worker template, in order.
///
/// Reads the array as written rather than evaluating JavaScript: the entries are
/// string literals in a fixed list, and the test needs to know the list even in
/// a template that would not parse.
fn worker_assets(worker: &str) -> Vec<String> {
    let start = worker
        .find("const ASSETS = [")
        .expect("the worker must declare ASSETS");
    let body_start = start + "const ASSETS = [".len();
    let end = worker[body_start..]
        .find("]")
        .expect("the ASSETS list must be terminated");
    worker[body_start..body_start + end]
        .split(',')
        .filter_map(|entry| {
            let entry = entry.trim();
            let inner = entry.strip_prefix('\'')?.strip_suffix('\'')?;
            Some(inner.to_string())
        })
        .collect()
}

/// The names of the files a complete `dist/` holds, gathered from the build
/// script rather than written out here.
///
/// Two sources, because there are two builds: `build.rs` names the files it
/// copies and derives, and the bindings and the wasm are the two files the
/// `wasm-bindgen` step writes with `--out-name app`. Both are read from the
/// committed sources, so this needs no `dist/` — which is gitignored, and so
/// absent from the tree the release gate exports.
fn published_files() -> Vec<String> {
    let build = std::fs::read_to_string(root().join("build.rs")).expect("build.rs");
    let mut files = Vec::new();

    // `("index.html", "src/ui.html")` — the copied shell files.
    for line in build.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("(\"") else {
            continue;
        };
        let Some((name, _)) = rest.split_once("\",") else {
            continue;
        };
        files.push(name.to_string());
    }

    // The derived names: the two icons at the sizes ICON_SIZES lists, plus the
    // manifest and the worker.
    if let Some(sizes) = build
        .lines()
        .find(|line| line.trim_start().starts_with("const ICON_SIZES"))
    {
        for size in sizes
            .trim_start_matches("const ICON_SIZES: [u32; 2] = [")
            .trim_end_matches("];")
            .split(',')
        {
            let size = size.trim();
            if !size.is_empty() {
                files.push(format!("icon-{size}.png"));
            }
        }
    }
    // The derived names are in a plain array rather than buried in the calls
    // that push them, so a file the build stopped publishing is a line missing
    // from a list anyone can read.
    if let Some(start) = build.find("let derived = [") {
        let body_start = start + "let derived = [".len();
        let end = build[body_start..]
            .find(']')
            .expect("the derived list must be terminated");
        for name in build[body_start..body_start + end].split(',') {
            let name = name.trim().trim_matches('"');
            if !name.is_empty() {
                files.push(name.to_string());
            }
        }
    }

    // The two files the `wasm-bindgen` step writes, from `--out-name app`.
    files.push("app.js".to_string());
    files.push("app_bg.wasm".to_string());
    files
}

/// Nothing the player can read may name how the app is built.
///
/// A player who cannot start the app needs something they can act on. "The Rust
/// application did not load" tells them nothing: they did not choose the
/// language, cannot change it, and will not know what to do with the sentence.
/// The interface says what failed and what to try; the implementation is an
/// explanation for whoever maintains this, and belongs in `AGENTS.md`.
///
/// Scoped to what the DOM actually renders — the tags that carry text — because
/// `src/ui.html` is full of comments that are *about* Rust and must stay that
/// way.
#[test]
fn no_user_visible_text_names_the_implementation() {
    let page = shell();
    let text = rendered_text(&page);

    for banned in [
        "rust",
        "webassembly",
        "wasm",
        "javascript",
        "bindings",
        "compiled",
        "compile",
    ] {
        assert!(
            !text.to_lowercase().contains(banned),
            "the rendered page says {banned:?}, which a player cannot act on: {text:?}"
        );
    }

    // And the same for the half of the UI that Rust writes into the DOM, which
    // this test can only reach as source.
    let ui = std::fs::read_to_string(root().join("src/ui.rs")).expect("src/ui.rs");
    for string in string_literals(&ui) {
        let shows_in_the_ui = string.contains("could not start")
            || string.contains("finger")
            || string.contains("wins")
            || string.contains("Chwazi");
        if !shows_in_the_ui {
            continue;
        }
        for banned in ["rust", "webassembly", "wasm", "javascript", "bindings"] {
            assert!(
                !string.to_lowercase().contains(banned),
                "src/ui.rs writes {string:?} into the DOM, and it says {banned:?}"
            );
        }
    }

    // A loader failure the player can do something about. Checked rather than
    // assumed, because this string is the only thing standing between a failed
    // load and a blank black screen.
    assert!(
        page.contains("could not start")
            && page.contains("Reload the page"),
        "the failure UI must say what failed and what to try"
    );
    assert!(
        !page.contains("The Rust application"),
        "the old implementation-naming failure message must not come back"
    );
}

/// The text the page renders: HTML comments, CSS and scripts removed.
///
/// A tag-aware pass is overkill for a shell with no attributes containing prose.
/// What it must not do is match comments or the stylesheet, which legitimately
/// talk about Rust and about `touch-action` by name.
fn rendered_text(page: &str) -> String {
    let mut text = page.to_string();
    for (open, close) in [
        ("<style", "</style>"),
        ("<!--", "-->"),
        ("<script", "</script>"),
    ] {
        while let Some(start) = text.find(open) {
            let end = text[start..]
                .find(close)
                .map(|end| start + end + close.len())
                .unwrap_or(text.len());
            text = format!("{}{}", &text[..start], &text[end..]);
        }
    }
    // Tags out. Entities are left alone: no word the rule bans can hide in one.
    while let Some(start) = text.find('<') {
        let end = text[start..]
            .find('>')
            .map(|end| start + end + 1)
            .unwrap_or(text.len());
        text = format!("{}{}", &text[..start], &text[end..]);
    }
    text.replace("&nbsp;", " ")
}

/// The contents of every string literal in a source file.
///
/// Deliberately simple: it splits on quotes rather than parsing Rust, because
/// the point is to have every candidate in hand cheaply, and the filter above
/// discards the overwhelming majority of them.
fn string_literals(source: &str) -> Vec<String> {
    source
        .split('"')
        .skip(1)
        .step_by(2)
        .map(|literal| literal.to_string())
        .collect()
}

/// The manifest is assembled in `build.rs` from constants, so this asserts on
/// those — the only copy a test can reach, since the built one is gitignored.
/// What it pins is what the original `manifest.json` chose: the name, the short
/// name, and `fullscreen`, which for a phone passed round a table is the whole
/// point — nothing but the fingers on the glass.
#[test]
fn the_manifest_keeps_the_originals_name_and_fullscreen_display() {
    let build = std::fs::read_to_string(root().join("build.rs")).expect("build.rs");
    for (constant, expected) in [
        ("const NAME: &str = ", "\"Chwazi Finger Chooser\""),
        ("const SHORT_NAME: &str = ", "\"Chwazi\""),
        ("const DISPLAY: &str = ", "\"fullscreen\""),
    ] {
        let line = build
            .lines()
            .find(|line| line.trim_start().starts_with(constant))
            .unwrap_or_else(|| panic!("build.rs must declare {constant}"));
        assert!(
            line.contains(expected),
            "{constant} must be {expected}, to keep the original manifest: {line}"
        );
    }
    for key in ["\"id\"", "\"start_url\"", "\"scope\""] {
        assert!(
            build.contains(&format!("{key}: \"./\"")),
            "{key} must be \"./\" so the site mounts anywhere"
        );
    }
    assert!(
        build.contains("\"purpose\": \"any maskable\""),
        "the install icons must be declared maskable"
    );
}

/// Nothing generated may be tracked — not the wasm, not the bindings, not the
/// site, and not the rasterized icons. This is the test that would have caught
/// them being committed.
#[test]
fn no_build_artifact_is_committed() {
    let Some(tracked) = tracked_files() else {
        return; // not a checkout: the gate's exported tree
    };
    for artefact in [
        "assets/icon-192.png",
        "assets/icon-512.png",
        "dist/index.html",
        "dist/app.js",
        "dist/app_bg.wasm",
        "dist/icon-192.png",
        "dist/icon-512.png",
    ] {
        assert!(
            !tracked.lines().any(|line| line == artefact),
            "{artefact} is tracked; generated artefacts must never be committed"
        );
    }
    // Nor may the PNGs exist in the source tree at all: `assets/icon.svg` is the
    // icon, and a PNG beside it would be a second, competing source of truth.
    assert!(
        !root().join("assets/icon-192.png").exists(),
        "the install PNGs are build output; only assets/icon.svg is committed"
    );
    assert!(
        !root().join("assets/icon-512.png").exists(),
        "the install PNGs are build output; only assets/icon.svg is committed"
    );
}

/// `dist/` has to be ignored, or a build would leave the next commit dirty.
#[test]
fn dist_is_ignored() {
    if tracked_files().is_none() {
        return; // not a checkout
    }
    let ignored = std::process::Command::new("git")
        .args(["check-ignore", "-q", "dist/"])
        .current_dir(root())
        .status()
        .expect("git check-ignore")
        .success();
    assert!(ignored, "dist/ must be in .gitignore");
}

/// The committed icon is the author's original, and this pins it. A rewrite that
/// redrew it would be a redesign, and the PNGs derived from a redrawn SVG would
/// carry the redrawing into every install icon silently — the difference is only
/// ever visible on a home screen.
#[test]
fn the_committed_icon_is_the_authors_original() {
    let icon = std::fs::read(root().join("assets/icon.svg")).expect("assets/icon.svg");
    assert_eq!(
        icon.len(),
        1144,
        "assets/icon.svg must be the original, byte for byte"
    );
    let icon = String::from_utf8(icon).expect("the icon is text");
    assert!(
        icon.contains("touch_long"),
        "the original icon is Material Symbols \"touch_long\""
    );
    assert!(
        icon.contains("fill: #434343"),
        "and it is the original's #434343, on transparent"
    );
    // It is published, not just committed: the shell links it and the worker
    // precaches it, so `build.rs` has to copy it.
    assert!(
        std::fs::read_to_string(root().join("build.rs"))
            .expect("build.rs")
            .contains("(\"icon.svg\", \"assets/icon.svg\")"),
        "build.rs must publish icon.svg, or the favicon 404s and the whole worker \
         install fails"
    );
}

/// The original JavaScript PWA is gone, not merely unused.
///
/// A repository that still carries `app.js`, `sw.js` and `manifest.json` beside
/// the Rust rewrite ships two apps: the one in `dist/`, and a dead one that a
/// reader can still find, run, and believe is the app. They are deleted in the
/// commit that introduces the crate, and this says so.
#[test]
fn the_original_javascript_pwa_is_gone() {
    for gone in ["app.js", "sw.js", "manifest.json", "index.html"] {
        assert!(
            !root().join(gone).exists(),
            "{gone} is the original hand-written PWA; it must be deleted, not left \
             beside the rewrite"
        );
    }
    if let Some(tracked) = tracked_files() {
        for gone in ["app.js", "sw.js", "manifest.json", "index.html"] {
            assert!(
                !tracked.lines().any(|line| line == gone),
                "{gone} is still tracked"
            );
        }
    }
    // What replaced them, so the assertions above cannot pass on an empty tree.
    for present in [
        "src/ui.html",
        "src/service-worker.js",
        "assets/icon.svg",
        "build.rs",
    ] {
        assert!(
            root().join(present).exists(),
            "{present} must exist: it is what replaced the original PWA"
        );
    }
}

/// The crate is a library plus a wasm entry point, and nothing else. A `[[bin]]`
/// would be a CLI this app has no use for, and a second one could only be
/// maintained next to a server that no longer exists.
#[test]
fn the_crate_has_no_binary() {
    let manifest = std::fs::read_to_string(root().join("Cargo.toml")).expect("Cargo.toml");
    assert!(
        !manifest.contains("[[bin]]"),
        "Chwazi is browser-only; a binary target is a CLI with nothing to do"
    );
    assert!(
        manifest.contains("crate-type = [\"cdylib\", \"rlib\"]"),
        "the crate is a wasm module and a testable library"
    );
    assert!(
        manifest.contains("wasm-bindgen = \"=0.2.128\""),
        "the generator pin is exact: a mismatched one emits bindings the runtime \
         will not load, and the page fails in a way that looks like an app bug"
    );
    assert!(
        manifest.contains("features = [\"wasm_js\"]"),
        "getrandom's wasm feature is `wasm_js`, not `js`"
    );
    assert!(
        !manifest.contains("[target.'cfg(not(target_arch = \"wasm32\"))'"),
        "there is no native-only code path to compile on the host"
    );
}

/// The chooser's rules live in a file that does not touch `web-sys`, which is
/// what makes them testable without a browser.
#[test]
fn the_state_machine_is_separate_from_the_dom() {
    let source = std::fs::read_to_string(root().join("src/chooser.rs")).expect("src/chooser.rs");
    for forbidden in ["web_sys", "web-sys", "wasm_bindgen", "getrandom"] {
        assert!(
            !source.contains(forbidden),
            "src/chooser.rs is the testable core and must not depend on {forbidden}"
        );
    }
    let ui = std::fs::read_to_string(root().join("src/ui.rs")).expect("src/ui.rs");
    assert!(
        ui.contains("web_sys"),
        "and the DOM layer is where web-sys belongs"
    );
}
