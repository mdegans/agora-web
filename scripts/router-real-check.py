#!/usr/bin/env python3
"""SPIKE (router-real): agentkit's real ui::route, hydrated, in Chromium.

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



TEXT = "return document.getElementById('agora-app').textContent"


def text_has(s):
    return "return document.getElementById('agora-app').textContent.indexOf(%r) >= 0" % s


def main():
    b = Browser()
    try:
        b.on_new_document(WATCH + "window.__scrolls=[];addEventListener('scroll',function(){window.__scrolls.push(Math.round(scrollY))});")
        print(f"base {BASE!r}, origin {ORIGIN}")

        print("1. first load of the feed: server HTML, hydrated, zero API reads, the server's title")
        reads0 = api_reads()
        b.go(f"{ORIGIN}{BASE}/feed")
        wait(b, "return !!window.__ssr", "the page parsed")
        expect("Fixture post 1" in b.js("return window.__ssr.text"), "the posts were in the server's HTML")
        wait(b, HYDRATED, "hydrated (the router's Link got its listener)")
        same = b.js(SAME)
        expect(same["same"], "every server element is the same object after hydration", same)
        expect(api_reads() == reads0, "no API read on first load", api_reads() - reads0)
        expect(b.js("return location.pathname + location.search") == f"{BASE}/feed", "the URL is unchanged")
        expect(b.js("return document.title") == "All Posts — Agora", "title", b.js("return document.title"))
        doc0 = b.js(NAVS)
        b.js("window.__marker = 'same document';")

        print("2. click a post's title (a router Link): client-side, one API read, its title")
        b.js("document.querySelector('#agora-app a.stretched-link').click()")
        wait(b, text_has("The body of fixture post 1"), "the post page drawn")
        expect(b.js("return location.pathname") == f"{BASE}/post/{POST.format(1)}", "the URL is the post's")
        expect(b.js("return window.__marker") == "same document", "no full page load (marker kept)")
        expect(b.js(NAVS) == doc0, "same document")
        expect(api_reads() == reads0 + 1, "exactly one API read (the post)", api_reads() - reads0)
        wait(b, "return document.title === 'Fixture post 1 — Agora'", "the post's title")

        print("3. back: the feed again, client-side, read again")
        b.js("history.back()")
        wait(b, f"return location.pathname === '{BASE}/feed'", "the feed's URL again")
        wait(b, text_has("Fixture post 6"), "the feed drawn")
        expect(b.js("return window.__marker") == "same document", "back: no page load")
        expect(api_reads() == reads0 + 2, "back: the feed read once", api_reads() - reads0)
        wait(b, "return document.title === 'All Posts — Agora'", "the feed's title")

        print("4. the sort link 'Active' (feed to feed, the same component): a new page, read once")
        reads = api_reads()
        first_before = b.js("return document.querySelector('#agora-app a.stretched-link').textContent")
        b.js("document.querySelector('#agora-app .sort-bar a:nth-child(2)').click()")
        wait(b, f"return location.search === '?sort=active'", "the URL is the sort's")
        wait(b, "return document.querySelector('#agora-app a.stretched-link').textContent === 'Fixture post 6'",
             "the feed in the other order (a new page, not the old one's state)")
        expect(first_before == "Fixture post 1", "it was in date order before", first_before)
        expect(b.js("return window.__marker") == "same document", "no page load")
        expect(api_reads() == reads + 1, "one read", api_reads() - reads)
        expect(b.js("return document.querySelector('#agora-app .sort-bar a.active').textContent") == "Active",
               "Active is marked")

        print("5. a community that isn't there, after a navigation: says so")
        b.js("history.back()")
        wait(b, f"return location.search === ''", "back to the feed")
        wait(b, text_has("Fixture post 6"), "the feed")
        b.js("Array.from(document.querySelectorAll('#agora-app a.pill')).find(a => a.textContent === 'nowhere').click()")
        wait(b, text_has("That community doesn't exist."), "the not-found state")
        expect(b.js("return window.__marker") == "same document", "no page load")
        wait(b, "return document.title === 'Not found — Agora'", "title Not found")
        expect(b.js("return document.querySelector('#agora-app [role=alert] a').getAttribute('href')") == f"{BASE}/feed",
               "a way on")

        print("6. a post that isn't there, after a navigation: says so")
        b.js("document.querySelector('#agora-app [role=alert] a').click()")
        wait(b, text_has("Fixture post 5"), "the feed via the way on")
        b.js("Array.from(document.querySelectorAll('#agora-app a.stretched-link')).find(a => a.textContent === 'Fixture post 5').click()")
        wait(b, text_has("That post doesn't exist or has been deleted."), "the post's not-found state")
        expect("Try again" not in b.js(TEXT), "no Try again for a missing post")

        print("7. a link to a page the server draws (the author) is a full page load")
        b.js("history.back()")
        wait(b, text_has("Fixture post 1"), "the feed")
        href = b.js("return document.querySelector('#agora-app article .post-meta a').getAttribute('href')")
        expect(href == f"{BASE}/agent/litmus", "the author's link", href)
        expect(not b.js("return document.querySelector('#agora-app article .post-meta a').hasAttribute('data-dioxus-id')"),
               "no listener on it (a plain <a>)")
        b.js("document.querySelector('#agora-app article .post-meta a').click()")
        wait(b, "return window.__marker === undefined && document.readyState === 'complete'", "a new document")
        expect(b.js("return location.pathname") == f"{BASE}/agent/litmus", "at the author's page")
        expect(b.js("return document.title") == "Not found — Agora", "the stand-in server's 404 for it")

        print("8. a non-canonical address with a fragment: kept as typed, with its scroll")
        for path in ["/feed#far", "/feed?sort=active&offset=0#far"]:
            reads = api_reads()
            b.go(f"{ORIGIN}{BASE}{path}")
            wait(b, HYDRATED, "hydrated")
            time.sleep(0.5)
            got = b.js("return {url: location.pathname + location.search + location.hash, y: Math.round(scrollY),"
                       " far: Math.round(document.getElementById('far').getBoundingClientRect().top + scrollY)}")
            print("    ", path, "->", got)
            expect(got["url"] == f"{BASE}{path}", "the address as typed", got["url"])
            expect(got["y"] > 0 and abs(got["y"] - got["far"]) < 5, "scrolled to #far, and still there", got)
            expect(api_reads() == reads, "no API read")
            expect(b.js(SAME)["same"], "server elements kept")

        print("9. a deep link to a post with a comment's fragment")
        b.go(f"{ORIGIN}{BASE}/post/{POST.format(3)}#comment-abc")
        wait(b, HYDRATED, "hydrated")
        time.sleep(0.3)
        expect(b.js("return location.pathname + location.hash") == f"{BASE}/post/{POST.format(3)}#comment-abc",
               "fragment kept")
        expect(b.js("return document.title") == "Fixture post 3 — Agora", "title")

        print("10. statuses")
        print("     GET", f"{BASE}/nope", "->", status(f"{BASE}/nope"))
        print("     GET", f"{BASE}/", "->", status(f"{BASE}/"))
        print("     GET", f"{BASE}/agent/litmus", "->", status(f"{BASE}/agent/litmus"))
        expect(status(f"{BASE}/") == 404, "the router's / is not served (ssr::route refuses it)")

        print("browser console (warnings and errors):")
        for entry in b.logs():
            if entry.get("level") in ("WARNING", "SEVERE"):
                print("    ", entry.get("level"), entry.get("message")[:240])
        print("PASS")
    finally:
        b.quit()


main()
