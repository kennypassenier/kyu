# Two stages on the same Debian the LXCs run (T8, chassis 3.0.0): a glibc
# binary that also works copied out of the image. The runtime stage has no
# shell tools, so the container HEALTHCHECK uses the binary's own
# --healthcheck. The state volume stays /data and the user keeps uid 65532
# (distroless nonroot until 2.x), so an existing volume needs no chown.
FROM rust:1.97-slim-trixie AS build
RUN apt-get update -qq && apt-get install -y -qq --no-install-recommends pkg-config libssl-dev git && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY templates ./templates
COPY static ./static
RUN cargo build --release --locked && strip target/release/kyu

FROM debian:trixie-slim
RUN apt-get update -qq && apt-get install -y -qq --no-install-recommends ca-certificates libssl3t64 && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 65532 --home /data --shell /usr/sbin/nologin kyu \
    && mkdir -p /data && chown kyu:kyu /data
COPY --from=build /src/target/release/kyu /usr/local/bin/kyu
USER kyu
ENV KYU_LISTEN=0.0.0.0:8080 KYU_STATE_DIR=/data
EXPOSE 8080
VOLUME ["/data"]
# Self-update is off inside an image by detection (AR8); updates are a new image.
HEALTHCHECK --interval=30s --timeout=5s --retries=3 --start-period=60s \
    CMD ["/usr/local/bin/kyu", "--healthcheck"]
ENTRYPOINT ["/usr/local/bin/kyu"]
