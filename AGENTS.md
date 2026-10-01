# chwazi

Chwazi Finger Chooser: a multi-touch finger chooser for a phone passed around a
table. Every finger on the glass becomes a coloured circle; once two or more are
down they gather into a wheel, the wheel spins, and the pointer settles on one of
them, whose colour then takes the screen. All application logic, state and
rendering are Rust.

One job, one way: several fingers in, exactly one finger chosen out. No modes, no
team splitting, no multiple winners. `cargo build` generates the
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

## Timings

These are the original's, unchanged, and they are the reason the app feels the
way it does. They are listed here because a rewrite is exactly when they quietly
get "improved", and on this app every one of them is the feel:

| | | |
|---|---|---|
| `SCALING_PERIOD_MS` = 900 | one breath of the pulse |
| `REGISTRATION_TIME_MS` = 420 | a new circle drawing itself in behind its halo |
| `DRAWING_TIME_MS` = 2500 | the whole window: spin, then a beat to read it |
| `wheel::SPIN_FRACTION` = 0.55 | of that window, the spin; the rest is the result |
| `wheel::GATHER_FRACTION` = 0.34 | of the spin, the circles reaching their slots |
| `CHOSEN_PLAYER_ANIMATION_TIME_MS` = 1000 | the winner's colour flooding the screen |
| `RESTART_DELAY` = 2000 | after the winner lifts, before the next draw |

**These are no longer the original's numbers, deliberately.** The original's
window and its reset are kept, because they are the game's pacing. Everything
*inside* the window was measured against how it felt, and three of those were
wrong:

- The pulse was 1500ms and read as a swell rather than a pulse, because at that
  rate a circle spends most of its time near a turning point. 900ms puts a full
  breath in under a second.
- The circles were 58px across and two of them overlapped into one blob on a phone
  held in one hand. They are 40.5px now, with the swing reduced from ±12.5% to
  ±5.5% — smaller *and* livelier, which is the opposite of the usual trade.
- The spin overran into the reveal and the circles crawled most of the way before
  snapping together. The spin now takes just over half the window and the circles
  reach their slots in the first third of it, leaving ~900ms of a landed result.

`WINNER_RADIUS` is recomputed from the same formula on the smaller circles rather
than carried over: the original's 74.25 was sized to clear a 58px ring, and that
ring is 40.5px now. The clearance is still the author's 8px, scaled by the same
swing, and a test pins it.

A circle appears on the frame after `pointerdown` — the same frame the original
drew it on, because the original also mutated its map in the event handler and
drew on the next `requestAnimationFrame`. Nothing is deferred, queued or
throttled between a finger landing and its circle.

The two notes below that look like caveats about speed are not: `reduced-motion`
is about *whether* the pulse runs, not how fast, and the pulse's rate is the
original's 1500ms in either case.

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

- `src/chooser.rs` has 29 unit tests over the pointer state machine: add, move,
  lift, cancel, the two-player minimum, every draw-timer restart rule, winner
  selection and its anchoring, the 2000ms reset and its boundary, the pulse, the
  winner-radius geometry and the colour formula. `tests/shell.rs` asserts the
  file mentions no DOM crate at all.
- `src/wheel.rs` has 22 unit tests over the spin: that it fits any screen including
  a 320pt phone, that it is even, that it starts and stops at the top, that the
  pointer never runs backwards, that it decelerates, that it stops on a player and
  stays on that player, and that no circle ever snaps back to its finger. Several
  of those exist because the first version of the code broke them.
- `tests/shell.rs` (18 tests) asserts the invariants of the committed shell: the
  page loads the generated bindings rather than a hand-written wasm ABI, exactly
  one `<script>`, no absolute URLs, the original's viewport and
  `touch-action: none` survive, exactly one `__VERSION__`, no `skipWaiting`,
  `dist/` ignored, no build artefact tracked, the original `app.js`/`sw.js`/
  `manifest.json`/`index.html` are gone, every file the worker precaches is
  published, the splash exists and cannot swallow the first touch, and no
  user-visible text names the implementation.

There are no browser tests, no Node and no Chromium. `dist/` is gitignored, so
the release gate's exported tree never has it; `.github/workflows/build.yml` runs
both build steps on every push and then inspects the result — eight files, no
strays, no unsubstituted `__VERSION__`, bindings that export.

## Code map

