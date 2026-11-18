#!/usr/bin/env python3
"""SPIKE: the router, hydrated, in Chromium (agora's browser harness).

    AGORA_BROWSER_APP_PORT=<port> scripts/browser-test.sh exec \\
        python3 -I <this file> <base>

against `router-spike-server <port> <base> …`. Standard library only.
Prints each observation; exits non-zero on the first failed expectation.
"""

import json
import os
import sys
import time
import urllib.request

DRIVER = os.environ["AGORA_BROWSER_DRIVER_URL"]
PORT = os.environ["AGORA_BROWSER_APP_PORT"]
BASE = sys.argv[1] if len(sys.argv) > 1 else "/agora"
ORIGIN = f"http://localhost:{PORT}"
POST = "5b1ce000-0000-4000-8000-{:012x}"
TIMEOUT = 20


def call(method, path, body=None):
    data = None if body is None else json.dumps(body).encode()
    req = urllib.request.Request(DRIVER + path, data=data, method=method,
                                 headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=60) as resp:
        return json.load(resp)["value"]


def api_reads():
    # The server's own count, asked from the host (not through the page)
    with urllib.request.urlopen(f"http://127.0.0.1:{PORT}/__spike/api-reads") as r:
        return int(r.read())


def status(path):
    try:
        with urllib.request.urlopen(f"http://127.0.0.1:{PORT}{path}") as r:
            return r.status
    except urllib.error.HTTPError as e:
        return e.code


class Browser:
    def __init__(self):
        caps = {"capabilities": {"alwaysMatch": {
            "browserName": "chrome",
            "goog:loggingPrefs": {"browser": "ALL"},
            "goog:chromeOptions": {"args": ["--headless=new", "--no-sandbox", "--disable-dev-shm-usage"]},
        }}}
        self.sid = call("POST", "/session", caps)["sessionId"]

    def js(self, script, *args):
        return call("POST", f"/session/{self.sid}/execute/sync", {"script": script, "args": list(args)})

    def go(self, url):
        call("POST", f"/session/{self.sid}/url", {"url": url})

    def on_new_document(self, source):
        call("POST", f"/session/{self.sid}/goog/cdp/execute",
             {"cmd": "Page.addScriptToEvaluateOnNewDocument", "params": {"source": source}})

    def logs(self):
        return call("POST", f"/session/{self.sid}/se/log", {"type": "browser"})

    def quit(self):
        call("DELETE", f"/session/{self.sid}")


def wait(b, script, what):
    start = time.monotonic()
    while time.monotonic() - start < TIMEOUT:
        try:
            if b.js(script):
                return
        except Exception:
            pass
        time.sleep(0.05)
    try:
        print("  at", b.js("return location.href"), "\n  html:", b.js("return document.getElementById('agora-app').outerHTML")[:1500])
        for entry in b.logs():
            print("    ", entry.get("level"), entry.get("message")[:400])
    except Exception as e:
        print("  (no diagnostics:", e, ")")
    raise SystemExit(f"FAIL: never: {what}")


def expect(cond, what, detail=""):
    print(("  ok   " if cond else "  FAIL ") + what + (f"  [{detail}]" if detail else ""), flush=True)
    if not cond:
        raise SystemExit(f"FAIL: {what}")


# Before the page's own scripts: keep the island as the server sent it
WATCH = r"""
new MutationObserver(function (records, observer) {
  var loader = document.querySelector('script[type="module"]');
  var root = document.getElementById('agora-app');
  if (!loader || !root || window.__ssr) return;
  observer.disconnect();
  window.__ssr = { nodes: Array.from(root.querySelectorAll('*')), text: root.textContent };
}).observe(document, { childList: true, subtree: true });
"""

HYDRATED = ("var a = document.querySelector('#agora-app a.stretched-link')"
            " || document.querySelector('#agora-app div > a.pill-btn[href*=community]');"
            " return !!a && a.hasAttribute('data-dioxus-id');")

SAME = """var root = document.getElementById('agora-app');
var now = Array.from(root.querySelectorAll('*'));
var then = window.__ssr.nodes;
var kept = now.filter(function (n) { return then.indexOf(n) >= 0; });
return { then: then.length, now: now.length,
  same: then.length === kept.length && then.every(function (n, i) { return n === kept[i]; }) };"""

NAVS = "return performance.getEntriesByType('navigation').length + ':' + performance.timeOrigin;"


