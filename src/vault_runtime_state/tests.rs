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
        })
    );
}

/// A failure's code and message are the reason the record carries more than a
/// timestamp: they are what a restarted instance republishes, so they must
/// survive the round trip verbatim rather than degrading to the generic
/// sentence.
#[test]
fn a_remembered_failure_keeps_its_code_and_message_across_the_file() {
    let directory = tempdir().expect("temporary state directory");
    let vault = vault_id("00000000-0000-4000-8000-000000000001");
    let completed_at = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_787_000_000);
    let path = directory.path().join("vault-runtime.json");

    VaultRuntimeStateStore::new(&path)
        .record_git_turn(
            vault,
            GitTurnRecord {
                completed_at,
                outcome: GitTurnOutcome::Failed {
                    code: "managed_git_authentication_failed".to_string(),
                    message: "The remote rejected the stored credential.".to_string(),
                },
            },
        )
        .expect("record the failure");

    assert_eq!(
        VaultRuntimeStateStore::new(&path).last_git_turn(vault),
        Some(GitTurnRecord {
            completed_at,
            outcome: GitTurnOutcome::Failed {
                code: "managed_git_authentication_failed".to_string(),
                message: "The remote rejected the stored credential.".to_string(),
            },
        })
    );
}

/// A failure record whose detail did not survive still reads as a failure.
/// The fallbacks belong at the file boundary, so nothing above it has to
/// treat a missing code or message as a case of its own.
#[test]
fn a_failure_record_missing_its_detail_falls_back_at_the_file_boundary() {
    let directory = tempdir().expect("temporary state directory");
    let path = directory.path().join("vault-runtime.json");
    let vault = vault_id("00000000-0000-4000-8000-000000000001");
    std::fs::write(
        &path,
        format!(
            r#"{{"schema_version":{RUNTIME_STATE_SCHEMA_VERSION},"vaults":{{"{vault}":{{"completed_at":"2026-08-30T11:47:54Z","outcome":"failed"}}}}}}"#
        ),
    )
    .expect("write a record with no detail");

    assert_eq!(
        VaultRuntimeStateStore::new(&path)
            .last_git_turn(vault)
            .map(|record| record.outcome),
        Some(GitTurnOutcome::Failed {
            code: UNKNOWN_FAILURE_CODE.to_string(),
            message: UNKNOWN_FAILURE_MESSAGE.to_string(),
        })
    );
}

/// Corruption is not a reason to stop scheduling forever. An unparseable file
/// reads as no record — the Vault simply becomes due now — and the next turn
/// replaces it, which is the one case a future-schema file deliberately does
/// *not* share.
#[test]
fn an_unparseable_file_reads_as_no_record_and_is_replaced_by_the_next_turn() {
    let directory = tempdir().expect("temporary state directory");
    let path = directory.path().join("vault-runtime.json");
    let vault = vault_id("00000000-0000-4000-8000-000000000001");
    std::fs::write(&path, b"{ this is not json").expect("write a corrupt file");

    let store = VaultRuntimeStateStore::new(&path);
    assert_eq!(
        store.last_git_turn(vault),
        None,
        "a file this build cannot parse must read as no record"
    );

    let completed_at = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_787_000_000);
    store
        .record_git_turn(
            vault,
            GitTurnRecord {
                completed_at,
                outcome: GitTurnOutcome::Synchronized,
            },
        )
        .expect("a corrupt file is overwritten, not refused");

    assert_eq!(
        VaultRuntimeStateStore::new(&path).last_git_turn(vault),
        Some(GitTurnRecord {
            completed_at,
            outcome: GitTurnOutcome::Synchronized,
        })
    );
}

/// Pruning a disconnected Vault must remove only that Vault. Callers prune
/// unconditionally, so a Vault with nothing stored — and a file with nothing
/// in it at all — must both be no-ops rather than errors.
#[test]
fn forgetting_one_vault_leaves_the_others_and_is_a_no_op_when_nothing_is_stored() {
    let directory = tempdir().expect("temporary state directory");
    let path = directory.path().join("vault-runtime.json");
    let departing = vault_id("00000000-0000-4000-8000-000000000001");
    let staying = vault_id("00000000-0000-4000-8000-000000000002");
    let completed_at = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_787_000_000);
    let store = VaultRuntimeStateStore::new(&path);

    store
        .forget(departing)
        .expect("pruning with no file at all is a no-op");

    for vault in [departing, staying] {
        store
            .record_git_turn(
                vault,
                GitTurnRecord {
                    completed_at,
                    outcome: GitTurnOutcome::UpToDate,
                },
            )
            .expect("record the turn");
    }

    store.forget(departing).expect("prune the departing Vault");

    assert_eq!(
        store.last_git_turn(departing),
        None,
        "a disconnected Vault must not hand its countdown to whatever reconnects under its ID"
    );
    assert!(
        store.last_git_turn(staying).is_some(),
        "pruning one Vault must not disturb another's schedule"
    );

    store
        .forget(departing)
        .expect("pruning a Vault that is already gone is a no-op");
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
        r#"{{"schema_version":{},"vaults":{{"{vault}":{{"completed_at":"2026-08-30T11:47:54Z","outcome":"up_to_date"}}}},"something_we_do_not_know":true}}"#,
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
