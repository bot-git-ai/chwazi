# chwazi

Chwazi Finger Chooser: a multi-touch finger chooser for a phone passed around a
table. Every finger on the glass becomes a coloured circle; after 2500ms of
stillness one of them is chosen at random and its colour takes the screen. All
application logic, state and rendering are Rust. `cargo build` generates the
**static PWA** into `./dist` — a front-end-only site any file host can serve,
with no server and no runtime dependency on a binary. Nothing is stored, nothing
is sent anywhere.

AGPL-3.0-only. See `LICENSE`.

This is a standalone, private repository, seeded with the history of the public
JavaScript PWA it replaces (`github.com/wdomitrz/chwazi`). The rewrite keeps the
author's behaviour, geometry, colours and wording; the JavaScript is gone.

## Build and run

Two builds, because there are two targets. Nothing generated is committed.

```
# 1. the site: compile the crate to wasm and run the bindings generator
rustup target add wasm32-unknown-unknown
cargo build --locked --lib --target wasm32-unknown-unknown --release
wasm-bindgen --target web --no-typescript --out-dir dist --out-name app \
  target/wasm32-unknown-unknown/release/chwazi.wasm

# 2. the rest of the site
touch build.rs
cargo build --release --locked
```

Step 1 writes `dist/app.js` and `dist/app_bg.wasm`; step 2 adds the six files
`build.rs` owns. The order matters: step 2's cache hash covers the wasm, so
running it first would pin a version to whatever the previous build left behind.

The `touch build.rs` is not redundant. `build.rs` writes into the source tree
rather than `OUT_DIR`, so Cargo cannot see that anything changed and will not
re-run it for a second identical invocation, leaving a `dist/` with the two wasm
artefacts and none of the six shell files — the exact half-built site this shape
of project has already had.

`wasm-bindgen` is pinned to `=0.2.128` in `Cargo.toml` and must match the CLI
exactly; a mismatched generator emits bindings the runtime will not load, and the
page then fails to start with "Chwazi could not start". In a bare or non-login
shell call it by absolute path (`~/.cargo/bin/wasm-bindgen`).

There is **no `[[bin]]`, no run step and no server**: this is a browser-only app,
so the build is the whole story. `dist/` can be published by nginx, Caddy, GitHub
Pages or `python3 -m http.server`.

### After any change to the chooser or the drawing

Run **both** steps again, in order. Neither artefact is committed, so there is
nothing to forget to commit, but a `dist/` built from a stale wasm will ship an
app that predates the source.

## The static site

Eight files, none of them committed:

- `app.js` and `app_bg.wasm` come from `wasm-bindgen` (step 1). They are the app
  itself and exist nowhere else in the tree.
- `index.html`, `icon.svg`, `icon-192.png`, `icon-512.png`,
  `manifest.webmanifest` and `service-worker.js` are written by `build.rs` during
  step 2.

`build.rs` writes only the files it owns, each under a scratch name and renamed
into place, so a host serving `dist/` never sees a half-written file. It does not
replace the directory, because the wasm step owns two files in there.

The service worker's cache name is derived from the bytes of every other file in
`dist/`, **including `app_bg.wasm`**, and from the worker's own source. So a
change to the Rust invalidates the cache, and so does a change to the caching
logic.

Everything the shell references is relative (`./app.js`,
`new URL('./', self.location.href)`, `start_url: "./"`), so one build works from
any subdirectory.

## Timings and sizes, measured

These are **measured off screen recordings of the native app**, not chosen. Where
each number came from is recorded, because a number nobody can re-derive is a
number nobody can correct.

| | value | measured from |
|---|---|---|
| `SCALING_PERIOD_MS` | 1000 | pixel area peaks at 1.07s, 2.03s, 3.03s, 4.03s → 0.99s apart |
| `MAX_PULSE_SCALE` | 0.07 | area varies 1.33:1; area ∝ r², so the radius swings 1.15:1 |
| whole mark | 80 CSS px across | a player circle is 241 native px on a 1080px/3x screen |
| `MARK_RADIUS` | 40 | that circle's outer edge, measured on every clean frame |
| `DOT_RADIUS` | 6.5 | the pale dot at its centre, measured the same way |
| `DOT_COLOUR` | rgb(251,242,186) | sampled from the dot itself |
| `COLOUR_LIGHTNESS` | 49% | median over 1184 saturated pixels; the web app's 40% |
| `ARC_WIDTH` | 12 | the original's, unchanged |
| `CHOSEN_PLAYER_ANIMATION_TIME_MS` | 180 | the fill is complete 5 frames into 6 at 60fps |
| `FILL_FRACTION` | 0.5 | same: 5%, 51%, 83%, done — done by the halfway point |
| `DRAWING_TIME_MS` | 2500 | the original's, kept |
| `RESTART_DELAY` | 2000 | the original's, kept |

