use super::*;
use tempfile::tempdir;

fn vault_id(value: &str) -> VaultId {
    value.parse().expect("valid test Vault ID")
}

/// The whole point of the file: a Git turn recorded by one process is
/// readable by the next one.
#[test]
fn a_recorded_git_turn_is_readable_by_a_later_store_over_the_same_file() {
    let directory = tempdir().expect("temporary state directory");
    let vault = vault_id("00000000-0000-4000-8000-000000000001");
    let completed_at = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_787_000_000);

    let writer = VaultRuntimeStateStore::new(directory.path().join("state/vault-runtime.json"));
    writer
        .record_git_turn(
            vault,
            GitTurnRecord {
                completed_at,
                outcome: GitTurnOutcome::UpToDate,
                code: None,
            },
        )
        .expect("record the turn");

    // A second store over the same path stands in for the next process.
    let reader = VaultRuntimeStateStore::new(directory.path().join("state/vault-runtime.json"));
    assert_eq!(
        reader.last_git_turn(vault),
        Some(GitTurnRecord {
            completed_at,
            outcome: GitTurnOutcome::UpToDate,
            code: None,
        })
    );
}

/// A file written by a newer Hatchdoor is not ours to interpret or replace:
/// reading it reports no record (so the Vault simply becomes due now), and
/// writing must leave it exactly as found. Mirrors the registry's
/// `FutureSchema` recovery posture, minus the fail-closed part — a schedule
/// we cannot read costs one extra Git turn, so it degrades instead of
/// refusing to start.
#[test]
fn a_future_schema_file_is_read_as_unknown_and_never_overwritten() {
    let directory = tempdir().expect("temporary state directory");
    let path = directory.path().join("vault-runtime.json");
    let vault = vault_id("00000000-0000-4000-8000-000000000001");
    // Carries a record for this very Vault, so reading `None` below can only
    // come from the schema check — not from an empty file.
    let from_the_future = format!(
        r#"{{"schema_version":{},"vaults":{{"{vault}":{{"git":{{"completed_at":"2026-08-30T11:47:54Z","outcome":"up_to_date"}}}}}},"something_we_do_not_know":true}}"#,
        RUNTIME_STATE_SCHEMA_VERSION + 1
    );
    std::fs::write(&path, &from_the_future).expect("write a newer file");

    let store = VaultRuntimeStateStore::new(&path);
    assert_eq!(
        store.last_git_turn(vault),
        None,
        "a schema this build does not understand must read as no record"
    );

    let refused = store.record_git_turn(
        vault,
        GitTurnRecord {
            completed_at: SystemTime::UNIX_EPOCH,
            outcome: GitTurnOutcome::UpToDate,
            code: None,
        },
    );

    assert!(
        refused.is_err(),
        "writing over a newer file must be refused"
    );
    assert_eq!(
        std::fs::read_to_string(&path).expect("file still there"),
        from_the_future,
        "the newer file must survive byte for byte"
    );
}
