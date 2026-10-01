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

40 tests, no browser: 28 over the pointer state machine in `src/chooser.rs` and 12
over the committed shell in `tests/shell.rs`.

## How it works

Each finger down becomes a player: a filled disc, a black gap, and a ring — 80px
across, pulsing between 0.93× and 1.07× its size, in a colour spread around the hue
wheel by `hsl(pointerId * 223 + 263, 100%, 40%)`. The sizes and the pulse rate are
measured off screen recordings of the native app, not guessed, and the canvas is
drawn at the screen's full pixel density so the edges are smooth. A white arc sweeps each ring while a draw
runs. Two or more players and 2500ms of stillness choose one at random; its colour
expands from its circle to fill the screen, leaving the winner visible as a hole
in the colour, and two seconds after the winner lifts the app is ready again.

`src/chooser.rs` is all of that, with no DOM and no clock, which is why it can be
tested exhaustively. `src/ui.rs` is the thin browser layer around it. See
[AGENTS.md](AGENTS.md) for the details and the two subtleties of the original.

## License

AGPL-3.0-only. Rewritten from `github.com/wdomitrz/chwazi`, also AGPL-3.0.
