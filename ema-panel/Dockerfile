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
EXPOSE 8099
CMD ["/usr/local/bin/ema-panel"]
