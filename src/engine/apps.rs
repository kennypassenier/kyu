//! App management (W2): registered apps and their encrypted tokens.

use crate::crypto::SecretKey;
use crate::store::queries;

use super::clock::Millis;
use super::error::check_name;
use super::{Engine, EngineError, Result};

/// W2 · a registered app as the dashboard shows it, token in the clear.
///
/// This type exists only between decrypting and rendering. It deliberately
/// has no `Debug`, `Serialize` or `Display`: the plaintext-scan test proves
/// tokens stay out of logs and metrics, and the cheapest way to keep that
/// true is to make printing one awkward.
pub struct App {
    pub name: String,
    pub token: String,
    pub created_at: Millis,
    pub revoked_at: Option<Millis>,
}

impl App {
    pub fn is_live(&self) -> bool {
        self.revoked_at.is_none()
    }
}

/// App management (W2). Every one of these needs the encryption key, which
/// the engine does not hold — configuration lives in `Config`, and handing
/// the key to the engine would put it one careless `Debug` away from a log
/// line. Callers pass it in.
impl Engine {
    /// Registers an app and returns it with a freshly generated token.
    ///
    /// The token is generated here rather than accepted from the caller:
    /// a token someone chose is a token someone can guess.
    pub fn register_app(&self, name: &str, key: &SecretKey) -> Result<App> {
        check_name("app", name)?;
        let token = crate::crypto::generate_token();
        let sealed = key.seal(&token).map_err(EngineError::Internal)?;
        let now = self.now_ms();
        let name_owned = name.to_string();

        self.write(move |tx| {
            if queries::live_app_by_name(tx, &name_owned)?.is_some() {
                // Reported as a conflict rather than an internal error, so
                // the dashboard can say "pick another name" instead of
                // "something went wrong".
                return Ok(Err(EngineError::AppExists { name: name_owned }));
            }
            queries::create_app(tx, &name_owned, &sealed, now)?;
            Ok(Ok(()))
        })??;

        Ok(App {
            name: name.to_string(),
            token,
            created_at: now,
            revoked_at: None,
        })
    }

    /// Every app, decrypted for display.
    ///
    /// A token that will not decrypt does not sink the page: the app is
    /// listed with an empty token, because "this app exists but its token is
    /// unreadable" is exactly what someone who changed the key needs to see,
    /// and a 500 would tell them nothing.
    pub fn list_apps(&self, key: &SecretKey) -> Result<Vec<App>> {
        let stored = self
            .store
            .read(queries::list_apps)
            .map_err(EngineError::Internal)?;
        Ok(stored
            .into_iter()
            .map(|app| App {
                name: app.name,
                token: key.open(&app.token).unwrap_or_default(),
                created_at: app.created_at,
                revoked_at: app.revoked_at,
            })
            .collect())
    }

    /// The token of one live app, for rendering a snippet.
    pub fn app_token(&self, name: &str, key: &SecretKey) -> Result<Option<String>> {
        let name_owned = name.to_string();
        let stored = self
            .store
            .read(move |conn| queries::live_app_by_name(conn, &name_owned))
            .map_err(EngineError::Internal)?;
        match stored {
            None => Ok(None),
            Some(app) => Ok(key.open(&app.token).ok()),
        }
    }

    pub fn revoke_app(&self, name: &str) -> Result<()> {
        let name_owned = name.to_string();
        let now = self.now_ms();
        self.write(move |tx| {
            if queries::revoke_app(tx, &name_owned, now)? {
                Ok(Ok(()))
            } else {
                Ok(Err(EngineError::UnknownApp { name: name_owned }))
            }
        })?
    }

    /// Which live app, if any, presented `candidate`.
    ///
    /// Walks every live app and compares in constant time even after a
    /// match, so the time this takes reveals neither which app matched nor
    /// how far a guess got. App counts here are single digits; if that ever
    /// changes, this becomes an indexed lookup on a keyed hash.
    pub fn app_for_token(&self, candidate: &str, key: &SecretKey) -> Result<Option<String>> {
        let apps = self
            .store
            .read(queries::live_apps)
            .map_err(EngineError::Internal)?;
        let mut found: Option<String> = None;
        for app in apps {
            let Ok(token) = key.open(&app.token) else {
                continue;
            };
            if crate::crypto::constant_time_eq(token.as_bytes(), candidate.as_bytes())
                && found.is_none()
            {
                found = Some(app.name);
            }
        }
        Ok(found)
    }
}
