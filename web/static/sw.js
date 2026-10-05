/* PaycheckZero service worker: installable PWA with offline viewing (spec §7.4, §13.7).
 * - Static assets: cache first.
 * - Pages and their SSE content: network first, falling back to the last copy seen.
 * - Mutations, auth, API and sync requests are never cached.
 */
// The server stamps a hash of the bundled assets here, so every release
// installs a fresh worker and drops the previous caches.
const VERSION = "__PZ_ASSET_VERSION__";
const STATIC = "pz-static-" + VERSION;
const PAGES = "pz-pages-" + VERSION;
const PRECACHE = ["/static/app.css", "/static/app.js", "/static/datastar.js", "/static/icon.svg", "/offline", "/manifest.webmanifest"];

self.addEventListener("install", (event) => {
  event.waitUntil(caches.open(STATIC).then((c) => c.addAll(PRECACHE.map((u) => new Request(u, { cache: "reload" })))).then(() => self.skipWaiting()));
});

self.addEventListener("activate", (event) => {
  event.waitUntil(
    caches.keys()
      .then((keys) => Promise.all(keys.filter((k) => k !== STATIC && k !== PAGES).map((k) => caches.delete(k))))
      .then(() => self.clients.claim())
  );
});

const neverCache = (url) =>
  url.pathname.startsWith("/ui/") || url.pathname.startsWith("/api/") || url.pathname.startsWith("/__test") ||
  url.pathname === "/sync" || url.pathname === "/live" || url.pathname === "/login" || url.pathname === "/register" ||
  url.pathname.endsWith(".csv") || url.pathname.endsWith(".json");

self.addEventListener("fetch", (event) => {
  const req = event.request;
  const url = new URL(req.url);
  if (req.method !== "GET" || url.origin !== self.location.origin || neverCache(url)) return;

  if (url.pathname.startsWith("/static/")) {
    event.respondWith(
      caches.match(req).then((hit) => hit || fetch(req).then((res) => {
        if (res.ok) { const copy = res.clone(); caches.open(STATIC).then((c) => c.put(req, copy)); }
        return res;
      }))
    );
    return;
  }

  event.respondWith(
    fetch(req).then((res) => {
      if (res.ok && !res.redirected) {
        const copy = res.clone();
        caches.open(PAGES).then((c) => c.put(req, copy));
      }
      return res;
    }).catch(() =>
      caches.match(req).then((hit) => {
        if (hit) return hit;
        if (req.mode === "navigate") return caches.match("/offline");
        return new Response("offline", { status: 503, headers: { "Content-Type": "text/plain" } });
      })
    )
  );
});
