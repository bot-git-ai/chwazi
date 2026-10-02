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

/// Rust source with every comment removed, for substring assertions.
///
/// A test that asserts on source text is asserting on the wrong thing if it can be
/// satisfied by a comment. A doc comment naming a call satisfies a substring check
/// with the call deleted, which is the failure mode that matters here: the drawing
/// code is wasm-only and cannot be unit-tested on the host, so these tests are the
/// only automated check it has.
///
/// Deliberately simple: it strips `//` to end of line and `/* ... */`, and does not
/// try to understand string literals. A `//` inside a string would end the "comment"
/// early and leave some real code behind, which can only make an assertion stricter,
/// never looser.
fn strip_rust_comments(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut chars = source.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '/' if chars.peek() == Some(&'/') => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut prev = '\0';
                for c in chars.by_ref() {
                    if prev == '*' && c == '/' {
                        break;
                    }
                    prev = c;
                }
                out.push(' ');
            }
            _ => out.push(c),
        }
    }
    out
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
        .is_ok_and(|out| out.status.success());
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
    assert!(
        page.contains("<!DOCTYPE html>"),
        "the shell must be a document"
    );
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

/// The worker only ever answers for a URL inside its own directory.
///
/// This is the guard that stops one of these apps from taking over the pages it
/// shares an origin with. A service worker registered for a scope is consulted
/// for every URL under that scope, and these apps are all served from the same
/// origin as pages that are not apps at all — so "the scope is small" is a
/// promise, and this test is what keeps it one.
#[test]
fn the_worker_never_answers_outside_its_own_directory() {
    let worker = worker();
    assert!(
        worker.contains("new URL('./', self.location.href)"),
        "the worker must resolve its own directory from its location"
    );
    // The guard is a prefix test against that directory, on the request URL,
    // applied before the allowlist decides anything.
    assert!(
        worker.contains("IS_OWN(url)"),
        "the fetch handler must check the request is inside this app's directory; \
         without it a mis-scoped registration serves whatever it cached"
    );
    assert!(
        worker.contains("const IS_OWN = url => url.startsWith(ROOT.href)"),
        "the directory guard must be a prefix test against the worker's own root"
    );
}

/// The page states the worker's scope instead of inheriting it, and cleans up a
/// wider registration left behind by an earlier version.
///
/// A registration outlives the page that created it, and nothing short of an
/// explicit `unregister` takes one away. So the second half is what makes this
/// recoverable without the user clearing their browser: a stale registration is
/// not fixed by a reload, and the newer worker cannot take control of a scope it
/// does not own.
#[test]
fn the_page_states_the_scope_and_releases_a_wider_one() {
    let ui = std::fs::read_to_string(root().join("src/ui.rs")).expect("src/ui.rs");
    assert!(
        ui.contains("register_with_options"),
        "the worker must be registered with an explicit scope; left to default, \
         the scope is whatever directory the registering page sits in"
    );
    assert!(
        ui.contains("RegistrationOptions::new()") && ui.contains("set_scope(SCOPE)"),
        "the scope has to be actually stated, not merely a named constant"
    );
    assert!(
        ui.contains("get_registrations") && ui.contains("unregister"),
        "a stale wider registration survives a reload, a version bump and a \
         reinstall; only an explicit unregister clears it"
    );
}

/// A worker's script is compared by suffix, not by `trim_end_matches`.
///
/// `trim_end_matches` strips a *set of characters*, so a directory whose name
/// ends in those letters is silently treated as ours — and a registration
/// belonging to a sibling app would be torn down. This is a regression test for
/// a real bug in the first version of this code.
#[test]
fn the_script_comparison_strips_a_suffix_rather_than_a_character_set() {
    let ui = std::fs::read_to_string(root().join("src/ui.rs")).expect("src/ui.rs");
    // The prose in this file names the method to explain why it is not used, so
    // the assertion is about code: a call, not the word.
    let calls: Vec<&str> = ui
        .lines()
        .filter(|line| {
            let code = line.split("//").next().unwrap_or(line);
            code.contains("trim_end_matches(")
        })
        .collect();
    assert!(
        calls.is_empty(),
        "`trim_end_matches` strips a character set, not a filename: it would eat \
         any directory ending in those letters and tear down a sibling's worker. \
         Found: {calls:?}"
    );
    assert!(
        ui.contains("strip_suffix(\"service-worker.js\")"),
        "the comparison must strip the one filename it expects"
    );
}