The two that changed most are the pulse and the reveal, and both changed for the
same reason: the original's values are a third to a quarter again as slow as the
app people actually use.

**The mark is a solid disc with a pale dot at its centre.** That is all of it: no
gap, no separate ring, and the colour runs unbroken from 13 to 36 CSS px with black
outside 40.

Two builds have now got this wrong, in opposite directions, and both were caught by
the recordings rather than by reasoning. The first merged a disc and a ring painted
edge to edge in one colour, which is a blob. The second "fixed" that by inserting a
3px black gap and a separate ring -- which the native app does not have either; I had
read them off a blurred 340px crop of a frame where two circles happened to overlap.

The white dot is what settled it, and for a reason worth keeping: it gives a frame
of video an unambiguous centre to scan radially from. Scanning out from the dot
shows one solid disc and nothing else, which no amount of squinting at a thumbnail
had managed to establish.

The dot is also why it *looks* the way it does rather than merely measuring right: a
disc of colour with a bright centre reads as a bead of light, which on a black screen
is the whole impression. `tests/shell.rs` pins the draw order, the sizes, the
lightness, and the absence of `destination-out` -- so a ring that comes back as a
draw call rather than as a constant cannot slip through.

**Smoothness is resolution, not anti-aliasing.** The canvas is sized to
`innerWidth × devicePixelRatio` (capped at 3) and drawn through a transform of the
same ratio, so a 40px circle is rasterised across 120 device pixels rather than
40 and stretched by the compositor. Sizing it in CSS pixels -- as this did, and as
the 2013 original did -- is visible as jagged edges on any modern phone. All the
arithmetic stays in CSS pixels, so no constant and no test changed with it.

**The reveal is a smoothstep over the first half, then a hold.** Measured 5%, 51%,
83%, complete across five frames — so the fill is done at the halfway point and
the rest of the window is the winner resting in its own colour. That is why the
native one reads as a snap rather than a sweep, and it is not something a shorter
duration alone would have got: a *linear* radius is fastest exactly where the
screen is densest, so it appears to stall halfway.

`MIN_WINNER_RADIUS` is derived from the ring rather than carried over, so it moves
whenever the ring or the swing does. Its one invariant is the author's: at the top
of the pulse the fill clears the winner's own ring by exactly `CHOSEN_SEPARATION`.

A circle appears on the frame after `pointerdown` — the same frame the original
drew it on, because the original also mutated its map in the event handler and
drew on the next `requestAnimationFrame`. Nothing is deferred, queued or
throttled between a finger landing and its circle.

The note below that looks like a caveat about speed is not: `reduced-motion` is
about *whether* the pulse runs, not how fast, and the pulse's rate is the same in
either case.

## The icon

`assets/icon.svg` is the author's original Material Symbols **"touch_long"**,
`#434343`, on transparent, kept byte for byte (1144 bytes; `tests/shell.rs` pins
that). It is the committed source of truth: never deleted, never replaced by a
PNG, never redrawn. `build.rs` rasterizes the 192 and 512 install PNGs from it
with `usvg` + `resvg` + `tiny-skia` and copies the SVG itself into `dist/`, so the
favicon the shell links and the PNGs the manifest declares are the same drawing.

The SVG is published *and* precached. Leaving it out of `dist/` looks cosmetic —
the page still runs — but `caches.addAll` rejects the whole worker install on one
404, so the app silently loses offline support. `tests/shell.rs` derives the
service worker's `ASSETS` list and checks every entry against the files the build
publishes, which is what catches that.

## The manifest

`build.rs` assembles it, so its icon list cannot drift from what was actually
rasterized. `name` ("Chwazi Finger Chooser"), `short_name` ("Chwazi") and
`display` (`fullscreen`) are the original's. The original declared no colours at
all, so these are new and were chosen for one reason: **black**, because the app
is black from edge to edge, and a black splash is the only colour that does not
flash white between the launcher and the first frame. `id`, `start_url` and
`scope` are all `"./"`.

