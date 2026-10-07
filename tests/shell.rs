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
//! AGENTS.md gives them: `.github/workflows/build.yml` does exactly that on every
//! push and then inspects what came out.

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
/// A test that asserts on source text is asserting on the wrong thing if a comment
/// can satisfy it: a doc comment naming a call passes a substring check with the
/// call deleted. That matters here because the drawing code is wasm-only and cannot
/// be unit-tested on the host, so these tests are the only automated check it has.
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
    // And the touch rule needs the CSS as well as the listener: a browser that
    // honours `touch-action` never fires the scroll it would otherwise cancel.
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
    // A prefix test against that directory, on the request URL, applied before the
    // allowlist decides anything.
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
    // `derived` is a plain array rather than buried in the calls that push them,
    // so a file the build stopped publishing is a line missing from a list anyone
    // can read.
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

    // And the same for the half of the UI that Rust writes into the DOM, which this
    // test can only reach as source.
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

    // Scoped to `draw_player`: the constants below appear in more than one place in
    // this file, so searching the whole file would compare offsets from the wrong
    // one.
    let draw = ui
        .split("fn draw_player(")
        .nth(1)
        .expect("draw_player")
        .split("\n}\n")
        .next()
        .expect("the end of the function");

    // Three bands, and no dot.
    //
    // The dot is the important half. A small pale circle at the centre of every
    // mark looks entirely plausible -- it is the most distinctive thing in the
    // reference recordings, it sits at the exact centre, and a radial scan finds it
    // first. It is Android's "show touches" indicator, not the app's: the same
    // colour in every mark whatever that mark's own colour, and it never moves,
    // grows or pulses with the mark. On an indicator-off recording it is absent.
    //
    // Checked on the *stripped* source, because a literal radius reintroduces the
    // dot without the name `DOT_RADIUS` anywhere -- which is how it first came
    // back, with a name check alone letting it through.
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
    // Draw order: the disc first, then one radius on which the ring is drawn in
    // whichever of its three states is current.
    let disc = draw
        .find("DISC_RADIUS * scale * arrived")
        .expect("the disc");
    let ring = draw
        .find("RING_STROKE_RADIUS * scale")
        .expect("the ring radius");
    let sweep = draw.find("let draw_ring =").expect("the ring, drawn");
    let finished = draw
        .find("player.ring_color()")
        .expect("the resting ring's tint");
    assert!(
        disc < ring && ring < sweep && sweep < finished,
        "draw order must be disc ({disc}), ring radius ({ring}), the ring ({sweep}), \
         its colours ({finished})"
    );

    // The ring is stroked at the band's *centreline*, at its own width.
    //
    // Both halves of that matter and they are different mistakes. Stroking at the
    // disc's edge merges the ring into the disc. Stroking at the band's *outer* edge
    // instead lays the band from 49.8 to 58.8 CSS px -- outside the measured mark,
    // and leaving 4.5px of the gap showing as a second black band -- which is "the
    // gap is too big", and it is invisible in the constants because every number in
    // it is correct.
    assert!(
        draw.contains("RING_STROKE_RADIUS * scale"),
        "the ring must be stroked at the middle of its band, not at its edge"
    );
    assert!(
        draw.contains("chooser::ARC_WIDTH * scale"),
        "and at the width of the band itself"
    );

    let chooser = std::fs::read_to_string(root().join("src/chooser.rs")).expect("chooser.rs");
    let constant = |name: &str| -> f64 {
        chooser
            .lines()
            .find_map(|line| line.strip_prefix(&format!("pub const {name}: f64 = ")))
            .and_then(|rest| rest.split(';').next())
            .and_then(|n| n.trim().parse().ok())
            .unwrap_or_else(|| panic!("{name}"))
    };
    // Measured on an indicator-off frame, the only kind these numbers are valid on.
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

    // The per-finger loading is passed in as a fraction, so the mark grows into
    // place rather than appearing at full size or snapping in.
    assert!(
        code.contains("Player::disc_arrival"),
        "the disc must be sized by the finger's own registration, not a constant"
    );

    // Neither loading may be drawn in a fixed pale colour.
    //
    // A constant colour is orange against every hue that is not orange, so the
    // sweep and the fill would belong to whichever player happened to be orange.
    for fixed in ["DOT_COLOUR", "LOADING_COLOR"] {
        assert!(
            !code.contains(fixed),
            "{fixed} is a fixed pale colour; the loadings must use the player's own"
        );
    }
    // Both loadings ask the player for its own colours: the registration draws the
    // ring slightly brighter than it will rest, and the draw window covers that ring
    // in the player's colour at full strength.
    assert!(
        code.contains("player.loading_color()") && code.contains("player.ring_color()"),
        "both loadings must take their colours from the player"
    );
    assert!(
        !code.contains("DOT_COLOUR"),
        "and neither may use the app's fixed pale colour"
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
fn the_two_loadings_are_different_gestures() {
    // A finger lands: the ring arrives, slightly brighter than it will rest, and
    // settles in place. That is the first loading.
    //
    // The draw window: the ring the first loading left behind is *covered* in the
    // player's own colour. It is not a second sweep that makes the loaded ring
    // disappear and then loads it again from nothing -- the difference between the
    // two stages is a colour filling the band, not a length of arc growing.
    let ui = std::fs::read_to_string(root().join("src/ui.rs")).expect("src/ui.rs");
    let draw = ui
        .split("fn draw_player(")
        .nth(1)
        .expect("draw_player")
        .split("\n}\n")
        .next()
        .expect("the end of the function");
    let code = strip_rust_comments(draw);
    let flat: String = code.split_whitespace().collect();

    assert!(
        flat.contains("ifletSome(t)=loading{"),
        "the first loading must be a distinct branch"
    );
    assert!(
        flat.contains("draw_ring(0.0,TWO_PI,&ring);")
            && flat.contains("chooser::sweep_arcs(draw_origin,t)"),
        "the second loading must draw the resting ring first and then cover it"
    );
    // The resting ring is laid down before the fill that covers it.
    let under = flat
        .find("draw_ring(0.0,TWO_PI,&ring);")
        .expect("the resting ring");
    let over = flat.find("sweep_arcs(draw_origin,t)").expect("the fill");
    assert!(
        under < over,
        "the ring must be drawn before the colour covers it"
    );
    assert!(
        flat.contains("&loaded") && flat.contains("&ring") && flat.contains("&colour"),
        "three distinct colours: the loading tint, the resting ring, and the fill"
    );

    // Both loadings sweep from their own origin, in two halves, meeting opposite.
    //
    // A single sweep from a fixed 135 degrees is what this replaced: it reads as a
    // dial, and every mark on a table fills in lockstep from the same point, so the
    // marks stop reading as belonging to particular fingers.
    assert!(
        flat.contains("chooser::sweep_arcs(load_origin,t)")
            && flat.contains("chooser::sweep_arcs(draw_origin,t)"),
        "both loadings must sweep from their own origin, in two halves"
    );
    assert!(
        !code.contains("LOADING_ARC_START"),
        "there is no fixed loading origin any more"
    );
    // The two arcs are stroked by looping over both, not by drawing one and
    // mirroring: a mirror is the same shape twice and cannot be told apart if a
    // half is ever a different length.
    assert_eq!(
        flat.matches("for(from,to)inchooser::sweep_arcs(").count(),
        2,
        "each loading strokes both of its halves"
    );
    // And each side is read from that player's own random origin rather than a
    // literal. A fixed literal here would make every mark on a table fill in
    // lockstep, which is what this change exists to stop -- and a test that only
    // checks the call site cannot see it, because the call site looks the same
    // either way. Both mutations below are caught by this and by nothing else.
    assert!(
        code.contains("let load_origin = player.load_origin;"),
        "the registration must sweep from the player's own origin"
    );
    assert!(
        !code.contains("2.35619449") && !code.contains("135.0"),
        "and no fixed angle may stand in for it"
    );
    assert!(
        code.contains("let draw_origin = app_draw_origin;"),
        "the selection must sweep from the draw's own origin"
    );

    // The winner's ring keeps the colour the draw covered it in.
    assert!(
        code.contains("app_is_chosen: bool"),
        "the drawing needs to know whether this player was chosen, or it cannot \
         tell a winner from a player who is merely waiting"
    );
    assert!(
        flat.contains("ifapp_is_chosen{draw_ring(0.0,TWO_PI,&colour);"),
        "a chosen player's ring must be drawn in the full colour, not the resting \
         tint -- the draw covers it in and it keeps that"
    );
    let last_stroke = code.rsplit("draw_ring(").next().expect("the last stroke");
    assert!(
        last_stroke.contains("&ring)"),
        "a player who has not won still rests at the dim tint, as the last stroke: \
         {last_stroke}"
    );
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

/// The committed icon is the Chwazi brand mark, and this pins its essentials. A
/// regression to some other drawing would be a redesign nobody asked for, and
/// the PNGs derived from the SVG would carry it into every install icon
/// silently — the difference is only ever visible on a home screen.
#[test]
fn the_committed_icon_is_the_chwazi_brand_mark() {
    let icon = std::fs::read_to_string(root().join("assets/icon.svg")).expect("assets/icon.svg");
    // The mark: a black tile with a 2x2 grid of four flat dots, the chosen one
    // yellow and the other three blue, in the user-chosen colours.
    assert!(
        icon.contains("viewBox=\"0 0 512 512\""),
        "the icon is a 512-unit tile"
    );
    assert!(
        icon.contains("fill=\"#000000\""),
        "the tile is black, edge to edge, like the app"
    );
    assert_eq!(
        icon.matches("fill=\"#ffea00\"").count(),
        1,
        "exactly one yellow dot: the chosen one"
    );
    assert_eq!(
        icon.matches("fill=\"#00a2ff\"").count(),
        3,
        "three azure blue dots: the not-chosen ones"
    );
    assert_eq!(icon.matches("<circle").count(), 4, "four flat dots, nothing else");
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

/// A workflow file, as committed.
///
/// Nonexistent is not a reason to fail. The Pages workflow is not the app, and
/// a release-branch export or a partial tree that has only `build.yml` says
/// nothing about the chooser. Callers decide which they want.
fn workflow(name: &str) -> Option<String> {
    let path = root().join(".github/workflows").join(name);
    std::fs::read_to_string(&path).ok()
}

/// The workflow with its comments removed.
///
/// A `#` inside a quoted string is not a comment, and a `#` in a value is not
/// one either -- but this workflow quotes nothing in the keys it is read for
/// and carries no `#` in any value it depends on, so a line-wise cut at the
/// first `#` is enough and a YAML parser is not worth a dependency.
///
/// This is not a nicety. The exact mutation that produced the bug these tests
/// exist to prevent was *commenting a line out* rather than deleting it, and an
/// assertion over the raw text would have passed on the broken file.
fn strip_yaml_comments(source: &str) -> String {
    source
        .lines()
        .map(|line| match line.find('#') {
            Some(index) => &line[..index],
            None => line,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The Pages deployment cannot drift from the crate it publishes.
///
/// The deploy job installs its own `wasm-bindgen`, pinned to a literal in the
/// YAML, and compares its digest against a second literal in the same file.
/// `Cargo.toml` pins the same version the crate compiles against. Move the
/// dependency and the workflow keeps building happily: it generates bindings
/// for a runtime the page does not have, and the only symptom is a live site
/// that fails at startup with "Chwazi could not start" -- for every visitor,
/// and only in the browser.
#[test]
fn the_pages_build_generates_bindings_for_the_pinned_runtime() {
    let Some(pages) = workflow("pages.yml") else {
        return;
    };
    let live = strip_yaml_comments(&pages);
    let manifest = std::fs::read_to_string(root().join("Cargo.toml")).expect("Cargo.toml");

    // Read the pin as Cargo writes it: `wasm-bindgen = "=0.2.128"`, an exact
    // requirement. A looser form ("0.2.128", "^0.2") would resolve to whatever
    // is newest in the lockfile, and the workflow's literal would then be
    // naming one arbitrary version of several.
    let pin = manifest
        .lines()
        .find_map(|line| {
            let rest = line.trim().strip_prefix("wasm-bindgen")?;
            let rest = rest.trim_start().strip_prefix('=')?;
            Some(rest.trim().trim_matches('"').to_owned())
        })
        .unwrap_or_else(|| panic!("Cargo.toml pins no exact wasm-bindgen version"));
    assert!(
        pin.starts_with('='),
        "Cargo.toml must pin wasm-bindgen exactly ({pin:?})",
    );
    let version = pin.trim_start_matches('=').trim_matches('"');

    // The `env:` block must exist and must declare the version at that same
    // literal. `version="$WASM_BINDGEN_VERSION"` with the variable undeclared
    // expands to nothing, the generator step installs nothing, and every other
    // check in the job still passes -- so this is asserted separately rather
    // than inferred from the install step mentioning the variable.
    assert!(
        live.contains("\nenv:"),
        "pages.yml must have a top-level `env:` block; the generator version is declared there",
    );
    let declared = format!("WASM_BINDGEN_VERSION: {version}");
    assert!(
        live.contains(&declared),
        "pages.yml must declare `{declared}`, matching the Cargo.toml pin; an undeclared \
         variable expands to nothing and the step installs no generator at all",
    );

    // And the version must reach the job that runs the generator, not merely
    // sit in `env:` where nothing reads it.
    assert!(
        live.contains("version=\"$WASM_BINDGEN_VERSION\""),
        "the install step must use the declared variable (`version=\"$WASM_BINDGEN_VERSION\"`) \
         or the literal; a declared-but-unused version pins nothing",
    );
}

/// Chwazi must not set `web_sys_unstable_apis`, and this is the one workflow
/// invariant here that is an *absence*.
///
/// `build.yml` states it at length, and the reason is not cosmetic: the cfg is
/// not merely a feature gate, it changes the *types* web-sys exposes behind it.
/// `PointerEvent::client_x` is `i32` normally and `f64` with the cfg set.
/// Setting it in a repo whose code is written against the ungated types lints a
/// different web-sys than the one the crate is built against, and the visible
/// failure is a wall of type errors in a file that was correct.
///
/// Sibling apps in this family *do* need the flag -- they use the Screen Wake
/// Lock -- and their workflows carry it. Copying one of those files is exactly
/// how it arrives here, so the absence is asserted rather than left to
/// reviewer vigilance. The wasm build and the wasm clippy run both pass with no
/// `RUSTFLAGS` set at all, verified on a clean checkout with no
/// `.cargo/config.toml`.
#[test]
fn the_pages_build_does_not_set_the_wake_lock_cfg() {
    let Some(pages) = workflow("pages.yml") else {
        return;
    };
    let live = strip_yaml_comments(&pages);
    assert!(
        !live.contains("web_sys_unstable_apis"),
        "chwazi uses no web-sys unstable surface, so pages.yml must not set it; the cfg \
         retypes getters (`PointerEvent::client_x` is `i32` without it and `f64` with it) and \
         lints a different web-sys than the crate is built against",
    );
    assert!(
        !live.contains("\n    RUSTFLAGS:") && !live.contains("\n  RUSTFLAGS:"),
        "no `RUSTFLAGS:` key belongs in this workflow at all, step-scoped or top-level; if a \
         future dependency makes one necessary, this test and the comment above it are what \
         have to change",
    );
}

/// A deploy that can run from any branch is a deploy a stranger can run.
///
/// `pages: write` and `id-token: write` are the two permissions that let a job
/// overwrite the live site, and the token behind them is minted for the
/// repository however the workflow was reached. The project *wants* an
/// automatic deploy on every merge to master -- that is the point. What it does
/// not want is that same power on every other ref, so the invariant is the
/// narrow one that survives the convenience: master is the only ref that can
/// reach the live site, and the publishing permissions live in the one job
/// gated on it.
#[test]
fn only_master_can_reach_the_live_site() {
    let Some(pages) = workflow("pages.yml") else {
        return;
    };
    let live = strip_yaml_comments(&pages);

    // The trigger must be the named branch, not a bare `push:`. A bare push
    // deploys from every branch that exists, including a contributor's.
    assert!(
        live.contains("branches: [master]"),
        "pages.yml must trigger on `branches: [master]`, not a bare `push:`; a bare push \
         deploys from every branch, including other people's",
    );
    // A tag trigger alongside the branch trigger would publish a version that
    // was never on master.
    assert!(
        !live.contains("tags:"),
        "pages.yml must not also deploy on tags; a tagged commit that never reached master \
         would be published to the live site",
    );

    // The deploy job's gate, named so the assertion cannot be satisfied by a
    // gate on some other job.
    let deploy_job = live
        .split("\n  deploy:")
        .nth(1)
        .expect("pages.yml must have a `deploy:` job");
    assert!(
        deploy_job.contains("if:") && deploy_job.contains("github.ref == 'refs/heads/master'"),
        "the `deploy` job must be gated on the build being for master",
    );

    // ... and it must ALSO be gated on not being a fork. This file is
    // byte-identical in `wdomitrz/chwazi` and in its fork `bot-git-ai/chwazi`,
    // so a gate that tests only the branch name cannot tell the two
    // repositories apart: both have a `master`, and a push to the fork's master
    // tries to publish a site it was never given. A fork has no Pages site of
    // its own until someone enables one by hand, so every push to fork master
    // builds green -- full site check included -- and then dies at "Creating
    // Pages deployment failed ... Ensure GitHub Pages has been enabled". That is
    // a red run per push, and it is a bad way to learn that. If Pages *were*
    // enabled there, the fork would serve its own copy, which drifts from the
    // published site as soon as the two masters diverge.
    //
    // `github.event.repository.fork` is the discriminator because it needs no
    // configuration: it comes from the event, false upstream and true in the
    // fork. The obvious alternative, a repository Actions variable, has the
    // failure mode this assertion exists to prevent -- it would have to be set
    // on the *upstream* repository to publish, and no account but the user's
    // can do that, so the gate would ship as silently off on the one
    // repository where it matters. A gate that must be configured before it
    // works is a gate that ships silently off.
    //
    // The gate is read out of the live text, with comments stripped. The
    // comment block above the `if:` names both halves of the condition while
    // explaining it, so an assertion over the raw file would be satisfied by
    // that comment alone, with the clause deleted. That is not hypothetical: it
    // is the mistake the `RUSTFLAGS` assertion in `chess_clock`'s copy of this
    // file shipped with, and it shipped.
    let live_gate = live
        .split("\n  deploy:")
        .nth(1)
        .and_then(|job| job.split_once("if:").map(|(_, after)| after))
        .expect("the `deploy` job must have an `if:` gate");
    // The two halves are ONE condition, not two gates. An `if:` per job would
    // be an AND across two independent gates, and a `build`-job gate would
    // silently stop the *build* from running on the fork rather than just its
    // publish -- the opposite of what this is for. The fork rule must skip the
    // deploy and let the build stand.
    //
    // Only the gate's own LINE is read, not the rest of the job. Taking
    // everything after the first `if:` would let a second, sibling `if:` later
    // in the job satisfy the clause on its own, which is precisely the
    // two-independent-gates shape this is meant to rule out -- and it reads as
    // a working gate until the two disagree.
    let gate_line = live_gate
        .lines()
        .next()
        .expect("the `if:` gate must be a line of its own");
    assert!(
        gate_line.contains("!github.event.repository.fork"),
        "the `deploy` job must be gated on `!github.event.repository.fork`; this workflow is \
         byte-identical in the fork `bot-git-ai/chwazi`, so a branch-name-only gate publishes \
         from the fork too -- failing with 'Ensure GitHub Pages has been enabled' until Pages is \
         enabled there, and serving a divergent copy afterwards",
    );
    assert!(
        gate_line.contains("github.ref == 'refs/heads/master'"),
        "the fork rule must extend the master gate, not replace it: `deploy` must be one `if:` \
         testing both `github.ref` and `github.event.repository.fork` on the same line, not two \
         independent gates",
    );

    // And the permissions that can publish must be scoped to that job rather
    // than granted workflow-wide, so a build step or a third-party action added
    // later cannot spend them.
    //
    // Workflow-level permissions are the *first* `permissions:` key in the file,
    // and this has to be checked rather than assumed: moving `pages: write` up
    // from the job to the top of the file leaves the `build` job's own text
    // untouched, so an assertion that only reads the build job passes on a
    // workflow that has just handed the build a publish button. The key is
    // located structurally -- everything before `jobs:`, at two-space
    // indentation -- rather than by counting occurrences, because a job's own
    // `permissions:` is a legitimate second occurrence and finding it first
    // would make this check vacuous.
    let workflow_level = live.split("\njobs:").next().unwrap_or_default();
    assert!(
        !workflow_level.contains("pages: write") && !workflow_level.contains("id-token: write"),
        "pages: write and id-token: write must not be granted at workflow level; they belong \
         to the `deploy` job alone, or every job in the run inherits the power to overwrite the \
         live site",
    );

    let build_job = live
        .split("\n  build:")
        .nth(1)
        .and_then(|after| after.split("\n  deploy:").next())
        .expect("pages.yml must have a `build:` job");
    assert!(
        !build_job.contains("pages: write") && !build_job.contains("id-token: write"),
        "the `build` job must not hold pages: write or id-token: write; those belong to `deploy`",
    );
}

/// The deploy has to be handed something the upload actually produced.
///
/// `deploy-pages` v5 takes `artifact_name`. There is no `artifact_id` input:
/// passing one is reported as `Unexpected input(s) 'artifact_id'` and the action
/// falls back to its own default, which is only right while the upload side
/// also defaults to the same string. Change one side and the deploy finds no
/// artifact and fails with a bare `HttpError: Not Found` -- which names nothing
/// useful, so the cause has to be read out of the workflow.
#[test]
fn the_deploy_is_handed_the_artifact_the_build_uploaded() {
    let Some(pages) = workflow("pages.yml") else {
        return;
    };
    let live = strip_yaml_comments(&pages);

    // The input that v5 does not have. Its presence is a warning at run time
    // and never an error, so nothing else would ever report it.
    assert!(
        !live.contains("artifact_id:"),
        "pages.yml passes `artifact_id` to deploy-pages v5, which has no such input; it is \
         warned about and ignored, leaving the deploy to guess the artifact name",
    );

    // Both sides name the artifact the same way. The keys are read out of the
    // live text rather than asserting a fixed string, so the invariant is the
    // *agreement* and not the particular name.
    //
    // The upload step's own `name:` is eight spaces deep; the artifact's is
    // ten, under `with:`. Only the latter is read.
    let name_of = |key: &str, indent: usize| {
        live.lines().find_map(|line| {
            let prefix = format!("{}{key}: ", " ".repeat(indent));
            let rest = line.strip_prefix(prefix.as_str())?;
            Some(rest.trim().trim_matches('"').to_owned())
        })
    };
    let uploaded = name_of("name", 10).unwrap_or_else(|| {
        panic!("pages.yml must state the upload step's artifact `name:` so the deploy can match it")
    });
    let deployed = name_of("artifact_name", 10).unwrap_or_else(|| {
        panic!(
            "pages.yml must pass `artifact_name:` to deploy-pages, or it uses a default that \
             can drift from the upload"
        )
    });
    assert_eq!(
        uploaded, deployed,
        "the artifact the build uploads ({uploaded:?}) and the one the deploy asks for \
         ({deployed:?}) must be the same name",
    );

    // The Pages artifact is a tarball the deploy finds by name -- not a plain
    // run artifact. `upload-artifact` produces a green build followed by a
    // deploy that cannot find what it was given.
    assert!(
        live.contains("actions/upload-pages-artifact@"),
        "pages.yml must use upload-pages-artifact, not upload-artifact; the Pages artifact is a \
         single tarball the deploy action finds by name",
    );
}

/// The deploy must be a separate workflow, not extra steps in the build.
///
/// `build.yml` runs on every pull request, including from forks, where
/// `pages: write`, `id-token: write` and the `github-pages` environment do not
/// exist. Folding the deploy into it puts a publish button on every fork pull
/// request against this repository, and makes the read-only CI build fail on a
/// pull request for a reason that has nothing to do with the code.
#[test]
fn the_deploy_is_not_bolted_onto_the_pull_request_build() {
    let Some(build) = workflow("build.yml") else {
        return;
    };
    let live = strip_yaml_comments(&build);
    for forbidden in [
        "deploy-pages",
        "upload-pages-artifact",
        "pages: write",
        "id-token: write",
        "github-pages\n",
    ] {
        assert!(
            !live.contains(forbidden),
            "build.yml must not carry {forbidden:?}: it runs on fork pull requests, where the \
             Pages permissions and environment do not exist. The deploy belongs in pages.yml",
        );
    }
    // And the workflow that does deploy must actually exist, or the file above
    // is a hole with nothing in it.
    assert!(
        workflow("pages.yml").is_some(),
        "pages.yml must exist: build.yml is the read-only CI build and publishes nothing",
    );
}

/// The two build steps have to run in the order AGENTS.md gives them, here too.
///
/// This is not a style assertion. `build.rs` derives the service worker's cache
/// version from the bytes of every other file in `dist/`, **including the wasm**,
/// so running the host build first pins the cache to whatever the previous build
/// left behind -- and the site ships a worker that never invalidates. The
/// failure is invisible: the build is green and the pages load.
#[test]
fn the_pages_workflow_builds_the_wasm_before_the_shell() {
    let Some(pages) = workflow("pages.yml") else {
        return;
    };
    let live = strip_yaml_comments(&pages);

    let bindings = live
        .find("wasm-bindgen --target web")
        .unwrap_or_else(|| panic!("pages.yml must run the bindings generator into dist/"));
    let shell = live.find("touch build.rs").unwrap_or_else(|| {
        panic!(
            "pages.yml must run the host build that writes the six shell files; without the \
                 `touch`, Cargo skips build.rs and dist/ keeps only the two wasm artefacts"
        )
    });
    let wasm = live
        .find("cargo build --locked --lib --target wasm32-unknown-unknown --release")
        .unwrap_or_else(|| panic!("pages.yml must build the wasm target"));

    assert!(
        wasm < bindings,
        "the crate must be compiled to wasm before the bindings are generated from it",
    );
    assert!(
        bindings < shell,
        "build.rs hashes the wasm into the service worker's cache version, so the host build \
         must run after the bindings step -- running it first ships a worker that never \
         invalidates, with a green job",
    );

    // And it must be this crate's wasm, not a sibling's name copied over.
    assert!(
        live.contains("target/wasm32-unknown-unknown/release/chwazi.wasm"),
        "the bindings step must read chwazi.wasm; a different crate name here builds a \
         different app and still exits zero",
    );
}
