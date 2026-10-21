# API server + static web app in one image. Build from the REPO ROOT:
#   docker build -f crates/bim-server/Dockerfile -t buildsim-server .
# Run with DATABASE_URL set (see docker-compose.yml); listens on 8081, serves web/ at /.
FROM rust:1.83-slim-bookworm AS build
RUN apt-get update && apt-get install -y --no-install-recommends curl ca-certificates \
 && rm -rf /var/lib/apt/lists/* \
 && rustup target add wasm32-unknown-unknown \
 && curl -sSfL https://rustwasm.github.io/wasm-pack/installer/init.sh | sh
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY rules ./rules
RUN wasm-pack build crates/bim-wasm --target web --out-dir /out/pkg --release
# The workspace release profile is size-tuned for wasm; the server wants speed.
RUN CARGO_PROFILE_RELEASE_OPT_LEVEL=3 cargo build -p bim-server --release --locked \
 && cp target/release/bim-server /out/bim-server

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --no-create-home bim
COPY --from=build /out/bim-server /usr/local/bin/bim-server
COPY web/ /app/web/
COPY rules/ /app/web/rules/
COPY --from=build /out/pkg /app/web/pkg
RUN rm -f /app/web/Dockerfile /app/web/nginx.conf /app/web/README.md /app/web/*.test.mjs \
          /app/web/build.sh /app/web/build.ps1
ENV BIND_ADDR=0.0.0.0:8081 WEB_DIR=/app/web
USER bim
EXPOSE 8081
CMD ["bim-server"]
