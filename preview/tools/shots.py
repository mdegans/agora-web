#!/usr/bin/env python3
"""Screenshots of the preview pages through the harness's Chromium.
argv: <origin path prefix> <out dir> <page names...>"""
import base64, json, os, sys, urllib.request

DRIVER = os.environ["AGORA_BROWSER_DRIVER_URL"]
PORT = os.environ["AGORA_BROWSER_APP_PORT"]
prefix, out = sys.argv[1], sys.argv[2]
names = sys.argv[3:]
os.makedirs(out, exist_ok=True)

def call(method, path, body=None):
    data = None if body is None else json.dumps(body).encode()
    req = urllib.request.Request(DRIVER + path, data=data, method=method,
                                 headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=120) as r:
        return json.load(r)["value"]

caps = {"capabilities": {"alwaysMatch": {"browserName": "chrome",
        "goog:chromeOptions": {"args": ["--headless=new", "--no-sandbox", "--disable-dev-shm-usage", "--hide-scrollbars"]}}}}
sid = call("POST", "/session", caps)["sessionId"]
def cdp(cmd, params):
    return call("POST", f"/session/{sid}/goog/cdp/execute", {"cmd": cmd, "params": params})
def js(script):
    return call("POST", f"/session/{sid}/execute/sync", {"script": script, "args": []})
problems = []
try:
    for name in names:
        variants = [(name, False)]
        if name in ("comment", "comment-long"):
            variants.append((name + "-open", True))
        for label, open_details in variants:
            for width, scale, mobile in [(390, 2, True), (768, 1, False), (1280, 1, False)]:
                cdp("Emulation.setDeviceMetricsOverride", {"width": width, "height": 900, "deviceScaleFactor": scale, "mobile": mobile})
                call("POST", f"/session/{sid}/url", {"url": f"http://localhost:{PORT}{prefix}/{name}.html"})
                if open_details:
                    js("document.querySelectorAll('details').forEach(d => d.open = true)")
                over = js("return [document.documentElement.scrollWidth, window.innerWidth]")
                if over[0] > over[1]:
                    problems.append(f"{label}@{width}: scrolls sideways {over}")
                small = js("""return [...document.querySelectorAll('.ui-page a.ui-pill, .ui-page summary.ui-pill, .ui-page .ui-skip')]
                    .map(e => e.getBoundingClientRect()).filter(r => r.height > 0 && r.height < 44).length""")
                if width == 390 and small:
                    problems.append(f"{label}@{width}: {small} pill targets under 44px")
                h = js("return Math.ceil(document.documentElement.scrollHeight)")
                shot = cdp("Page.captureScreenshot", {"format": "png", "captureBeyondViewport": True,
                           "clip": {"x": 0, "y": 0, "width": width, "height": h, "scale": 1}})
                path = os.path.join(out, f"{label}-{width}.png")
                with open(path, "wb") as f:
                    f.write(base64.b64decode(shot["data"]))
                print(path, h)
finally:
    call("DELETE", f"/session/{sid}")
print("PROBLEMS:", problems or "none")
