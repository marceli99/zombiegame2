# zombiegame2

Top-down 2D zombie survival shooter that runs in the browser. Single player
and online co-op for up to 4 players — one player creates a room, the rest
join with a 4-letter code (or an invite link) and the game traffic flows
peer-to-peer over WebRTC.

[![Web](https://github.com/mrcl77/zombiegame2/actions/workflows/web.yml/badge.svg?branch=main)](https://github.com/mrcl77/zombiegame2/actions/workflows/web.yml)

## Play

Open the deployed site and the game starts as soon as it has downloaded —
no click-through. Phones work too (touch sticks + aim assist; the first tap
goes fullscreen and locks landscape).

- **SINGLE PLAYER** — just play.
- **CREATE ROOM** — you host. The lobby shows a room code, and the address bar
  gains `?room=CODE` so you can copy the URL and send it to friends.
- **JOIN ROOM** — type a nick and the code (or open an invite link, which
  fills the code in for you).

Hosting means your browser runs the simulation for everyone: keep the tab in
the foreground — background tabs are throttled and the round freezes for all.

## Controls

| Key                | Action                            |
| ------------------ | --------------------------------- |
| `WASD` / arrows    | Move                              |
| Mouse              | Aim                               |
| Left click         | Shoot                             |
| Right click        | Throw grenade                     |
| `R`                | Reload                            |
| `E`                | Use / interact (hold to revive)   |
| `1` / `2` / `3`    | Switch weapon                     |
| `T`                | Chat (multiplayer)                |
| `Esc`              | Pause                             |
| `Q` / `M` (paused) | Quit to menu                      |

Gamepads are supported everywhere; touch controls switch on automatically on
phones and tablets.

## How multiplayer works

Browsers can't listen for connections, so players find each other through a
tiny **signaling server** (`signaling/`): the host asks it for a room, joiners
present the code, and it forwards the WebRTC offer/answer/ICE handshake. After
that it's out of the loop — inputs and 60 Hz snapshots go straight between the
browsers over reliable, ordered data channels. On a LAN the peers connect
directly; across the internet a public STUN server helps them find each other.

The signaling server also serves the site, so one process is a complete
deployment.

## Host it yourself

Everything is in one container image built by CI:

```sh
docker run --rm -p 8000:8000 ghcr.io/mrcl77/zombiegame2/signaling:latest
```

Then open `http://<that-machine>:8000/`. For anything beyond a LAN put it behind
a TLS-terminating proxy (Caddy, nginx, Fly.io, …) so the page is `https://` —
the game then uses `wss://` for signaling automatically.

The static site on its own (e.g. the GitHub Pages deployment) can't create
rooms; point it at a running server with `?signaling=wss://your-host/ws`.

## Development

```sh
# one-off
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version "$(grep -A1 '^name = "wasm-bindgen"$' Cargo.lock | sed -n 's/^version = "\(.*\)"/\1/p')" --locked
# wasm-opt (binaryen) on PATH is optional: -O3 pass, ~10 % smaller .wasm, faster code

./build-web.sh --fast          # build web/ (skip --fast for the wasm-opt pass)
cargo run -p signaling         # serve web/ + /ws on http://0.0.0.0:8000
```

`cargo run` / `cargo test` still work natively — that's the quickest way to
iterate on single-player logic and run the unit tests. Multiplayer is
browser-only (the menu says so if you try).

Headless browser tests (need Firefox and `pip install websockets`):

```sh
tools/web-headless.py 'http://127.0.0.1:8000/?autostart&debug' --wait 10 \
    --click 60,60 --keys Enter --after 5 --shot game.png      # single player
tools/lan_test.py                                             # host + joiner
```

Layout: the game is a library crate (`src/`), `src/main.rs` is the native
shim, `src/net_web.rs` is the browser transport, `web/index.html` the page
shell, `signaling/` the room broker.
