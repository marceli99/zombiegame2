#!/usr/bin/env python3
"""Two headless browsers play together: one creates a room, the other joins
by code, the host starts the round, both screenshot the running game.

    python3 tools/lan_test.py [--url http://127.0.0.1:8000/] [--out DIR]

Needs the signaling server up (`cargo run -p signaling`) and a web build in
web/ (`./build-web.sh --fast`).  Exit code 0 only if both ended up in-game.
"""
import argparse, asyncio, json, os, sys
sys.path.insert(0, os.path.dirname(__file__))
from bidi import Browser

ap = argparse.ArgumentParser()
ap.add_argument("--url", default="http://127.0.0.1:8000/")
ap.add_argument("--out", default=".")
ap.add_argument("--joiners", type=int, default=1, help="how many browsers join the room (1-3)")
ap.add_argument("--invite", action="store_true", help="joiners open the ?room= invite link instead of typing the code")
args = ap.parse_args()
URL = args.url.rstrip("/") + "/?autostart&debug"
SETTINGS = ('localStorage.setItem("zombiegame2:settings.json", JSON.stringify({resolution_idx:0,'
            'window_mode:"Borderless",vsync:true,fps_cap_idx:0,quality_idx:2,show_fps:true,volume:0.8})); "ok"')


def out(name):
    return os.path.join(args.out, name)


async def boot(b, url=URL, wait=9):
    await b.start()
    await b.navigate(args.url.rstrip("/") + "/__pre")
    await b.eval(SETTINGS)
    await b.navigate(url)
    await asyncio.sleep(wait)
    await b.click(60, 60)  # outside the menu panel: focus only, no item hit
    await asyncio.sleep(0.5)


async def join_by_code(client, code):
    """Main menu → JOIN ROOM → Tab to the code field → type → Enter."""
    await client.keys(["ArrowDown", "ArrowDown", "Enter"])
    await asyncio.sleep(0.8)
    await client.keys(["Tab"])
    await client.type_text(code)
    await client.keys(["Enter"])


async def join_by_invite(client, code):
    """Open ?room=CODE: lands on the join screen with the code filled in."""
    await boot(client, args.url.rstrip("/") + f"/?room={code}&autostart&debug", wait=12)
    await client.keys(["Enter"])


async def main():
    n = max(1, min(3, args.joiners))
    host = Browser(9222, name="host")
    clients = [Browser(9223 + i, name=f"client{i + 1}") for i in range(n)]
    ok = False
    try:
        await boot(host)
        print("host: CREATE ROOM")
        await host.keys(["ArrowDown", "Enter"])
        code = None
        for _ in range(20):
            await asyncio.sleep(0.5)
            code = await host.eval("window.zgRoom || ''")
            if code:
                break
        await host.shot(out("lan-host-lobby.png"))
        print("host: room code =", repr(code))
        if not code:
            print("FAIL: no room code"); return False

        # Joiners come in one after another (a burst of wasm compiles makes
        # the menus miss early keypresses).
        for c in clients:
            print(f"{c.name}: join {code} via {'invite link' if args.invite else 'typed code'}")
            if args.invite:
                await join_by_invite(c, code)
            else:
                await boot(c)
                await join_by_code(c, code)
            await asyncio.sleep(5)
            await c.shot(out(f"lan-{c.name}-lobby.png"))
        await host.shot(out("lan-host-lobby2.png"))

        print("host: start round")
        await host.keys(["Enter"])
        await asyncio.sleep(3 + 2)
        # A 1 px nudge so both sides have a non-spawn baseline for the check.
        # The first joiner walks around while we sample `window.zgState`
        # (exported under ?debug).  Its own entity — the one with id ==
        # my_id, which must not be the host's 0 — has to move; a joiner
        # driving the wrong entity (my_id stuck at 0) or none at all fails.
        state = json.loads(await clients[0].eval("window.zgState || '{}'"))
        my_id = state.get("my_id")
        print(f"{clients[0].name}: my_id = {my_id}, players = {[p['id'] for p in state.get('players', [])]}")
        if my_id != 1:
            print("FAIL: joiner's my_id should be 1"); return False
        start = next((p for p in state["players"] if p["id"] == my_id), None)

        async def pos_of(browser):
            st = json.loads(await browser.eval("window.zgState || '{}'"))
            return next((p for p in st.get("players", []) if p["id"] == my_id), None)

        def dist(p):
            return ((p["x"] - start["x"]) ** 2 + (p["y"] - start["y"]) ** 2) ** 0.5 if (p and start) else 0.0

        # Down / left / right / up cancels out overall, so track the peak
        # displacement after each leg — on the joiner (prediction) and on the
        # host (authoritative sim of the joiner's inputs).
        far = host_far = 0.0
        for key in ("s", "a", "d", "w"):
            await clients[0].hold(key, 0.8)
            far = max(far, dist(await pos_of(clients[0])))
            host_far = max(host_far, dist(await pos_of(host)))
        print(f"{clients[0].name}: moved {far:.0f}px (own view), {host_far:.0f}px (host's view); need > 40")
        if far < 40 or host_far < 40:
            print("FAIL: joiner's own player did not move"); return False
        await asyncio.gather(host.shot(out("lan-host-game.png")),
                             *(c.shot(out(f"lan-{c.name}-game.png")) for c in clients))

        # Everyone stands still and dies; the host's GameOver must reach the
        # joiners (it is announced by a final snapshot, not the 60 Hz stream).
        async def game_state(b):
            return json.loads(await b.eval("window.zgState || '{}'")).get("state")
        host_over = None
        for _ in range(40):
            await asyncio.sleep(1)
            if await game_state(host) == "GameOver":
                host_over = True
                break
        print("host: GameOver reached =", host_over)
        if host_over:
            await asyncio.sleep(2)
            states = [await game_state(c) for c in clients]
            print("joiners' state after host GameOver:", states)
            if any(st != "GameOver" for st in states):
                print("FAIL: a joiner did not follow the host into GameOver"); return False
        else:
            print("WARN: host never reached GameOver within 40 s — skipping propagation check")

        # Joiners close their tabs mid-round one by one: the host must drop
        # each from the roster (data channel close → Disconnected → despawn).
        for c in clients:
            print(f"{c.name}: leave (close tab)")
            await c.close()
            await asyncio.sleep(3)
        await host.shot(out("lan-host-after-leave.png"))
        ok = True
    finally:
        for b in [host, *clients]:
            errs = b.console_errors()
            print(f"--- {b.name} console ({len(b.console)} entries, {len(errs)} non-info) ---")
            for l in errs[:40]:
                print("  ", l[:300])
        await host.close()
        for c in clients:
            try:
                await c.close()
            except Exception:
                pass
    return ok


sys.exit(0 if asyncio.run(main()) else 1)
