//! Environment names kyu no longer reads, refused rather than ignored.

use std::collections::BTreeMap;

/// `KYU_DATA_DIR`, the 2.x name of the state root, is retired in 4.0.0:
/// kyu reads only `KYU_STATE_DIR`. A set `KYU_DATA_DIR` is refused rather
/// than ignored. Ignored, an environment file that still carried it would
/// start the hub on an empty store in the default directory, with every
/// topic and app token left behind in the old one — fix-state-1's fault
/// (CT 109, 2026-09-10) reached by another road (standing rules 12 and 45).
pub fn refuse_retired_data_dir(env: &BTreeMap<String, String>) -> Result<(), String> {
    match env.get("KYU_DATA_DIR") {
        None => Ok(()),
        Some(data) => Err(format!(
            "KYU_DATA_DIR ({data}) is the 2.x name of the state directory and is no \
             longer read since kyu 4.0.0, so starting would open whatever store \
             KYU_STATE_DIR points at instead. Rename it to KYU_STATE_DIR in the \
             environment file. If the store lives in {data} and KYU_STATE_DIR names \
             another directory, move kyu.db, kyu.db-wal and kyu.db-shm together \
             (all three, or the newest writes are lost)."
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn the_retired_2x_name_refuses_to_start_and_names_its_successor() {
        // 4.0 reads only KYU_STATE_DIR. An environment file that still says
        // KYU_DATA_DIR would otherwise start the hub on an empty store in the
        // default directory: fix-state-1's fault, reached by another road.
        for pairs in [
            &[("KYU_DATA_DIR", "/appdata/kyu/kyu-config/data")][..],
            &[
                ("KYU_STATE_DIR", "/appdata/kyu/kyu-config"),
                ("KYU_DATA_DIR", "/appdata/kyu/kyu-config"),
            ][..],
        ] {
            let error =
                refuse_retired_data_dir(&env(pairs)).expect_err("the 2.x name must not start");
            assert!(
                error.contains("KYU_STATE_DIR"),
                "names the successor: {error}"
            );
            assert!(
                error.contains("kyu.db-wal"),
                "the remedy moves all three files: {error}"
            );
        }
    }

    #[test]
    fn the_3x_name_alone_starts() {
        assert!(refuse_retired_data_dir(&env(&[("KYU_STATE_DIR", "/var/lib/kyu")])).is_ok());
        assert!(refuse_retired_data_dir(&env(&[])).is_ok());
    }
}