- `wheel.rs`: the spinning selection animation, no DOM and no canvas. Where the
  wheel sits, how fast the pointer turns, and which player it stops on. The
  pointer rides `POINTER_OFFSET` *outside* the ring of circles, so it stops beside
  the winner rather than on it: landing on the circle hides the very thing it is
  pointing at behind a white dot and a ring of the same colour. It needs
  no state: given the same players and the same millisecond it draws the same
  wheel, which is what lets it stay in step with fingers moving underneath it.
  The circle-to-wheel move is eased, not linear, so the circles arrive rather than
  snap, and the pointer holds its angle once stopped rather than running on past
  the player it is resting on.
- `chooser.rs`: the whole app's behaviour, with no DOM, no clock and no canvas.
  `Chooser` holds the players, the draw window and the chosen player; `Player`
  holds a pointer id, a position and the instant it was chosen. Timing is passed
  in as `f64` milliseconds, so a test can place every event at an exact instant.
  The random draw takes an index rather than a generator: the browser picks one
  from `getrandom`, and a test picks the one it wants to assert about.
- `ui.rs`: wasm-only. Pointer events in, `requestAnimationFrame` out, and the
  canvas. It holds no rules — every radius, angle and colour it draws comes out
  of `chooser.rs` and `wheel.rs`. It draws the wheel, the pointer, the hint when
  fewer than two fingers are down, and the reveal.
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

## Why the winner is chosen before the animation

The wheel has to land on the winner, so the winner cannot be a consequence of
where the pointer stopped — that would make the animation an input to its own
result, and a finger landing or lifting mid-spin could change who wins.
`Chooser::pending_winner` therefore fixes the result the instant the last change
to the glass settles, and the spin is laid out to arrive at it.

It also makes the fairness argument trivial: the result is a random draw taken at
one moment, and nothing that happens on the screen afterwards can move it. That
is the claim the native app makes when it says no one can argue with the spinning
circle, and it is only true if the circle is not the input.

## Two things that are not about speed

## What is not on screen

Nothing. The canvas is the whole interface, and that is a decision rather than an
oversight:

- **No splash, and no floating icon.** There was one, and it was removed: a logo
  over black in front of an app whose input is people slapping a phone.
- **No prompt when the glass is empty.** "Put two or more fingers on the screen"
  and its "One more finger" variant both existed and both are gone. An instruction
  across the middle of the screen is in the way of the thing the player is looking
  at, and the app is meant to be understood by putting a finger down.

What replaced both is a *load* rather than a message: a new finger's circle now
draws itself in behind a collapsing white halo over `REGISTRATION_TIME_MS`. That is
the app's first loading — per finger, at the moment of arrival, and it is how the
app says *I have you* — and it needed no text to do it. The second loading is the
choice: the halo is the first, the draw is the second.

The one line of guidance that remains is the meta description, which the player
sees in the launcher or a share sheet and never while the app is open.

## One deliberate difference

The animation-frame loop is **one** `Closure` for the life of the page. A
`requestAnimationFrame` callback that forgets a fresh `Closure` every frame leaks
one JS function per frame.

## Known limitations

- Offline needs one successful online visit over HTTPS or localhost. Browser
  cache eviction, or clearing site data, removes it.
- The canvas is sized to `innerWidth`/`innerHeight` in CSS pixels, not scaled by
  `devicePixelRatio` — the original's behaviour, kept so every radius here is a
  CSS pixel and the picture is the picture the author shipped. On a
  high-density screen it is drawn softer than the browser could.
- `prefers-reduced-motion` is honoured for the splash only. The pulse and the
  spin are *not* reduced: this is a decision about whether they run, not how fast,
  and they run at the original's 1500ms and the original's window either way. The
  pulse is how a player tells their own finger apart from everyone else's, and a
  chooser whose result appears without a spin is a different app — which is the
  thing this rewrite was for.
- The winner buzzes the phone (`navigator.vibrate`, 40ms) once, on the frame the
  winner is announced. Browsers may ignore it, and iOS Safari always does; it is
  additive and nothing depends on it.
- The canvas is drawn at `devicePixelRatio`, capped at 3. The original sized its
  backing store in CSS pixels, so on any modern phone every circle was being drawn
  across a third of the pixels it occupied and upscaled by the compositor — which
  is why the edges looked soft. All arithmetic here is still in CSS pixels; the
  ratio appears only in `resize` and in one `set_transform`.
- A pointer event is handled before the frame that draws it, and the only work
  in that handler is the state change itself. The screen-reader announcement is
  written from the render loop rather than from the handler, so nothing but the
  chooser runs between a finger landing and its circle appearing.
