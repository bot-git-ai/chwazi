# chwazi

Chwazi Finger Chooser: put two or more fingers on a phone screen and it picks one
at random. A Rust rewrite of the original JavaScript PWA — the rules, the state
and the drawing are all Rust, compiled to WebAssembly; the page is a static shell
and a service worker.

Everything runs on the device. Nothing is stored and nothing is sent anywhere.

## Build

Two builds, in this order, because there are two targets:

```sh
rustup target add wasm32-unknown-unknown

# 1. the app itself
cargo build --locked --lib --target wasm32-unknown-unknown --release
wasm-bindgen --target web --no-typescript --out-dir dist --out-name app \
  target/wasm32-unknown-unknown/release/chwazi.wasm

# 2. the rest of the static site
touch build.rs
cargo build --release --locked
```

`wasm-bindgen` must be exactly `0.2.128`, the version pinned in `Cargo.toml`.

`dist/` is the whole publishable artifact: eight static files, no server and no
binary. Serve it with anything — `python3 -m http.server` in `dist/` is enough.

## Verify

```sh
cargo test --locked
cargo clippy --all-targets -- -D warnings
cargo clippy --lib --target wasm32-unknown-unknown -- -D warnings
```

69 tests, no browser: 29 over the pointer state machine in `src/chooser.rs`, 22
over the spin in `src/wheel.rs`, and 18 over the committed shell in
`tests/shell.rs`.

## How it works

Each finger down becomes a player, drawn as a filled disc inside a ring, pulsing
between 0.875× and 1.125× its size, in a colour spread around the hue wheel by
`hsl(pointerId * 223 + 263, 100%, 40%)`.

Once two or more are down they gather into a wheel and a white arc closes around
each of them, and then the wheel spins — 2.6 turns, fast at first, easing to a
stop on one finger over the first 2000ms of the 2500ms window. A result that lands
in front of everyone cannot be argued with. The winner is picked the instant the
last finger settles, not by the animation, so nothing that happens on screen can
change who wins; the spin is there to show you, not to decide. Then its colour
expands from its circle to fill the screen, leaving the winner visible as a hole
in the colour, the phone buzzes once, and two seconds after the winner lifts the
app is ready again.

`src/chooser.rs` is the rules, `src/wheel.rs` is the spin, and neither has a DOM or
a clock in it — which is why they can be tested exhaustively. `src/ui.rs` is the
thin browser layer around them. See [AGENTS.md](AGENTS.md) for the details, the
timings (all the original's), and the two subtleties of the original that are easy
to get wrong.

## License

AGPL-3.0-only. Rewritten from `github.com/wdomitrz/chwazi`, also AGPL-3.0.
