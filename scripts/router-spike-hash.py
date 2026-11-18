#!/usr/bin/env python3
"""SPIKE: does hydrating the router keep a #fragment and the scroll it set?"""
import json, os, sys, time, urllib.request
DRIVER = os.environ["AGORA_BROWSER_DRIVER_URL"]; PORT = os.environ["AGORA_BROWSER_APP_PORT"]
def call(m, p, b=None):
    r = urllib.request.Request(DRIVER + p, data=None if b is None else json.dumps(b).encode(), method=m,
                               headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(r, timeout=60) as resp:
        return json.load(resp)["value"]
caps = {"capabilities": {"alwaysMatch": {"browserName": "chrome", "goog:chromeOptions": {"args": ["--headless=new", "--no-sandbox", "--disable-dev-shm-usage"]}}}}
sid = call("POST", "/session", caps)["sessionId"]
js = lambda s: call("POST", f"/session/{sid}/execute/sync", {"script": s, "args": []})
call("POST", f"/session/{sid}/goog/cdp/execute", {"cmd": "Page.addScriptToEvaluateOnNewDocument", "params": {"source":
  "window.__scrolls=[];addEventListener('scroll',function(){window.__scrolls.push(Math.round(scrollY))});"}})
try:
    for path in ["/agora/feed#far", "/agora/feed?sort=active&offset=0#far"]:
        call("POST", f"/session/{sid}/url", {"url": f"http://localhost:{PORT}{path}"})
        start = time.monotonic()
        while not js("var a=document.querySelector('#agora-app a.stretched-link');return !!a&&a.hasAttribute('data-dioxus-id')"):
            if time.monotonic() - start > 20: raise SystemExit("never hydrated")
            time.sleep(0.05)
        time.sleep(0.5)
        print(path, "->", js("return {url: location.pathname+location.search+location.hash, scrollY: Math.round(scrollY), farTop: Math.round(document.getElementById('far').getBoundingClientRect().top + scrollY), scrolls: window.__scrolls.slice(0,8), restoration: history.scrollRestoration}"))
finally:
    call("DELETE", f"/session/{sid}")
