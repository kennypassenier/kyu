//! The schema SQL, one constant per version (AR3). Append only: a
//! published entry is never edited, or databases in the field diverge
//! from new ones. The runner in `super` applies them in order.

/// AR3's four tables. Times are integer milliseconds since the Unix epoch
/// (AR7). `STRICT` makes SQLite enforce the column types instead of
/// coercing them.
pub(super) const MIGRATION_1_INITIAL_SCHEMA: &str = r#"
CREATE TABLE topics (
    id           INTEGER PRIMARY KEY,
    name         TEXT    NOT NULL UNIQUE,
    retention_ms INTEGER,
    created_at   INTEGER NOT NULL
) STRICT;

CREATE TABLE subscriptions (
    id           INTEGER PRIMARY KEY,
    topic_id     INTEGER NOT NULL REFERENCES topics(id) ON DELETE CASCADE,
    name         TEXT    NOT NULL,
    state        TEXT    NOT NULL CHECK (state IN ('active', 'flagged', 'archived')),
    lease_ms     INTEGER,
    max_attempts INTEGER,
    backoff_ms   INTEGER,
    ttl_ms       INTEGER,
    created_at   INTEGER NOT NULL,
    last_poll_at INTEGER,
    UNIQUE (topic_id, name)
) STRICT;

-- `seq` is the rowid: insertion order, which is what delivery order is
-- based on (AR7). `id` is the public ULID and is never used for ordering.
CREATE TABLE messages (
    seq          INTEGER PRIMARY KEY AUTOINCREMENT,
    id           TEXT    NOT NULL UNIQUE,
    topic_id     INTEGER NOT NULL REFERENCES topics(id) ON DELETE CASCADE,
    payload      BLOB    NOT NULL,
    content_type TEXT,
    published_at INTEGER NOT NULL,
    due_at       INTEGER
) STRICT;

-- Fan-out is materialized: one row per (message, subscription), created in
-- the same transaction as the message (AR3). Every delivery state
-- transition is then a single guarded UPDATE (AR9).
CREATE TABLE deliveries (
    msg_seq          INTEGER NOT NULL REFERENCES messages(seq) ON DELETE CASCADE,
    sub_id           INTEGER NOT NULL REFERENCES subscriptions(id) ON DELETE CASCADE,
    state            TEXT    NOT NULL CHECK (state IN
                         ('pending', 'claimed', 'acked', 'dead', 'expired', 'lapsed')),
    attempts         INTEGER NOT NULL DEFAULT 0,
    lease_expires_at INTEGER,
    next_attempt_at  INTEGER,
    dead_at          INTEGER,
    expired_at       INTEGER,
    PRIMARY KEY (msg_seq, sub_id)
) STRICT;

-- The claim query: oldest deliverable row for one subscription.
CREATE INDEX deliveries_claimable
    ON deliveries (sub_id, state, next_attempt_at, msg_seq);

-- The sweeper's scans: expiring leases and settling states.
CREATE INDEX deliveries_by_state
    ON deliveries (state, lease_expires_at);

-- Retention and replay walk a topic in insertion order.
CREATE INDEX messages_by_topic
    ON messages (topic_id, seq);
"#;

/// The hub's own event topic (W11) exists from the start, so that something
/// can subscribe to it *before* the first event happens. Creating it lazily
/// would mean the only way to start listening was to wait for a failure.
///
/// `created_at` is 0: it has been there since the schema was.
pub(super) const MIGRATION_2_EVENTS_TOPIC: &str = r#"
INSERT INTO topics (name, retention_ms, created_at) VALUES ('kyu.events', NULL, 0);
"#;

/// K11's idle thresholds start as hub-wide defaults, but the knowledge that
/// a consumer only polls monthly lives with that consumer — so they are
/// overridable per subscription, like every other policy field (K7).
pub(super) const MIGRATION_3_PER_SUBSCRIPTION_IDLE: &str = r#"
ALTER TABLE subscriptions ADD COLUMN idle_flag_ms INTEGER;
ALTER TABLE subscriptions ADD COLUMN idle_archive_ms INTEGER;
"#;

