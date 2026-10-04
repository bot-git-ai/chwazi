#!/bin/sh
# Build the whole static site, in the order the order matters in.
#
# `cargo build` alone is not enough and never was. Two targets means two builds:
#
#   1. compile the crate to wasm and run the bindings generator, which writes
#      `dist/app.js` and `dist/app_bg.wasm` -- the app itself;
#   2. the host build, which runs `build.rs` to write the other six files and to
#      derive the service worker's cache version from the bytes of the other
#      seven, *including the wasm*.
#
# Step 2 has to run second or the cache version is pinned to whatever the last
# build left behind. And step 2 will not run by itself: `build.rs` writes into
# the source tree rather than `OUT_DIR`, so Cargo cannot see that anything
# changed and skips it for an otherwise identical invocation. Hence the touch.
#
# This exists because `cobalt release` runs step 2 and only step 2. A release
# therefore refreshes the six files `build.rs` owns and leaves the two wasm
# artefacts exactly as they were -- which, after a release that changed `src/`,
# is a wasm that predates the source sitting next to a fresh `index.html`. Run
# this after every release.
#
# `wasm-bindgen` must be exactly the version pinned in Cargo.toml (0.2.128).
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
cd "$root"

WASM_BINDGEN=${WASM_BINDGEN:-"$HOME/.cargo/bin/wasm-bindgen"}
[ -x "$WASM_BINDGEN" ] || WASM_BINDGEN=wasm-bindgen

echo "==> 1/2  the app: wasm32 build and bindings"
cargo build --locked --lib --target wasm32-unknown-unknown --release
"$WASM_BINDGEN" --target web --no-typescript --out-dir dist --out-name app \
  target/wasm32-unknown-unknown/release/chwazi.wasm

echo "==> 2/2  the rest of the site"
touch build.rs
cargo build --release --locked

echo "==> checking the site is whole"
expected="app.js app_bg.wasm icon-192.png icon-512.png icon.svg index.html manifest.webmanifest service-worker.js"
missing=""
for file in $expected; do
  if [ -s "dist/$file" ]; then
    printf '  ok    %s (%s bytes)\n' "$file" "$(wc -c < "dist/$file" | tr -d ' ')"
  else
    missing="$missing $file"
  fi
done
printf '%s\n' $expected >"${TMPDIR:-/tmp}/chwazi-expected-files"
extra=$(cd dist && ls | grep -x -F -v -f "${TMPDIR:-/tmp}/chwazi-expected-files" || true)
if [ -n "$missing" ] || [ -n "$extra" ]; then
  echo "incomplete -- missing:${missing:- none}; unexpected:${extra:- none}" >&2
  exit 1
fi
grep -q '__VERSION__' dist/service-worker.js && {
  echo "dist/service-worker.js still carries the __VERSION__ placeholder" >&2
  exit 1
}
grep -q '__wbindgen_start' dist/app.js || {
  echo "dist/app.js has no start function; the app would load and never run" >&2
  exit 1
}
echo "  ok    eight files, no strays, cache version substituted, start function present"
