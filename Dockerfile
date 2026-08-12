# Statically linked musl binary, so the runtime image can be
# distroless/static: no shell, no libc, nonroot by default (T9). The
# build stage keeps a C toolchain because L1 links SQLite into the binary
# (rusqlite `bundled`, T2).
FROM rust:1-alpine AS build

RUN apk add --no-cache build-base

WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release --locked && strip target/release/mailbox

# /data must belong to the nonroot user before Docker creates the volume
# from it, otherwise the store directory is root-owned and unwritable.
RUN mkdir -p /empty-data

FROM gcr.io/distroless/static:nonroot

COPY --from=build /src/target/release/mailbox /usr/local/bin/mailbox
COPY --from=build --chown=65532:65532 /empty-data /data

ENV MAILBOX_LISTEN=0.0.0.0:8080 \
    MAILBOX_DATA_DIR=/data

EXPOSE 8080
VOLUME ["/data"]
USER nonroot:nonroot

ENTRYPOINT ["/usr/local/bin/mailbox"]