def main():
    b = Browser()
    try:
        b.on_new_document(WATCH)
        print(f"base {BASE!r}, origin {ORIGIN}")

        print("1. first load of the feed: server HTML, hydrated, zero API reads")
        reads0 = api_reads()
        b.go(f"{ORIGIN}{BASE}/feed")
        wait(b, "return !!window.__ssr", "the page parsed")
        expect("Fixture post 1" in b.js("return window.__ssr.text"), "the posts were in the server's HTML")
        wait(b, HYDRATED, "hydrated (the router's Link got its listener)")
        same = b.js(SAME)
        expect(same["same"], "every server element is the same object after hydration", same)
        expect(api_reads() == reads0, "no API read on first load", api_reads() - reads0)
        expect(b.js("return location.pathname + location.search") == f"{BASE}/feed", "the URL is unchanged")
        doc0 = b.js(NAVS)
        b.js("window.__marker = 'same document';")

        print("2. click a post's title (a router Link): client-side, one API read")
        href = b.js("return document.querySelector('#agora-app a.stretched-link').getAttribute('href')")
        expect(href == f"{BASE}/post/{POST.format(1)}", "the Link's href carries the base", href)
        b.js("document.querySelector('#agora-app a.stretched-link').click()")
        wait(b, "return document.getElementById('agora-app').textContent.indexOf('The body of fixture post 1') >= 0",
             "the post page drawn")
        expect(b.js("return location.pathname") == f"{BASE}/post/{POST.format(1)}", "the URL is the post's")
        expect(b.js("return window.__marker") == "same document", "no full page load (marker kept)")
        expect(b.js(NAVS) == doc0, "same document (navigation entries, timeOrigin)")
        expect(api_reads() == reads0 + 1, "exactly one API read (the post)", api_reads() - reads0)

        print("3. back: the feed again, client-side (refetched: nothing is cached)")
        b.js("history.back()")
        wait(b, f"return location.pathname === '{BASE}/feed'", "the feed's URL again")
        wait(b, "return document.getElementById('agora-app').textContent.indexOf('Fixture post 6') >= 0", "x")
        expect(b.js("return window.__marker") == "same document", "back: no page load")
        expect(api_reads() == reads0 + 2, "back: the feed refetched once", api_reads() - reads0)

        print("4. forward: the post again")
        b.js("history.forward()")
        wait(b, "return document.getElementById('agora-app').textContent.indexOf('The body of fixture post 1') >= 0",
             "the post page again")
        expect(b.js("return window.__marker") == "same document", "forward: no page load")
        expect(b.js("return location.pathname") == f"{BASE}/post/{POST.format(1)}", "forward: URL")

        print("5. the post's community link (Link) from the post page")
        b.js("document.querySelector('#agora-app a.pill-btn').click()")
        wait(b, "return location.pathname.endsWith('/community/meta-governance') && "
                "document.getElementById('agora-app').textContent.indexOf('Fixture post 3') >= 0",
             "the community's feed")
        txt = b.js("return document.getElementById('agora-app').textContent")
        expect("Fixture post 4" not in txt, "only the community's posts")
        expect(b.js("return window.__marker") == "same document", "no page load")

        print("6. a plain <a> in the feed (sort: still a plain link) is a full load")
        plain = b.js("return document.querySelector('#agora-app .sort-bar a:nth-child(2)').getAttribute('href')")
        print("     its href (site_path()):", plain, "-> server says", status(plain.split('#')[0]))
        if not plain.startswith(BASE + "/"):
            plain = None
        if plain is None or BASE == "":
            print("     (skipped: at this base the plain href is not the router's page)")
        else:
            b.js("document.querySelector('#agora-app .sort-bar a:nth-child(2)').click()")
            wait(b, "return window.__marker === undefined && !!window.__ssr", "a new document")
            wait(b, HYDRATED, "hydrated again")
            expect(b.js(SAME)["same"], "server elements kept after hydration")
            print("     at", b.js("return location.pathname + location.search"))

        print("7. reload a deep link: the server draws the post, no API read")
        reads = api_reads()
        b.go(f"{ORIGIN}{BASE}/post/{POST.format(3)}")
        wait(b, "return !!window.__ssr", "parsed")
        expect("The body of fixture post 3" in b.js("return window.__ssr.text"), "the post in the server's HTML")
        wait(b, HYDRATED, "hydrated")
        expect(b.js(SAME)["same"], "same objects after hydration")
        expect(api_reads() == reads, "no API read", api_reads() - reads)

        print("8. a non-canonical query and a fragment, after hydration")
        b.go(f"{ORIGIN}{BASE}/feed?sort=active&offset=0")
        wait(b, HYDRATED, "hydrated")
        print("     /feed?sort=active&offset=0 ->", b.js("return location.pathname + location.search"))
        b.go(f"{ORIGIN}{BASE}/post/{POST.format(2)}#comment-abc")
        wait(b, HYDRATED, "hydrated")
        time.sleep(0.3)
        print("     #comment-abc ->", repr(b.js("return location.pathname + location.search + location.hash")))
        b.go(f"{ORIGIN}{BASE}/")
        wait(b, HYDRATED, "hydrated")
        print("     / ->", b.js("return location.pathname + location.search"))

        print("9. not found")
        print("     GET", f"{BASE}/nope", "->", status(f"{BASE}/nope"))
        print("     GET", f"{BASE}/about", "->", status(f"{BASE}/about"))
        print("     GET", "/agora/api/social/feed", "->", status("/agora/api/social/feed"))

        print("browser console (all levels):")
        for entry in b.logs():
            print("    ", entry.get("level"), entry.get("message")[:240])
        print("PASS")
    finally:
        b.quit()


main()
