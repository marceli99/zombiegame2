#!/usr/bin/env python3
"""Screenshot the web build in headless Chromium (Playwright) — the
cross-check for Firefox-only findings.  WebGL2 runs on SwiftShader unless a
GPU is exposed; colours/blending are still representative.

  tools/chrome_shot.py URL [--wait S] [--keys Enter,ArrowDown] [--after S]
        [--shot out.png] [--pre-eval JS] [--eval JS]
"""
import argparse, asyncio, sys
from urllib.parse import urlsplit, urlunsplit
from playwright.async_api import async_playwright

ap = argparse.ArgumentParser()
ap.add_argument("url")
ap.add_argument("--wait", type=float, default=8.0)
ap.add_argument("--keys", default="")
ap.add_argument("--key-delay", type=float, default=0.4)
ap.add_argument("--after", type=float, default=2.0)
ap.add_argument("--shot")
ap.add_argument("--pre-eval", action="append", default=[])
ap.add_argument("--eval", action="append", default=[])
ap.add_argument("--gpu", action="store_true", help="try the real GPU instead of SwiftShader")
args = ap.parse_args()


async def main():
    async with async_playwright() as p:
        flags = ["--enable-unsafe-webgpu"]
        if args.gpu:
            flags += ["--use-gl=angle", "--use-angle=gl", "--ignore-gpu-blocklist", "--enable-gpu"]
        browser = await p.chromium.launch(headless=True, args=flags)
        page = await browser.new_page(viewport={"width": 1366, "height": 682})
        logs = []
        page.on("console", lambda m: logs.append(f"[{m.type}] {m.text}"))
        page.on("pageerror", lambda e: logs.append(f"[pageerror] {e}"))
        if args.pre_eval:
            u = urlsplit(args.url)
            await page.goto(urlunsplit((u.scheme, u.netloc, "/__pre", "", "")))
            for js in args.pre_eval:
                print("PRE-EVAL:", await page.evaluate(js))
        await page.goto(args.url)
        await asyncio.sleep(args.wait)
        await page.mouse.click(60, 60)
        if args.keys:
            for k in [k for k in args.keys.split(",") if k]:
                await page.keyboard.press(k)
                await asyncio.sleep(args.key_delay)
            await asyncio.sleep(args.after)
        for js in args.eval:
            print("EVAL:", await page.evaluate(js))
        if args.shot:
            await page.screenshot(path=args.shot)
            print("SHOT:", args.shot)
        print(f"CONSOLE ({len(logs)} entries):")
        for l in logs:
            if "INFO" not in l:
                print("  ", l[:300])
        await browser.close()

asyncio.run(main())
