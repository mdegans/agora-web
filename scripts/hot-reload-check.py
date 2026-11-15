#!/usr/bin/env python3
"""Watch `dx serve` hot-reload the island in a real Chromium.

Evidence that `just dev` does what the README says, not a CI test: it edits
source files in this checkout (and puts them back), and needs a running
`just dev` and agora's browser harness. Rerun it after a dioxus bump.

    # in this checkout:  just dev 8089
    # in agora (for the posts; without it the plain-function step is skipped):
    #   AGORA_DEV_PROXY_PORT=8089 just dev
    # then, from agora:
    AGORA_BROWSER_APP_PORT=8089 scripts/browser-test.sh exec \\
        python3 -I <this checkout>/scripts/hot-reload-check.py <this checkout>

It loads http://localhost:<port>/agora/app/, sets a marker on `window`, and
makes each edit below in turn, waiting for the page to show it:

- rsx text in a `#[component]` (forum::RecentPosts's heading)
- rsx text in a plain `fn -> Element` (forum::post_card's comment count)
- a declaration in the stylesheet (assets/site.css)
- a change outside rsx (a new interpolation), which rebuilds and reloads

The first three must keep the marker (patched in place, no navigation); the
last must lose it (a reload). It also counts the page's stylesheet links and
the CSS it fetched, to show the stylesheet is loaded once.

It refuses to start if any file it edits has uncommitted changes, and undoes
each of its edits on exit (an exception, Ctrl-C, SIGTERM or SIGHUP) by
reversing that edit alone, so anything else written to the file meanwhile
stays. A SIGKILL leaves its edits in place: `git diff` shows them, and
`git checkout -- <file>` removes them.

Standard library only (WebDriver over HTTP), like scripts/browser-relay.py
in agora.
"""

import json
import os
import signal
import subprocess
import sys
import time
import urllib.request
from pathlib import Path

DRIVER = os.environ["AGORA_BROWSER_DRIVER_URL"]
PORT = os.environ["AGORA_BROWSER_APP_PORT"]
PAGE = f"http://localhost:{PORT}/agora/app/"
ROOT = Path(sys.argv[1] if len(sys.argv) > 1 else Path(__file__).parent.parent).resolve()
FORUM = ROOT / "crates/agora-web/src/forum.rs"
CSS = ROOT / "crates/agora-web/assets/site.css"
MAIN = ROOT / "crates/agora-web/src/main.rs"
TIMEOUT = 120


def call(method, path, body=None):
    data = None if body is None else json.dumps(body).encode()
    req = urllib.request.Request(
        DRIVER + path, data=data, method=method, headers={"Content-Type": "application/json"}
    )
    with urllib.request.urlopen(req, timeout=TIMEOUT) as resp:
        return json.load(resp)["value"]


class Browser:
    def __init__(self):
        caps = {
            "capabilities": {
                "alwaysMatch": {
                    "browserName": "chrome",
                    "goog:chromeOptions": {
                        "args": ["--headless=new", "--no-sandbox", "--disable-dev-shm-usage"]
                    },
                }
            }
        }
        self.sid = call("POST", "/session", caps)["sessionId"]

    def js(self, script, *args):
        return call("POST", f"/session/{self.sid}/execute/sync", {"script": script, "args": list(args)})

    def go(self, url):
        call("POST", f"/session/{self.sid}/url", {"url": url})

    def quit(self):
        call("DELETE", f"/session/{self.sid}")


def wait_for(browser, script, what):
    start = time.monotonic()
    while time.monotonic() - start < TIMEOUT:
        try:
            if browser.js(script):
                return time.monotonic() - start
        except Exception:  # mid-reload: the page is between documents
            pass
        time.sleep(0.05)
    raise SystemExit(f"FAIL: timed out after {TIMEOUT}s waiting for {what}")


# The island's text, as an expression (parenthesized: `??` binds looser
# than a method call)
TEXT = "(document.querySelector('#agora-app')?.innerText ?? '')"
MARKED = "return window.__hotReloadMarker === 'still here'"


def record(results, name, took, kept):
    print(f"  {name}: shown after {took:.2f} s, marker kept: {kept}", flush=True)
    results.append((name, took, kept))


# The edits made so far, newest last, for undo()
EDITS = []


def edit(path, old, new):
    text = path.read_text()
    if text.count(old) != 1:
        raise SystemExit(f"FAIL: expected exactly one {old!r} in {path}")
    path.write_text(text.replace(old, new))
    EDITS.append((path, old, new))


def undo():
    """Reverse this run's edits, newest first, each only where it still stands"""
    while EDITS:
        path, old, new = EDITS.pop()
        text = path.read_text()
        if text.count(new) == 1:
            path.write_text(text.replace(new, old))
        else:
            print(f"WARNING: couldn't undo the edit in {path} ({new!r} isn't there once); check `git diff`",
                  file=sys.stderr)


