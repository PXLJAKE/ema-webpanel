# One image for both ways to run it:
#  * Home Assistant add-on (HA builds this file; the app detects the Supervisor)
#  * standalone container:  docker run -p 8099:8099 -v ema-data:/data ghcr.io/pxljake/ema-webpanel
#    Everything (systems, "allow control", password) is then set up in the web UI.

# Stage 1: build the static Rust binary
FROM rust:1-alpine AS build
RUN apk add --no-cache musl-dev
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY web ./web
RUN cargo build --release --locked

# Stage 2: tiny runtime image (the binary serves the UI and talks to the panel)
FROM alpine:3.20
COPY --from=build /src/target/release/ema-panel /usr/local/bin/ema-panel
ENV DATA_DIR=/data PORT=8099
VOLUME /data
EXPOSE 8099
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s CMD wget -q -O /dev/null http://127.0.0.1:${PORT}/api/health || exit 1
LABEL org.opencontainers.image.title="Alarm Keypad (EMA)" \
      org.opencontainers.image.description="Modern web keypad for a compatible intrusion alarm panel. Unofficial." \
      org.opencontainers.image.source="https://github.com/PXLJAKE/ema-webpanel" \
      org.opencontainers.image.licenses="MIT"
CMD ["/usr/local/bin/ema-panel"]
