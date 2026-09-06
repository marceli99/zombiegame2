# Complete self-hosted deployment in one image: the signaling server plus the
# static web build it serves.  Build from the repository root AFTER
# `./build-web.sh` has produced web/pkg (CI does exactly this):
#
#   docker build -f signaling/Dockerfile -t zombiegame2 .
#   docker run --rm -p 8000:8000 zombiegame2
#
# Put it behind any TLS-terminating proxy (Caddy, nginx, Fly.io, ...) —
# browsers only allow WebRTC + microphone-free data channels from https
# origins on the public internet, and `wss://` follows automatically.

FROM rust:1-bookworm AS build
WORKDIR /src
# The workspace manifest references the game crate, so the whole tree comes
# along — only `signaling` is compiled (bevy is never downloaded).
COPY . .
RUN cargo build --release --locked -p signaling

FROM debian:bookworm-slim
COPY --from=build /src/target/release/signaling /usr/local/bin/signaling
COPY web /srv/web
EXPOSE 8000
CMD ["signaling", "--listen", "0.0.0.0:8000", "--web", "/srv/web"]
