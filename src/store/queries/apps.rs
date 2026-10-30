//! Registered apps (sealed credentials): creation, listing, lookup and revocation.
//!
//! Part of the SQL layer (AR1, AR3); statements are parameterized (AR11).
use anyhow::{Context, Result};
use rusqlite::{OptionalExtension, Transaction};

use crate::engine::clock::Millis;

/// W2 · a registered app, as the store holds it. `token` is ciphertext;
/// turning it back into a token is `crypto::SecretKey::open`, and nothing
/// in this module ever does it — the store has no key and should not.
#[derive(Debug, Clone)]
pub struct StoredApp {
    pub id: i64,
    pub name: String,
    pub token: Vec<u8>,
    pub created_at: Millis,
    pub revoked_at: Option<Millis>,
}

fn app_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredApp> {
    Ok(StoredApp {
        id: row.get(0)?,
        name: row.get(1)?,
        token: row.get(2)?,
        created_at: row.get(3)?,
        revoked_at: row.get(4)?,
    })
}

/// Registers an app with an already-encrypted token.
///
/// Fails when a live app of that name exists, which the partial unique
/// index enforces rather than a check-then-insert that two dashboard tabs
/// could race past.
pub fn create_app(tx: &Transaction, name: &str, sealed: &[u8], now: Millis) -> Result<i64> {
    tx.execute(
        "INSERT INTO apps (name, token, created_at, revoked_at) VALUES (?1, ?2, ?3, NULL)",
        rusqlite::params![name, sealed, now],
    )
    .with_context(|| format!("cannot register app {name:?}"))?;
    Ok(tx.last_insert_rowid())
}

/// Every app, revoked ones last, so the page reads as "what is live" first.
pub fn list_apps(conn: &rusqlite::Connection) -> Result<Vec<StoredApp>> {
    let mut statement = conn
        .prepare(
            "SELECT id, name, token, created_at, revoked_at FROM apps \
             ORDER BY revoked_at IS NOT NULL, name",
        )
        .context("cannot list apps")?;
    let apps = statement
        .query_map([], app_from_row)
        .context("cannot list apps")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("cannot read the app list")?;
    Ok(apps)
}

/// The live apps only — what the auth check walks.
pub fn live_apps(conn: &rusqlite::Connection) -> Result<Vec<StoredApp>> {
    let mut statement = conn
        .prepare(
            "SELECT id, name, token, created_at, revoked_at FROM apps \
             WHERE revoked_at IS NULL ORDER BY name",
        )
        .context("cannot list live apps")?;
    let apps = statement
        .query_map([], app_from_row)
        .context("cannot list live apps")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("cannot read the live app list")?;
    Ok(apps)
}

pub fn live_app_by_name(conn: &rusqlite::Connection, name: &str) -> Result<Option<StoredApp>> {
    conn.query_row(
        "SELECT id, name, token, created_at, revoked_at FROM apps \
         WHERE name = ?1 AND revoked_at IS NULL",
        [name],
        app_from_row,
    )
    .optional()
    .with_context(|| format!("cannot look up app {name:?}"))
}

/// Turns an app off without forgetting it existed. Returns false when the
/// app was already revoked, so a double submit reports honestly instead of
/// claiming to have revoked something twice.
pub fn revoke_app(tx: &Transaction, name: &str, now: Millis) -> Result<bool> {
    let changed = tx
        .execute(
            "UPDATE apps SET revoked_at = ?2 WHERE name = ?1 AND revoked_at IS NULL",
            rusqlite::params![name, now],
        )
        .with_context(|| format!("cannot revoke app {name:?}"))?;
    Ok(changed == 1)
}
