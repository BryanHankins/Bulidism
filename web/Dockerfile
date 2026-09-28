# Static hosting for the buildsim web app (Railway, Render, Fly, or any Docker host).
# Build from the REPO ROOT so the wasm is compiled from source:
#   docker build -f web/Dockerfile -t buildsim .
FROM rust:1.83-slim AS wasm
RUN apt-get update && apt-get install -y --no-install-recommends curl ca-certificates \
 && rm -rf /var/lib/apt/lists/* \
 && rustup target add wasm32-unknown-unknown \
 && curl -sSfL https://rustwasm.github.io/wasm-pack/installer/init.sh | sh
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY rules ./rules
RUN wasm-pack build crates/bim-wasm --target web --out-dir /out/pkg --release

FROM nginx:alpine
COPY web/ /usr/share/nginx/html/
COPY rules/ /usr/share/nginx/html/rules/
COPY --from=wasm /out/pkg /usr/share/nginx/html/pkg
COPY web/nginx.conf /etc/nginx/conf.d/default.conf
RUN rm -f /usr/share/nginx/html/Dockerfile /usr/share/nginx/html/nginx.conf /usr/share/nginx/html/README.md
EXPOSE 80
