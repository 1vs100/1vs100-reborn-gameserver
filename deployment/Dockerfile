# Build from the repo root: docker build -f deployment/Dockerfile --target <gameserver|nginx> .
# Builder and runtime must share a Debian release: a binary linked against a newer glibc than the
# runtime's does not start ("GLIBC_2.xx not found").
FROM rust:1-trixie AS rust-build
WORKDIR /src
# Only what the crate compiles or embeds, so a frontend edit keeps this stage cached (unless
# GAMESERVER_BUILD_COMMIT changes with it).
COPY Cargo.toml Cargo.lock ./
COPY src src
COPY assets assets
COPY fixtures fixtures
COPY keys keys
# A workspace member must be there for the workspace to load; the image builds the server alone,
# so it links no HTTP client.
COPY question-import/Cargo.toml question-import/Cargo.toml
COPY question-import/src question-import/src
# The context has no .git: the commit and, for a reproducible build, its time come in as arguments.
ARG GAMESERVER_BUILD_COMMIT
ARG SOURCE_DATE_EPOCH
COPY build.rs ./
RUN cargo build --release --locked -p onevs100-gameserver \
 && target/release/openapi_dump > openapi.json \
 && target/release/openapi_dump status > status-openapi.json

FROM node:24-trixie-slim AS ui-build
RUN npm install -g pnpm@12
WORKDIR /src/frontend
COPY frontend/package.json frontend/pnpm-lock.yaml frontend/pnpm-workspace.yaml ./
RUN pnpm install --frozen-lockfile
COPY frontend .
COPY --from=rust-build /src/openapi.json .
RUN pnpm build:client

FROM node:24-trixie-slim AS site-build
RUN npm install -g pnpm@12
WORKDIR /src/site
COPY site/package.json site/pnpm-lock.yaml site/pnpm-workspace.yaml ./
RUN pnpm install --frozen-lockfile
COPY site .
COPY --from=rust-build /src/status-openapi.json openapi.json
RUN pnpm build:client

FROM debian:trixie-slim AS gameserver
COPY --from=rust-build /src/target/release/onevs100-gameserver /usr/local/bin/
ENTRYPOINT ["onevs100-gameserver"]

FROM nginx:stable AS nginx
COPY --from=ui-build /src/frontend/dist /app/frontend/dist
COPY --from=site-build /src/site/dist /app/site/dist
COPY deployment/nginx /app/deployment/nginx
ENV NGINX_RUN_DIR=/tmp/nginx
# The host's nginx reaches the published port through the compose network's gateway.
ENV NGINX_REAL_IP_FROM="10.0.0.0/8 172.16.0.0/12 192.168.0.0/16 fc00::/7"
ENTRYPOINT ["/app/deployment/nginx/serve.sh", "run"]
