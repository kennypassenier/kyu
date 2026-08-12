//! Persistence (AR3): the schema, its forward-only migrations and every
//! SQL statement in the project.
//!
//! All state — messages, subscriptions, delivery rows, dead letters —
//! lives in one SQLite database so each delivery transition is a single
//! transaction. Pragmas (WAL, `synchronous=FULL`, foreign keys, busy
//! timeout) are set explicitly at every open rather than inherited from
//! build defaults, because those defaults vary between builds and the
//! K12 durability promise depends on them.
//!
//! Built in L1.