def refuse_dirty(paths):
    """Exit unless every file to be edited matches what git has"""
    changed = subprocess.run(
        ["git", "-C", str(ROOT), "status", "--porcelain", "--", *map(str, paths)],
        capture_output=True, text=True, check=True,
    ).stdout.strip()
    if changed:
        raise SystemExit(f"FAIL: commit or stash these first; this check edits them:\n{changed}")


def on_signal(signum, _frame):
    raise SystemExit(f"stopped by signal {signum}")


def styles(browser):
    return browser.js(
        """return {
            links: [...document.querySelectorAll('link[rel=stylesheet]')].map(l => l.getAttribute('href')),
            fetched: performance.getEntriesByType('resource').map(e => e.name).filter(n => n.includes('.css')),
            background: getComputedStyle(document.body).backgroundColor,
        }"""
    )


def main():
    refuse_dirty((FORUM, CSS, MAIN))
    for signum in (signal.SIGTERM, signal.SIGHUP):
        signal.signal(signum, on_signal)
    browser = Browser()
    results = []
    try:
        browser.go(PAGE)
        wait_for(browser, f"return {TEXT}.includes('Recent posts')", "the island to mount")
        has_posts = bool(browser.js("return document.querySelector('.post-card') !== null")) or bool(
            wait_for_optional(browser, "return document.querySelector('.post-card') !== null", 15)
        )
        # A clean start: no earlier run's edit is showing. (dx diffs against
        # the source of its last build, so after an interrupted run restart
        # `just dev` before running this again.)
        for leftover in ("(hot 1)", "(hot 2)", " agora-app."):
            if browser.js(f"return {TEXT}.includes({json.dumps(leftover)})"):
                raise SystemExit(f"FAIL: the page already shows {leftover!r}; restart `just dev` and rerun")
        if browser.js("return getComputedStyle(document.body).backgroundColor") == "rgb(42, 13, 23)":
            raise SystemExit("FAIL: the page already has the edited background; restart `just dev` and rerun")
        browser.js("window.__hotReloadMarker = 'still here'")
        before = styles(browser)
        print(f"loaded {PAGE}; posts from the dev stack: {has_posts}")
        print(f"stylesheets: {json.dumps(before)}")
        if len(before["links"]) != 1 or len(before["fetched"]) != 1:
            raise SystemExit("FAIL: the stylesheet should be linked and fetched exactly once")

        # 1. rsx text in a #[component]
        edit(FORUM, 'h1 { "Recent posts" }', 'h1 { "Recent posts (hot 1)" }')
        took = wait_for(browser, f"return {TEXT}.includes('Recent posts (hot 1)')", "the component edit")
        record(results, "rsx text in a #[component]", took, browser.js(MARKED))

        # 2. rsx text in a plain fn -> Element
        if has_posts:
            edit(FORUM, '"{comments} comments"', '"{comments} comments (hot 2)"')
            took = wait_for(browser, f"return {TEXT}.includes('comments (hot 2)')", "the plain-function edit")
            record(results, "rsx text in a plain fn -> Element", took, browser.js(MARKED))
        else:
            print("SKIP: no posts on the page (is agora's dev stack up?), so post_card draws nothing")

        # 3. the stylesheet
        edit(CSS, "    --bg: #0d1117;\n", "    --bg: #2a0d17;\n")
        took = wait_for(
            browser,
            "return getComputedStyle(document.body).backgroundColor === 'rgb(42, 13, 23)'",
            "the stylesheet edit",
        )
        after = styles(browser)
        record(results, "a declaration in assets/site.css", took, browser.js(MARKED))
        print(f"stylesheets after the CSS edit: {json.dumps(after)}")
        if len(after["links"]) != 1:
            raise SystemExit("FAIL: the CSS edit left more than one stylesheet link")

        # 4. outside rsx: a new interpolation, so a rebuild and reload
        edit(
            MAIN,
            '"Agora web client {VERSION}, built with agora-agentkit {AGENTKIT_VERSION}."',
            '"Agora web client {VERSION}, built with agora-agentkit {AGENTKIT_VERSION} {ROOT_ID}."',
        )
        took = wait_for(browser, f"return {TEXT}.includes(' agora-app.')", "the rebuild")
        record(results, "a new interpolation (outside what rsx can patch)", took, browser.js(MARKED))
    finally:
        undo()
        try:
            browser.quit()
        except Exception:
            pass

    print()
    print(f"{'edit':<52} {'seconds':>8}  marker kept (no navigation)")
    failed = False
    for name, took, kept in results:
        print(f"{name:<52} {took:>8.2f}  {kept}")
        expect_kept = not name.startswith("a new interpolation")
        if bool(kept) != expect_kept:
            failed = True
            print(f"FAIL: {name}: expected the marker {'kept' if expect_kept else 'gone'}")
    if failed:
        raise SystemExit(1)
    print("OK")


def wait_for_optional(browser, script, seconds):
    start = time.monotonic()
    while time.monotonic() - start < seconds:
        if browser.js(script):
            return True
        time.sleep(0.2)
    return False


if __name__ == "__main__":
    main()