/// W2's registered apps. `token` holds the ciphertext from
/// `crypto::SecretKey::seal`, never the token itself — the column is a BLOB
/// because that is what it is, and calling it anything friendlier would
/// invite someone to read it.
///
/// Revoking keeps the row and stamps `revoked_at`, rather than deleting it:
/// "this app used to exist and I turned it off" is the thing you want to
/// see six months later, and a deleted row cannot tell you that. The unique
/// index therefore covers live apps only, so a revoked name can be reused.
pub(super) const MIGRATION_4_APPS: &str = r#"
CREATE TABLE apps (
    id         INTEGER PRIMARY KEY,
    name       TEXT    NOT NULL,
    token      BLOB    NOT NULL,
    created_at INTEGER NOT NULL,
    revoked_at INTEGER
) STRICT;

CREATE UNIQUE INDEX apps_live_name ON apps (name) WHERE revoked_at IS NULL;
"#;

/// W11, amended 2026-09-18: `message.expired` is announced once per
/// subscription per window rather than once per sweep, so the subscription
/// remembers when it last announced and how many expired since. Kept in the
/// store rather than in memory so a restart cannot lose a count or repeat an
/// announcement — events are written in the transaction that caused them,
/// and this state belongs to the same transaction.
pub(super) const MIGRATION_5_EXPIRY_ANNOUNCEMENTS: &str = r#"
ALTER TABLE subscriptions ADD COLUMN expired_announced_at INTEGER;
ALTER TABLE subscriptions ADD COLUMN expired_unannounced INTEGER NOT NULL DEFAULT 0;
"#;

/// A delivery the dashboard deleted stays as a `removed` row instead of
/// going away (2026-09-30). K8's replay hands a `from=beginning`
/// subscription every retained message it has no row for, so a deleted
/// row was an invitation: on CT 109 a dead letter Kenny deleted twice came
/// back after each replay. SQLite cannot widen a CHECK in place, so the
/// table is rebuilt with the same columns, keys and indexes; nothing
/// references `deliveries`, so no foreign key has to follow it.
pub(super) const MIGRATION_6_REMOVED_DELIVERIES: &str = r#"
CREATE TABLE deliveries_v6 (
    msg_seq          INTEGER NOT NULL REFERENCES messages(seq) ON DELETE CASCADE,
    sub_id           INTEGER NOT NULL REFERENCES subscriptions(id) ON DELETE CASCADE,
    state            TEXT    NOT NULL CHECK (state IN
                         ('pending', 'claimed', 'acked', 'dead', 'expired', 'lapsed', 'removed')),
    attempts         INTEGER NOT NULL DEFAULT 0,
    lease_expires_at INTEGER,
    next_attempt_at  INTEGER,
    dead_at          INTEGER,
    expired_at       INTEGER,
    PRIMARY KEY (msg_seq, sub_id)
) STRICT;
INSERT INTO deliveries_v6
    SELECT msg_seq, sub_id, state, attempts, lease_expires_at,
           next_attempt_at, dead_at, expired_at
      FROM deliveries;
DROP TABLE deliveries;
ALTER TABLE deliveries_v6 RENAME TO deliveries;
CREATE INDEX deliveries_claimable
    ON deliveries (sub_id, state, next_attempt_at, msg_seq);
CREATE INDEX deliveries_by_state
    ON deliveries (state, lease_expires_at);
"#;

