# Two stages: a static musl binary (G1, 2026-09-09) that carries its own
# libc rather than trusting the build image's glibc to match every LXC's
# — kyu 3.1.0's attempted native deploy to Debian 12 (glibc 2.36) failed
# outright against a binary built for trixie's glibc 2.39. musl's own
# pure-Rust crypto stack (rustls, no openssl-sys in this dependency tree)
# needs no libssl-dev either.
#
# The runtime stage is `distroless/static:nonroot` (T9, frozen at Phase 3):
# CA certs and the nonroot user (65532) come with the base image, so there
# is nothing left to apt-get or useradd — restored here, since the
# chassis-rs scaffold's own Dockerfile template had silently replaced it
# with a plain debian-slim + a hand-rolled uid 10001 user, contradicting
# what T9, AR12, README.md and compose.yml all still say the image is.
# `musl-tools` only in the build stage: the musl target itself comes from
# rust-toolchain.toml's own `targets` list once `COPY . .` brings that
# file into view — a `rustup target add` run here, before it exists,
# lands on a different toolchain resolution than the one `cargo build`
# uses two lines down, and the target quietly isn't there when it matters
# (found the hard way).
FROM rust:1.97-slim-trixie AS build
RUN apt-get update -qq && apt-get install -y -qq --no-install-recommends musl-tools && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY . .
RUN CC_x86_64_unknown_linux_musl=musl-gcc cargo build --release --locked --target x86_64-unknown-linux-musl
# distroless has no shell to mkdir with; the state directory is created
# here, with the target image's own nonroot ownership, and copied over.
RUN mkdir -p /out/var/lib/kyu

FROM gcr.io/distroless/static:nonroot
COPY --from=build --chown=nonroot:nonroot /out/var/lib/kyu /var/lib/kyu
COPY --from=build /src/target/x86_64-unknown-linux-musl/release/kyu /usr/local/bin/kyu
ENV KYU_LISTEN=0.0.0.0:8080 KYU_STATE_DIR=/var/lib/kyu
EXPOSE 8080
VOLUME ["/var/lib/kyu"]
# Self-update is off inside an image by detection (AR8); updates are a new image.
HEALTHCHECK --interval=30s --timeout=5s --retries=3 CMD ["/usr/local/bin/kyu", "--healthcheck"]
ENTRYPOINT ["/usr/local/bin/kyu"]
