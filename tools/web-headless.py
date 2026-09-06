#!/usr/bin/env python3
"""Drive the web build in headless Firefox: load a URL, optionally click /
send keys / run JS, take a screenshot, dump the console.

  tools/web-headless.py URL [--wait S] [--pre-eval JS] [--setup JS]
        [--click x,y] [--keys Enter,ArrowDown,a] [--key-delay S] [--after S]
        [--eval JS]... [--shot out.png] [--port P]

Typical: serve `web/` (e.g. `cargo run -p signaling`), then
  tools/web-headless.py 'http://127.0.0.1:8000/?autostart&debug' --wait 10 \
      --click 60,60 --keys Enter --after 5 --shot game.png

`--pre-eval` runs on the origin before the page loads (seed localStorage);
`--setup` runs after `--wait`, before input; `--eval` runs before the shot.
See bidi.py for the driver itself.
"""
import argparse, asyncio, os, sys
sys.path.insert(0, os.path.dirname(__file__))
from bidi import Browser

ap = argparse.ArgumentParser()
ap.add_argument("url")
ap.add_argument("--wait", type=float, default=6.0)
ap.add_argument("--pre-eval", action="append", default=[])
ap.add_argument("--setup", action="append", default=[])
ap.add_argument("--click")
ap.add_argument("--keys", default="")
ap.add_argument("--key-delay", type=float, default=0.4)
ap.add_argument("--after", type=float, default=2.0)
ap.add_argument("--eval", action="append", default=[])
ap.add_argument("--shot")
ap.add_argument("--port", type=int, default=9222)
ap.add_argument("--width", type=int, default=1280)
ap.add_argument("--height", type=int, default=720)
args = ap.parse_args()


async def main():
    b = Browser(args.port, args.width, args.height)
    await b.start()
    try:
        if args.pre_eval:
            from urllib.parse import urlsplit, urlunsplit
            u = urlsplit(args.url)
            await b.navigate(urlunsplit((u.scheme, u.netloc, "/__pre", "", "")))
            for js in args.pre_eval:
                print("PRE-EVAL:", await b.eval(js))
        await b.navigate(args.url)
        await asyncio.sleep(args.wait)
        for js in args.setup:
            print("SETUP:", await b.eval(js))
        if args.click:
            x, y = (int(v) for v in args.click.split(","))
            await b.click(x, y)
            await asyncio.sleep(0.5)
        if args.keys:
            await b.keys([k for k in args.keys.split(",") if k], args.key_delay)
            await asyncio.sleep(args.after)
        for js in args.eval:
            print("EVAL:", await b.eval(js))
        if args.shot:
            print("SHOT:", await b.shot(args.shot))
    finally:
        print(f"CONSOLE ({len(b.console)} entries):")
        seen = {}
        for l in b.console:
            seen[l] = seen.get(l, 0) + 1
        for l, n in seen.items():
            print(("  x%d " % n if n > 1 else "  ") + l[:400])
        await b.close()

asyncio.run(main())
