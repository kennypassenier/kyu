# mailbox — Architecture decisions

Decisions T1–T9 (tech choice) were taken at the Phase 3 gate on
2026-08-12. Ecosystem facts were verified against the live web that
day. AR-entries (Phase 4) follow below when frozen.

## T1 · Web framework: axum

axum 0.8.x (tokio team, MSRV 1.80, API stable since Dec 2024).
Handlers are plain async fns, so K2 long-polling is an awaited
notification with a timeout — no framework ceremony. Same maintainers
as the runtime (T3) keeps the async stack one family.
Rejected: actix-web 4.14 (healthy, marginally faster, no edge at
homelab load, aggressive MSRV).

## T2 · Storage: SQLite via rusqlite

rusqlite 0.40.x with the `bundled` feature (compiles SQLite 3.53 into
the binary; clean musl static builds). WAL mode with
`synchronous=FULL` — explicitly set, never default-trusted — is the
K12 contract: confirmed means fsynced. All engine state (messages,
cursors, leases, dead letters) lives in one database so every delivery
transition is a single SQL transaction. Confirms the Phase 1 lean.
Rejected: redb 4.1 (pure-Rust supply chain, but key-value only —
hand-rolled indexes and claim logic for no functional gain).

## T3 · Async runtime: tokio

tokio 1.x. async-std is discontinued (Mar 2025); smol has a minimal
ecosystem; axum requires tokio anyway. Recorded nuance: rusqlite is
synchronous, so store access runs on the blocking pool
(`spawn_blocking`) behind the storage module boundary — a deliberate
pattern, to be tested, not an accident.

## T4 · Dashboard rendering: minijinja + htmx 2.x

*(Kenny's pick over the askama recommendation.)* minijinja 2.x
(runtime Jinja2 templates, very active) renders the K10 dashboard
server-side; htmx 2.0.x (stability-only line, "supported in
perpetuity"; the 4.0 rewrite is explicitly ignored) provides
auto-refreshing counts and the W9 test-publish form. Template files
ship embedded in the binary (via include/embed) so the container
stays one artifact. Trade-off accepted: template errors surface at
runtime rather than compile time → Phase 7 must include
render-every-template-with-seeded-state tests to compensate.
Rejected: askama (compile-time safety, but Kenny prefers template
iteration without recompiles); embedded SPA (second toolchain, npm
churn — the re-entry tax this project exists to avoid).

## T5 · Logging: tracing

tracing + tracing-subscriber 0.3.x. Structured fields (topic,
subscription, message id) on every event; W7's JSON output is the
built-in `fmt().json()` layer; pretty output in dev.
Rejected: log + env_logger (string-based; W7 would be hand-rolled).

## T6 · Dependency policy: reluctant, policed

New direct dependencies need a one-line justification in the commit
that adds them; prefer std/existing deps; `Cargo.lock` committed;
cargo-deny in CI (advisories, license allowlist, duplicate bans) plus
a weekly scheduled advisory job. Expected direct-dep count for the
Essential set: order of ten.

## T7 · License: MIT OR Apache-2.0, public repo

*(Kenny's pick over the MIT-only recommendation.)* Dual license per
Rust-ecosystem convention (Apache adds the patent grant). Public
GitHub repo and public ghcr.io image — the LXC pulls with zero
credentials (no PAT to rotate; standing rule 10 favours fewer
standing secrets).

## T8 · Toolchain: edition 2024, track stable

Edition 2024 (settled since 1.85). Develop and CI on latest stable
(1.95 at decision time). `rust-version` declared in Cargo.toml at
whatever stable is at L0; bumped freely with a changelog note — we
build our own container, nobody else's compiler matters.
Rejected: N-2 MSRV window (buys nothing for a self-deployed binary).

## T9 · Container base: distroless/static

Multi-stage build → static musl binary → `gcr.io/distroless/static`
(~2 MB: CA certs + nonroot user included). The W6 Docker healthcheck
uses a `--healthcheck` flag on the mailbox binary itself, since the
image has no shell. Rejected: scratch (hand-rolled certs/nonroot for
2 MB), alpine (a shell and package manager the container doesn't
need).