/// The scope is named once, and the page and the worker agree on the directory.
///
/// Two independent resolutions of "where am I" — the page's `./` and the worker's
/// `new URL('./', self.location.href)`. They have to describe the same
/// directory, or the page registers a scope the worker's guard does not match.
#[test]
fn the_scope_is_a_relative_directory_shared_with_the_worker() {
    let ui = std::fs::read_to_string(root().join("src/ui.rs")).expect("src/ui.rs");
    assert!(
        ui.contains("const SCOPE: &str = \"./\";"),
        "the scope must be the app's own directory, relative — so one build works \
         from any subdirectory"
    );
    assert!(
        worker().contains("new URL('./', self.location.href)"),
        "the worker must resolve the same directory the page registered"
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
        .find(']')
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
        page.contains("could not start") && page.contains("Reload the page"),
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
                .map_or(text.len(), |end| start + end + close.len());
            text = format!("{}{}", &text[..start], &text[end..]);
        }
    }
    // Tags out. Entities are left alone: no word the rule bans can hide in one.
    while let Some(start) = text.find('<') {
        let end = text[start..]
            .find('>')
            .map_or(text.len(), |end| start + end + 1);
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
        .map(str::to_string)
        .collect()
}

/// The mark is four measured bands, and they are drawn in order.
///
/// Measured radially outward from a mark's centre in the native app, on a 1080px
/// Galaxy S25 at 3x: a pale dot to 7.7 CSS px, the saturated disc to 35.7, a black
/// gap to 44.3, and a pale ring to 54.3.
///
/// Two previous builds got this wrong in opposite directions, from the same frame:
/// one drew the disc and the ring edge to edge so they merged into a flat blob, and
/// the next took the gap and the ring for artefacts of a blurred crop and deleted
/// them. Both were confident and both were wrong, because every individual radius
/// is a plausible number -- only the order and the gaps between them are evidence.
///
/// So this checks the drawing code, not only the constants: a ring that comes back
/// as a draw call rather than as a constant would otherwise sail straight through.
#[test]
fn the_mark_is_three_bands_in_measured_order() {
    let ui = std::fs::read_to_string(root().join("src/ui.rs")).expect("src/ui.rs");

    // Scoped to the drawing function: `DOT_COLOUR` is referenced in more than one
    // place, so searching the whole file would compare offsets from the wrong one.
    let draw = ui
        .split("fn draw_player(")
        .nth(1)
        .expect("draw_player")
        .split("\n}\n")
        .next()
        .expect("the end of the function");

    // Three bands, and no dot.
    //
    // The dot is the important half: a small pale circle at the centre of every
    // mark looks entirely plausible -- it is the most distinctive thing in the
    // reference recordings, it sits at the exact centre, and a radial scan finds it
    // first. It is Android's "show touches" indicator, not the app's. It is the same
    // colour in every mark whatever that mark's own colour, and it does not move,
    // grow or pulse with the mark. On an indicator-off recording it is absent.
    //
    // Checked on the *stripped* source, and on the drawing function rather than the
    // whole file, for two reasons that are both about tests that cannot fail. A
    // literal radius reintroduces the dot without the name `DOT_RADIUS` anywhere, so
    // a name check alone lets it straight through -- which it did, the first time
    // this assertion was written. And a doc comment naming the call satisfies a
    // substring assertion with the call deleted.
    let code = strip_rust_comments(draw);
    assert!(
        !code.contains("DOT_RADIUS"),
        "the central dot is the phone's touch indicator, not the app's"
    );
    // A mark is the disc, the ring, and the two loading arcs -- four arcs and one
    // fill. Anything else drawn on top of the disc is a fifth shape that the
    // recordings do not contain.
    let fills = code.matches(".fill()").count();
    assert!(
        fills <= 1,
        "only the disc is filled; the dot was a second fill, and there are {fills}"
    );
    let disc = draw.find("DISC_RADIUS * disc_scale").expect("the disc");
    let ring = draw.find("RING_STROKE_RADIUS * scale").expect("the ring");
    // The ring's track is laid down before either loading, because a sweep drawn
    // onto nothing has nothing to reveal -- which is what made the first version of
    // the registration invisible.
    let track = draw.find("player.ring_color()").expect("the ring's track");
    let reg = draw
        .find("player.loading_color()")
        .expect("the registration sweep");
    let sel = draw
        .find("set_stroke_style_str(&colour)")
        .expect("the selection fill");
    assert!(
        disc < ring && ring < track && track < reg && reg < sel,
        "draw order must be disc ({disc}), ring ({ring}), ring track ({track}), \
         registration sweep ({reg}), selection fill ({sel})"
    );

    // The ring is stroked at the band's *centreline*, at its own width.
    //
    // Both halves of that matter and they are different mistakes. Stroking at the
    // disc's edge merges the ring into the disc, which is what the build before
    // last did. Stroking at the band's *outer* edge instead lays the band from
    // 49.8 to 58.8 CSS px -- outside the measured mark, and leaving 4.5px of the
    // gap showing as a second black band -- which is "the gap is too big", and it
    // is invisible in the constants because every number in it is correct.
    assert!(
        draw.contains("RING_STROKE_RADIUS * scale"),
        "the ring must be stroked at the middle of its band, not at its edge"
    );
    assert!(
        draw.contains("chooser::ARC_WIDTH * scale"),
        "and at the width of the band itself"
    );

    // The gap is the difference between the two bands, and it has to be visible.
    let chooser = std::fs::read_to_string(root().join("src/chooser.rs")).expect("chooser.rs");
    let constant = |name: &str| -> f64 {
        chooser
            .lines()
            .find_map(|line| line.strip_prefix(&format!("pub const {name}: f64 = ")))
            .and_then(|rest| rest.split(';').next())
            .and_then(|n| n.trim().parse().ok())
            .unwrap_or_else(|| panic!("{name}"))
    };
    // Measured on a frame recorded with the touch indicator OFF, which is the only
    // kind of frame these numbers are valid on.
    let (disc, gap, mark) = (
        constant("DISC_RADIUS"),
        constant("GAP_OUTER_RADIUS"),
        constant("MARK_RADIUS"),
    );
    for (name, got, want) in [
        ("the disc", disc, 38.2),
        ("the gap's outer edge", gap, 47.1),
        ("the mark's outer edge", mark, 57.0),
    ] {
        assert!(
            (got - want).abs() < 0.5,
            "{name} is {got} CSS px; measured {want}"
        );
    }
    assert!(gap - disc > 5.0, "the gap is wide enough to see");
    assert!(mark - gap > 5.0, "and so is the ring");

    // The ring is a *track* that always exists, and both loadings are drawn on top of
    // it.
    //
    // This is the fix for a loading animation that was completely invisible: the
    // ring was drawn in full and then an arc was painted over it, both in near-white
    // colours, so the arc had nothing to reveal. The track has to be there first.
    assert!(
        draw.contains("let ring = player.ring_color()"),
        "the ring's own tint must be drawn unconditionally, as a track for the \
         loadings to fill"
    );
    assert!(
        draw.contains("RING_STROKE_RADIUS * scale"),
        "and at the middle of its measured band"
    );

    // Both loadings are driven by their own progress, not by one shared number: the
    // per-finger one comes from the player's own registration and the draw's from
    // the window.
    assert!(
        draw.contains("if let Some(loading) = loading")
            && draw.contains("if let Some(progress) = draw"),
        "the two loadings must be distinguishable in the drawing code"
    );

    // The per-finger loading is passed in as a fraction, so the mark grows into
    // place rather than appearing at full size or snapping in.
    assert!(
        code.contains("if let Some(loading) = loading"),
        "the per-finger loading must reach the drawing code as a fraction"
    );

    // Neither loading may be drawn in a fixed pale colour.
    //
    // A constant colour is orange against every hue that is not orange, so the sweep
    // and the fill looked like they belonged to whichever player happened to be
    // orange rather than to the finger that had just landed. Both must ask the
    // player for their own colour.
    for fixed in ["DOT_COLOUR", "LOADING_COLOR"] {
        assert!(
            !code.contains(fixed),
            "{fixed} is a fixed pale colour; the loadings must use the player's own"
        );
    }
    assert!(
        code.contains("player.loading_color()") && code.contains("set_stroke_style_str(&colour)"),
        "the registration uses the player's lifted colour and the selection the \
         player's own, so each sweep belongs to the finger it is on"
    );
}
#[test]
fn the_mark_colours_are_measured() {
    // The colours, measured on an indicator-off frame.
    let chooser = std::fs::read_to_string(root().join("src/chooser.rs")).expect("chooser.rs");
    let constant = |name: &str| -> f64 {
        chooser
            .lines()
            .find_map(|line| line.strip_prefix(&format!("pub const {name}: f64 = ")))
            .and_then(|rest| rest.split(';').next())
            .and_then(|n| n.trim().parse().ok())
            .unwrap_or_else(|| panic!("{name}"))
    };
    let lightness = constant("COLOUR_LIGHTNESS");
    assert!(
        (lightness - 49.0).abs() < 1.0,
        "lightness is {lightness}%; the native median is 49%"
    );
    assert!(
        chooser.contains("DOT_COLOUR") && chooser.contains("RING_DARKEN"),
        "the loadings' colour and the ring's darkening must be the sampled ones"
    );
}

