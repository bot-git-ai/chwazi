// Copyright (c) 2026 Witalis Domitrz <witekdomitrz@gmail.com>
// AGPL License
// Browser lifecycle/cache plumbing only; all chooser/app logic remains Rust.
// An update never swaps itself under a live page unasked: left alone it waits
// for every page it controls to close, because an app whose entire state is
// one wasm module must not run half old code under half new. The one ask that
// exists comes from this app's own page (see `message` below), which reloads
// as the handover happens, so no page ever straddles the two versions.
// The fetch handler answers only for a URL inside this app's own directory,
// checked on every request rather than assumed from the scope: scope is a
// registration's claim, not a promise, and these apps share one origin with
// pages that are not this app. The manifest it never answers from the cache
// at all — it must always be a fresh read, because it decides what an
// install is.
const ROOT = new URL('./', self.location.href);
const CACHE = 'chwazi-finger-chooser-' + ROOT.pathname + '-__VERSION__';
const ASSETS = ['./', 'app.js', 'app_bg.wasm', 'manifest.webmanifest', 'icon-192.png', 'icon-512.png', 'icon.svg', 'index.html'].map(p => new URL(p, ROOT).href);
const IS_OWN = url => url.startsWith(ROOT.href);
// The manifest's URL on its own. It stays in ASSETS — a manifest that 404s
// should still fail the install, loudly — but it is the one precached URL the
// fetch handler will never answer out of the cache (see there).
const MANIFEST = new URL('manifest.webmanifest', ROOT).href;
self.addEventListener('install', event => {
  event.waitUntil(caches.open(CACHE).then(cache => cache.addAll(ASSETS)));
});
// The page's ask. `nudge_worker_update` in `ui.rs` sends this when it has a
// worker installed and waiting: this worker takes over at once, and the page
// — the sender — hears the resulting `controllerchange` and reloads. A page
// that never asks is never swapped, which keeps the guarantee above; the
// message is the ask, and it comes from the one client that immediately
// steps out of the way.
self.addEventListener('message', event => {
  if (event.data === 'SKIP_WAITING') self.skipWaiting();
});
self.addEventListener('activate', event => {
  event.waitUntil((async () => {
    const prefix = 'chwazi-finger-chooser-' + ROOT.pathname + '-';
    for (const key of await caches.keys()) {
      if (key.startsWith(prefix) && key !== CACHE) await caches.delete(key);
    }
    await self.clients.claim();
  })());
});
self.addEventListener('fetch', event => {
  // Deliberately leave unrelated pages, API requests, files and blobs alone:
  // a request outside this app's own directory is never this worker's to answer,
  // whatever the scope says.
  //
  // The manifest is the one own-directory GET that always goes to the network.
  // It is what the browser reads to decide what app an install *is*: while a
  // cached copy answered, a fresh install of this app could read a manifest
  // from before any change to it, and derive the same stale identity the
  // family once shared — the very bug the identity fix was meant to retire.
  // The manifest is tiny; no offline entry point needs it cached.
  const url = event.request.url;
  if (url === MANIFEST) return;
  if (event.request.method !== 'GET' || !IS_OWN(url) || !ASSETS.includes(url)) return;
  event.respondWith((async () => {
    const cache = await caches.open(CACHE);
    const cached = await cache.match(event.request);
    return cached || fetch(event.request);
  })());
});
