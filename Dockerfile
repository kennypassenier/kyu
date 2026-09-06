# Two stages on the same Debian the LXCs run (T8): a glibc binary that
# also works copied out of the image. The runtime stage has no shell
# tools, so the container HEALTHCHECK uses the binary's own --healthcheck.
FROM rust:1.97-slim-trixie AS build
RUN apt-get update -qq && apt-get install -y -qq --no-install-recommends pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY . .
RUN cargo build --release --locked

FROM debian:trixie-slim
RUN apt-get update -qq && apt-get install -y -qq --no-install-recommends ca-certificates libssl3t64 && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --home /var/lib/kyu --shell /usr/sbin/nologin kyu \
    && mkdir -p /var/lib/kyu && chown kyu:kyu /var/lib/kyu
COPY --from=build /src/target/release/kyu /usr/local/bin/kyu
USER kyu
ENV KYU_LISTEN=0.0.0.0:8080 KYU_STATE_DIR=/var/lib/kyu
EXPOSE 8080
VOLUME ["/var/lib/kyu"]
# Self-update is off inside an image by detection (AR8); updates are a new image.
HEALTHCHECK --interval=30s --timeout=5s --retries=3 CMD ["/usr/local/bin/kyu", "--healthcheck"]
ENTRYPOINT ["/usr/local/bin/kyu"]