/// The dashboard's history (kp-themes 11.0.0, 2026-10-19): what happened to
/// a delivery attempt by attempt, and how much moved per hour and per day.
/// Purely additive: no existing table changes, so the migration keeps every
/// message, subscription and delivery exactly as it was.
///
/// Every table here grows with traffic and therefore has its cap from the
/// start:
///
/// - `attempts` follows the message it belongs to (the cascade removes a
///   row with its message or its subscription) and the sweeper also drops
///   rows older than 30 days, so a topic kept forever cannot grow it
///   without bound.
/// - `stats_hourly` is kept 30 days, `stats_daily` 400 days and
///   `stats_samples` 48 hours, all pruned by the sweeper.
///
/// The stats tables name topics and subscriptions by text, not by id: a
/// counter is history, and deleting an archived subscription must not erase
/// the days it was alive. `hour` is the Unix millisecond at the start of the
/// UTC hour; `day` is a Europe/Brussels calendar day as days since
/// 1970-01-01 (the dashboard shows Brussels time).
pub(super) const MIGRATION_7_ATTEMPTS_AND_STATS: &str = r#"
CREATE TABLE attempts (
    id         INTEGER PRIMARY KEY,
    msg_seq    INTEGER NOT NULL REFERENCES messages(seq) ON DELETE CASCADE,
    sub_id     INTEGER NOT NULL REFERENCES subscriptions(id) ON DELETE CASCADE,
    n          INTEGER NOT NULL,
    claimed_at INTEGER NOT NULL,
    ended_at   INTEGER,
    outcome    TEXT    NOT NULL CHECK (outcome IN
                   ('in_flight', 'acked', 'nacked', 'expired', 'dead')),
    app        TEXT,
    reason     TEXT
) STRICT;

CREATE INDEX attempts_by_delivery ON attempts (msg_seq, sub_id, id);
CREATE INDEX attempts_by_age ON attempts (claimed_at);

CREATE TABLE stats_hourly (
    topic     TEXT    NOT NULL,
    hour      INTEGER NOT NULL,
    published INTEGER NOT NULL DEFAULT 0,
    dead      INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (topic, hour)
) STRICT, WITHOUT ROWID;

CREATE INDEX stats_hourly_by_hour ON stats_hourly (hour);

CREATE TABLE stats_daily (
    topic        TEXT    NOT NULL,
    subscription TEXT    NOT NULL,
    day          INTEGER NOT NULL,
    delivered    INTEGER NOT NULL DEFAULT 0,
    dead         INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (topic, subscription, day)
) STRICT, WITHOUT ROWID;

CREATE INDEX stats_daily_by_day ON stats_daily (day);

CREATE TABLE stats_samples (
    at        INTEGER PRIMARY KEY,
    messages  INTEGER NOT NULL,
    backlog   INTEGER NOT NULL,
    dead      INTEGER NOT NULL,
    in_flight INTEGER NOT NULL
) STRICT;
"#;

/// Damping of repeated notifications (see `crate::engine::dedup`): one row
/// per (topic, key) remembers which message a publisher's dedup key last
/// produced and until when a repeat is suppressed. Purely additive.
///
/// The table is capped from the start: the sweeper removes expired rows and
/// rows of topics that no longer exist, and a publish evicts the oldest rows
/// past `crate::store::dedup::MAX_ROWS`. `message_id` is the text id, not a
/// foreign key: retention may collect the message while the window is still
/// open, and a repeat inside the window must still be suppressed.
/// `stats_hourly.deduplicated` counts the suppressed repeats per topic.
pub(super) const MIGRATION_8_DEDUP_KEYS: &str = r#"
CREATE TABLE dedup_keys (
    topic_id     INTEGER NOT NULL REFERENCES topics(id) ON DELETE CASCADE,
    key          TEXT    NOT NULL,
    message_id   TEXT    NOT NULL,
    published_at INTEGER NOT NULL,
    expires_at   INTEGER NOT NULL,
    PRIMARY KEY (topic_id, key)
) STRICT, WITHOUT ROWID;

CREATE INDEX dedup_keys_by_expiry ON dedup_keys (expires_at);
CREATE INDEX dedup_keys_by_age ON dedup_keys (published_at);

ALTER TABLE stats_hourly ADD COLUMN deduplicated INTEGER NOT NULL DEFAULT 0;
"#;
