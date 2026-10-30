//! Adopting app tokens a 2.x hub issued into the kit's client door.

use crate::engine::Engine;

/// K2-1 · import of the 2.x app tokens into the kit's client store — at
/// every start, by name (3.4.0; it was once, and only while
/// `clients.json.enc` did not exist yet).
///
/// Runs before the kit opens the store, so nothing else writes the file
/// meanwhile. Every live app in the 2.x `apps` table whose NAME has no
/// client with a token in the door is added; everything already there is
/// left exactly as it is, and a start that finds nothing missing writes
/// nothing. The tokens are copied unchanged: an app that could publish
/// yesterday can publish today with the same line in its environment file.
///
/// Why by name and why every start (fix-state-1 follow-up, 2026-09-20): on
/// CT 109 the restored store brought eight apps back while the door,
/// created a week earlier by the first `chassis clients issue` against an
/// empty store, held two — and the one-time import skipped because the file
/// existed. Six services stayed at 401 until a hand-run tool adopted them.
/// That tool's first dedup compared ids against `app-<name>` and offered to
/// re-add the two working services, because a client the kit issues itself
/// carries a UUID: the name is the only key both sides share. A revoked row
/// keeps its name with no token, so it does not block re-adoption.
pub fn import_app_tokens(
    state_dir: &std::path::Path,
    engine: &Engine,
    key: &crate::crypto::SecretKey,
    key_hex: &str,
) -> anyhow::Result<usize> {
    use std::collections::BTreeSet;

    use chassis::core::clients::{Client, ClientsFile};
    use chassis::shell::store::{ClientStore, EncryptedFile, FileClientStore};

    let apps: Vec<_> = engine
        .list_apps(key)?
        .into_iter()
        .filter(|app| app.is_live() && !app.token.is_empty())
        .collect();
    if apps.is_empty() {
        return Ok(0);
    }
    let file = state_dir.join("clients.json.enc");
    let kit_key = chassis::core::crypto::Key::parse_hex("KYU_SECRET_KEY", key_hex, key_hex)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let store = FileClientStore::open(EncryptedFile::new(file, kit_key, "clients"))
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let present: BTreeSet<String> = store
        .snapshot()
        .clients
        .iter()
        .filter(|client| client.token.is_some())
        .map(|client| client.name.clone())
        .collect();
    let apps: Vec<_> = apps
        .into_iter()
        .filter(|app| !present.contains(&app.name))
        .collect();
    if apps.is_empty() {
        return Ok(0);
    }
    let now = chassis::shell::time::now_rfc3339();
    let count = apps.len();
    store
        .update(&mut |clients: &mut ClientsFile| {
            for app in &apps {
                // Client is #[non_exhaustive] since chassis-rs 2.0.0: built
                // through Client::adopted rather than field by field, so a
                // field the kit adds later defaults instead of refusing to
                // compile here.
                clients.clients.push(Client::adopted(
                    format!("app-{}", app.name),
                    app.name.clone(),
                    app.token.clone(),
                    now.clone(),
                ));
            }
            Ok(clients.clients.last().cloned().expect("at least one app"))
        })
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(count)
}