#[test]
fn both_loading_sweeps_are_drawn_on_their_own_band_outside_the_ring() {
    // The two loading sweeps are drawn on one band of their own, outside the ring's,
    // and the ring keeps its own colour underneath them for the whole animation.
    //
    // The previous build painted the pale sweep straight over the ring's band, so
    // the ring was the sweep's colour while the load ran and snapped back to its
    // resting tint the moment it ended. That flash is "the colour should stay
    // loaded, not flip back" -- the ring was being recoloured and then released,
    // rather than the sweep being a separate thing that arrives.
    //
    // Measured on the native app: the ring's band is one single colour throughout,
    // rgb(35, 113, 132) at the fixed 135-degree origin and everywhere else, at
    // every sampled instant of the load and long after it.
    let ui = std::fs::read_to_string(root().join("src/ui.rs")).expect("src/ui.rs");
    let draw = ui
        .split("fn draw_player(")
        .nth(1)
        .expect("draw_player")
        .split("\n}\n")
        .next()
        .expect("the end of the function");
    let code = strip_rust_comments(draw);

    // Both sweeps are on the same radius, and it is derived from the ring's radius
    // rather than being a second number that could drift away from it.
    let grown = code
        .matches("ring_radius + chooser::LOADING_ARC_GROWTH * scale")
        .count();
    assert_eq!(
        grown, 2,
        "the registration and the selection sweep share one band, outside the ring: \
         found {grown}"
    );
    assert!(
        !code.contains("LOADING_GROWTH"),
        "and there is no per-sweep offset constant: they are the same band"
    );

    // The ring is painted in its own tint and never in a sweep's, so nothing can
    // recolour it and then give the colour back.
    assert_eq!(
        code.matches("let ring = player.ring_color();").count(),
        1,
        "the ring is drawn once, in its own tint"
    );
    for sweep in ["player.loading_color()", "set_stroke_style_str(&colour)"] {
        let uses = code.matches(sweep).count();
        assert!(
            uses >= 1,
            "and the sweeps use their own colours, {sweep}, found {uses}"
        );
    }
}

/// The start screen is bare.
///
/// The native app's start screen carries three things this one has no business
/// having: a play counter ("You made 53 Chwazi's"), a prompt, and two menu icons in
/// corners. All three are out — a counter is state this app does not keep, a
/// prompt is in the way of the thing being looked at, and menu icons are a settings
/// screen this app does not have. The canvas is the whole interface.
#[test]
fn the_start_screen_is_bare() {
    let page = shell();
    for unwanted in ["You made", "Put at least", "Chwazi's", "1W", "counter"] {
        assert!(
            !page.contains(unwanted),
            "the native app's start screen has {unwanted:?}, and this one must not"
        );
    }
    // Nothing is drawn on the canvas before a finger lands either: the hint that
    // existed briefly is gone, and `tests/shell.rs` checks the rendered text.
    let ui = std::fs::read_to_string(root().join("src/ui.rs")).expect("src/ui.rs");
    for unwanted in ["You made", "Put at least", "draw_hint"] {
        assert!(
            !ui.contains(unwanted),
            "Rust must not draw or write {unwanted:?} either"
        );
    }
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