## Tests

```
cargo clippy --all-targets -- -D warnings
cargo clippy --lib --target wasm32-unknown-unknown -- -D warnings
cargo test --locked
```

- `src/chooser.rs` has 28 unit tests over the pointer state machine: add, move,
  lift, cancel, the two-player minimum, every draw-timer restart rule, winner
  selection and its anchoring, the 2000ms reset and its boundary, the pulse, the
  winner-radius geometry and the colour formula. `tests/shell.rs` asserts the
  file mentions no DOM crate at all.
- `tests/shell.rs` (12 tests) asserts the invariants of the committed shell: the
  page loads the generated bindings rather than a hand-written wasm ABI, exactly
  one `<script>`, no absolute URLs, the original's viewport and
  `touch-action: none` survive, exactly one `__VERSION__`, no `skipWaiting`,
  `dist/` ignored, no build artefact tracked, the original `app.js`/`sw.js`/
  `manifest.json`/`index.html` are gone, and every file the worker precaches is
  published.

There are no browser tests, no Node and no Chromium. `dist/` is gitignored, so
the release gate's exported tree never has it; `.github/workflows/build.yml` runs
both build steps on every push and then inspects the result — eight files, no
strays, no unsubstituted `__VERSION__`, bindings that export.

## Code map

- `chooser.rs`: the whole app's behaviour, with no DOM, no clock and no canvas.
  `Chooser` holds the players, the draw window and the chosen player; `Player`
  holds a pointer id, a position and the instant it was chosen. Timing is passed
  in as `f64` milliseconds, so a test can place every event at an exact instant.
  The random draw takes an index rather than a generator: the browser picks one
  from `getrandom`, and a test picks the one it wants to assert about.
- `ui.rs`: wasm-only. Pointer events in, `requestAnimationFrame` out, and the
  canvas. It holds no rules — every radius, angle and colour it draws comes out
  of `chooser.rs`.
- `ui.html`: the static shell. One black canvas, one module script whose whole
  body is `import('./app.js').then(m => m.default())`, and a hidden failure UI.
- `service-worker.js`: caches only a fixed app-shell allowlist, scope-specific
  content-versioned cache, atomic install, no `skipWaiting`.
- `build.rs`: writes the six files it owns into `dist/`, deriving the worker's
  cache version from the bytes of the other seven.

## Notes for anyone porting this app from the original

Two things in the original are subtler than they look, and both are pinned by a
test:

- **The pulse is symmetric.** `1 + 0.125 * sin(...)` means the circles grow to
  1.125 *and shrink to 0.875*. Anyone expecting only growth will get it wrong,
  and `MIN_WINNER_RADIUS` then looks 14px too small. It is not: the winner's ring
  is stroked at a centreline radius of 52 with a 12px stroke, so its outer edge
  is 58, which the pulse swings to 65.25, and the fill stops at 74.25 — leaving
  exactly the author's `CHOSEN_SEPARATION` of 8 at the tightest point of the
  breath. The author's arithmetic was right.
- **The reset is 2000ms after the winner lifts**, not after the draw. The winner's
  circle stays on screen after every other finger has left, because it is the
  hole in the colour, and the app is only reusable once it has gone.

One deliberate difference: the animation-frame loop is **one** `Closure` for the
life of the page. A `requestAnimationFrame` callback that forgets a fresh
`Closure` every frame leaks one JS function per frame.

## Known limitations

- Offline needs one successful online visit over HTTPS or localhost. Browser
  cache eviction, or clearing site data, removes it.
- The canvas is sized to `innerWidth`/`innerHeight` in CSS pixels, not scaled by
  `devicePixelRatio` — the original's behaviour, kept so every radius here is a
  CSS pixel and the picture is the picture the author shipped. On a
  high-density screen it is drawn softer than the browser could.
- `prefers-reduced-motion` is not honoured. This is a decision about *whether*
  the pulse runs, not about its speed — it runs at the original's 1500ms period
  either way. The pulse is the app, not decoration: it is how a player tells
  their own finger apart from everyone else's. The original ignored the setting
  too, and a chooser whose circles do not breathe is a different app.
- A pointer event is handled before the frame that draws it, and the only work
  in that handler is the state change itself. The screen-reader announcement is
  written from the render loop rather than from the handler, so nothing but the
  chooser runs between a finger landing and its circle appearing.
