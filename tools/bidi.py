"""Minimal WebDriver BiDi driver for headless Firefox — shared by the
web-headless CLI and the multi-browser test scenarios.

    b = Browser(port=9222)
    await b.start()
    await b.navigate(url); await b.click(x, y); await b.keys(["Enter"])
    v = await b.eval("window.zgRoom"); await b.shot("out.png")
    print(b.console); await b.close()

Keys are dispatched as synthetic KeyboardEvents with an explicit `code`
(Firefox's real BiDi key actions map Enter → NumpadEnter, which winit keys
off and the game's menus ignore).  Needs `pip install websockets`.
"""
import asyncio, base64, json, os, shutil, subprocess, tempfile
import websockets

KEY_JS = """
(k) => {
  const map = {Enter:['Enter','Enter'], Escape:['Escape','Escape'], Space:[' ','Space'],
               Tab:['Tab','Tab'], Backspace:['Backspace','Backspace'],
               ArrowLeft:['ArrowLeft','ArrowLeft'], ArrowRight:['ArrowRight','ArrowRight'],
               ArrowUp:['ArrowUp','ArrowUp'], ArrowDown:['ArrowDown','ArrowDown']};
  let key, code;
  if (map[k]) [key, code] = map[k];
  else if (/^[a-z]$/i.test(k)) { key = k; code = 'Key' + k.toUpperCase(); }
  else if (/^[0-9]$/.test(k)) { key = k; code = 'Digit' + k; }
  else { key = k; code = k; }
  const c = document.getElementById('game') || document.body;
  const ev = t => new KeyboardEvent(t, {key, code, bubbles: true, cancelable: true});
  c.dispatchEvent(ev('keydown'));
  return new Promise(r => setTimeout(() => { c.dispatchEvent(ev('keyup')); r(key + '/' + code); }, 60));
}"""

PREFS = '''user_pref("webgl.force-enabled", true);
user_pref("webgl.disabled", false);
user_pref("browser.shell.checkDefaultBrowser", false);
user_pref("datareporting.policy.dataSubmissionEnabled", false);
user_pref("remote.log.level", "Warn");
user_pref("media.autoplay.default", 0);
user_pref("media.peerconnection.enabled", true);
'''


class Browser:
    def __init__(self, port, width=1280, height=720, name=None):
        self.port, self.width, self.height = port, width, height
        self.name = name or f"ff{port}"
        self.console = []
        self._pending = {}
        self._nid = 0
        self._ws = None
        self._proc = None
        self._profile = tempfile.mkdtemp(prefix=f"bidi-{self.name}-")
        self._ctx = None

    async def start(self):
        with open(os.path.join(self._profile, "user.js"), "w") as f:
            f.write(PREFS)
        self._proc = subprocess.Popen(
            ["firefox", "--headless", "--no-remote", "--profile", self._profile,
             "--remote-debugging-port", str(self.port),
             f"--window-size={self.width},{self.height}", "about:blank"],
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        for _ in range(80):
            try:
                self._ws = await websockets.connect(
                    f"ws://127.0.0.1:{self.port}/session", max_size=64 * 1024 * 1024)
                break
            except OSError:
                await asyncio.sleep(0.5)
        if self._ws is None:
            raise RuntimeError(f"{self.name}: remote agent did not come up")
        self._reader = asyncio.create_task(self._read())
        await self.send("session.new", {"capabilities": {}})
        tree = await self.send("browsingContext.getTree", {})
        self._ctx = tree["result"]["contexts"][0]["context"]
        await self.send("session.subscribe", {"events": ["log.entryAdded"]})

    async def _read(self):
        async for raw in self._ws:
            msg = json.loads(raw)
            if "id" in msg and msg["id"] in self._pending:
                self._pending.pop(msg["id"]).set_result(msg)
            elif msg.get("type") == "event" and msg["method"] == "log.entryAdded":
                p = msg["params"]
                self.console.append(f"[{p.get('level', '?')}] {p.get('text') or ''}")

    async def send(self, method, params):
        self._nid += 1
        fut = asyncio.get_event_loop().create_future()
        self._pending[self._nid] = fut
        await self._ws.send(json.dumps({"id": self._nid, "method": method, "params": params}))
        r = await fut
        if "error" in r:
            raise RuntimeError(f"{self.name}: {method} → {r['error']}: {r.get('message')}")
        return r

    async def navigate(self, url):
        await self.send("browsingContext.navigate", {"context": self._ctx, "url": url, "wait": "complete"})

    async def eval(self, js):
        r = await self.send("script.evaluate", {"expression": js, "target": {"context": self._ctx}, "awaitPromise": True})
        res = r["result"]
        if res.get("type") == "exception":
            raise RuntimeError(f"{self.name}: eval raised {res.get('exceptionDetails', {}).get('text')}")
        return res.get("result", {}).get("value")

    async def call(self, fn_js, *args):
        r = await self.send("script.callFunction", {
            "functionDeclaration": fn_js,
            "arguments": [{"type": "string", "value": str(a)} for a in args],
            "target": {"context": self._ctx}, "awaitPromise": True})
        return r["result"].get("result", {}).get("value")

    async def click(self, x, y):
        await self.send("input.performActions", {"context": self._ctx, "actions": [{
            "type": "pointer", "id": "mouse", "parameters": {"pointerType": "mouse"}, "actions": [
                {"type": "pointerMove", "x": x, "y": y}, {"type": "pointerDown", "button": 0},
                {"type": "pause", "duration": 50}, {"type": "pointerUp", "button": 0}]}]})

    async def keys(self, keys, delay=0.4):
        for k in keys:
            await self.call(KEY_JS, k)
            await asyncio.sleep(delay)

    async def type_text(self, text, delay=0.15):
        await self.keys(list(text), delay)

    async def hold(self, key, seconds):
        """Hold a key down (movement) for `seconds`."""
        js = KEY_JS.replace("return new Promise(r => setTimeout(() => { c.dispatchEvent(ev('keyup')); r(key + '/' + code); }, 60));",
                            "return key + '/' + code;")
        await self.call(js, key)
        await asyncio.sleep(seconds)
        up = KEY_JS.replace("c.dispatchEvent(ev('keydown'));", "").replace(
            "return new Promise(r => setTimeout(() => { c.dispatchEvent(ev('keyup')); r(key + '/' + code); }, 60));",
            "c.dispatchEvent(ev('keyup')); return key + '/' + code;")
        await self.call(up, key)

    async def shot(self, path):
        r = await self.send("browsingContext.captureScreenshot", {"context": self._ctx})
        with open(path, "wb") as f:
            f.write(base64.b64decode(r["result"]["data"]))
        return path

    def console_errors(self):
        return [l for l in self.console if not l.startswith("[info]") and "INFO" not in l]

    async def close(self):
        if self._reader:
            self._reader.cancel()
        if self._ws:
            await self._ws.close()
        if self._proc:
            self._proc.terminate()
            try:
                self._proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self._proc.kill()
        shutil.rmtree(self._profile, ignore_errors=True)
