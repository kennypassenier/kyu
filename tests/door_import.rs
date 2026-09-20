//! [W2, fix-state-1 follow-up] The 2.x `apps` table is merged into the
//! kit's client store at every start, by name.
//!
//! On CT 109 the restored store (2026-09-18) brought eight apps back while
//! the door — `clients.json.enc`, created a week earlier by the first
//! `chassis clients issue` against an empty store — held two. The one-time
//! import skipped because the file existed, and six services stayed locked
//! out until a hand-run tool adopted them (2026-09-20). These tests pin the
//! three lessons that tool paid for: match on NAME (clients the kit issues
//! carry UUID ids, not `app-<name>`), a revoked row must not block
//! re-adoption, and a second start adds nothing.

use std::sync::Arc;

use chassis::core::clients::Client;
use chassis::core::crypto::Key;
use chassis::shell::store::{ClientStore, EncryptedFile, FileClientStore};
use kyu::crypto::SecretKey;
use kyu::engine::Engine;
use kyu::engine::clock::MockClock;
use kyu::kit::import_app_tokens;
use kyu::store::Store;

const START: i64 = 1_700_000_000_000;

struct Fixture {
    dir: tempfile::TempDir,
    engine: Engine,
    key: SecretKey,
    hex: String,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().expect("a temp dir");
    let store = Arc::new(Store::open(dir.path()).expect("a store"));
    let engine = Engine::new(store, Arc::new(MockClock::new(START)));
    let hex = SecretKey::generate_hex();
    let key = SecretKey::parse_hex(&hex).expect("a key");
    Fixture {
        dir,
        engine,
        key,
        hex,
    }
}

impl Fixture {
    fn door(&self) -> FileClientStore {
        let kit_key = Key::parse_hex("KYU_SECRET_KEY", &self.hex, &self.hex).expect("kit key");
        FileClientStore::open(EncryptedFile::new(
            self.dir.path().join("clients.json.enc"),
            kit_key,
            "clients",
        ))
        .expect("the client store")
    }

    /// Names of the clients that can authenticate, sorted.
    fn live_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .door()
            .snapshot()
            .clients
            .iter()
            .filter(|c| c.token.is_some())
            .map(|c| c.name.clone())
            .collect();
        names.sort();
        names
    }

    fn import(&self) -> usize {
        import_app_tokens(self.dir.path(), &self.engine, &self.key, &self.hex).expect("the import")
    }
}

#[test]
fn door_import_adds_the_apps_missing_by_name_to_an_existing_client_store() {
    let f = fixture();
    let ha = f.engine.register_app("ha", &f.key).expect("ha");
    f.engine.register_app("sonarr", &f.key).expect("sonarr");

    // The door already exists: kyu-runner issued by the kit itself, and ha
    // adopted earlier — both under UUID ids, the way the kit writes them.
    let door = f.door();
    door.update(&mut |file| {
        file.clients.push(Client::adopted(
            "4797df0d-fa4f-4523-b240-56c04cf41b50".to_string(),
            "kyu-runner".to_string(),
            "runner-token-not-in-the-table".to_string(),
            "2026-09-10T19:07:21Z".to_string(),
        ));
        file.clients.push(Client::adopted(
            "6a0d0c4e-1f2b-4c7d-9e8a-0b1c2d3e4f50".to_string(),
            "ha".to_string(),
            ha.token.clone(),
            "2026-09-20T18:57:12Z".to_string(),
        ));
        Ok(file.clients.last().cloned().expect("pushed"))
    })
    .expect("seed the door");
    drop(door);

    assert_eq!(f.import(), 1, "only sonarr is missing from the door");
    assert_eq!(f.live_names(), vec!["ha", "kyu-runner", "sonarr"]);

    let ha_row = f
        .door()
        .snapshot()
        .clients
        .into_iter()
        .find(|c| c.name == "ha")
        .expect("ha stays");
    assert_eq!(
        ha_row.id, "6a0d0c4e-1f2b-4c7d-9e8a-0b1c2d3e4f50",
        "a client already in the door is left exactly as it was"
    );

    assert_eq!(f.import(), 0, "a second start adds nothing");
    assert_eq!(f.live_names(), vec!["ha", "kyu-runner", "sonarr"]);
}

#[test]
fn door_import_ignores_a_revoked_row_with_the_same_name() {
    let f = fixture();
    let radarr = f.engine.register_app("radarr", &f.key).expect("radarr");

    // radarr was once in the door and revoked there: the row stays as
    // history with no token. The live app in the table must still come in.
    let door = f.door();
    door.update(&mut |file| {
        let mut row = Client::adopted(
            "0f9d8c7b-6a5e-4d3c-2b1a-09f8e7d6c5b4".to_string(),
            "radarr".to_string(),
            "an-old-token".to_string(),
            "2026-09-01T00:00:00Z".to_string(),
        );
        row.token = None;
        row.revoked_at = Some("2026-09-02T00:00:00Z".to_string());
        file.clients.push(row.clone());
        Ok(row)
    })
    .expect("seed the door");
    drop(door);

    assert_eq!(f.import(), 1);
    let live: Vec<_> = f
        .door()
        .snapshot()
        .clients
        .into_iter()
        .filter(|c| c.name == "radarr" && c.token.is_some())
        .collect();
    assert_eq!(live.len(), 1, "exactly one live radarr row");
    assert_eq!(live[0].token.as_deref(), Some(radarr.token.as_str()));
}
