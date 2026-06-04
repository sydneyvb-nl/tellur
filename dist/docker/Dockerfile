# Multi-stage build for the Tellur team hub (tellur-server).
# Build from the repository root:
#   docker build -f dist/docker/Dockerfile -t tellur-server .
#
# NOTE: tellur-server is source-available under FSL-1.1-ALv2 (not Apache-2.0);
# see crates/server/LICENSE.

FROM rust:1.96-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --release -p tellur-server --bin tellur-server

FROM debian:bookworm-slim
# Run as a non-root user; persist the SQLite DB under /data.
RUN useradd --system --uid 10001 tellur \
    && mkdir -p /data && chown tellur:tellur /data
COPY --from=build /src/target/release/tellur-server /usr/local/bin/tellur-server
USER tellur
# In a container the bind is non-loopback by necessity; this is the explicit
# opt-in for that. Front the hub with a TLS-terminating proxy/ingress — the hub
# authenticates with bearer tokens but does not terminate TLS itself.
ENV TELLUR_SERVER_DB=/data/tellur-hub.db \
    TELLUR_SERVER_BIND=0.0.0.0:4920 \
    TELLUR_SERVER_ALLOW_NON_LOOPBACK=1
EXPOSE 4920
VOLUME ["/data"]
ENTRYPOINT ["tellur-server"]
