// Copyright (c) 2026 Witalis Domitrz <witekdomitrz@gmail.com>
// AGPL License
// Browser lifecycle/cache plumbing only; all chooser/app logic remains Rust.
// An update waits for the old tabs to close rather than swapping the wasm under
// one that is live, which for an app whose entire state is a single wasm module
// means half old code drawing over half new code. So there is no skipWaiting
// call here, and the test checks for the call and not the bare word.
// The fetch handler answers only for a URL inside this app's own directory,
// checked on every request rather than assumed from the scope: scope is a
// registration's claim, not a promise, and these apps share one origin with
// pages that are not this app.
const ROOT = new URL('./', self.location.href);
const CACHE = 'chwazi-finger-chooser-' + ROOT.pathname + '-__VERSION__';
const ASSETS = ['./', 'app.js', 'app_bg.wasm', 'manifest.webmanifest', 'icon-192.png', 'icon-512.png', 'icon.svg', 'index.html'].map(p => new URL(p, ROOT).href);
const IS_OWN = url => url.startsWith(ROOT.href);
self.addEventListener('install', event => {
  event.waitUntil(caches.open(CACHE).then(cache => cache.addAll(ASSETS)));
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
  const url = event.request.url;
  if (event.request.method !== 'GET' || !IS_OWN(url) || !ASSETS.includes(url)) return;
  event.respondWith((async () => {
    const cache = await caches.open(CACHE);
    const cached = await cache.match(event.request);
    return cached || fetch(event.request);
  })());
});
