use cowchat_server::migrate::{migrate_legacy_data_dir, MigrationOutcome};
use std::fs;
use std::os::unix::net::UnixListener;

/// WAL-mode db named clawchat.db with one committed row, db+wal+shm copied
/// while the connection is still open — the files a crash (or plain WAL
/// operation) leaves behind, committed data in -wal only.
fn make_crashed_wal_db(dir: &std::path::Path) {
    fs::create_dir_all(dir).unwrap();
    let live = dir.join("live.db");
    let conn = rusqlite::Connection::open(&live).unwrap();
    conn.pragma_update(None, "journal_mode", "WAL").unwrap();
    conn.pragma_update(None, "wal_autocheckpoint", 0).unwrap();
    conn.execute("CREATE TABLE t (v TEXT)", []).unwrap();
    conn.execute("INSERT INTO t (v) VALUES ('survives')", [])
        .unwrap();
    for ext in ["", "-wal", "-shm"] {
        let src = dir.join(format!("live.db{ext}"));
        if src.exists() {
            fs::copy(&src, dir.join(format!("clawchat.db{ext}"))).unwrap();
        }
    }
    drop(conn);
    for ext in ["", "-wal", "-shm"] {
        let _ = fs::remove_file(dir.join(format!("live.db{ext}")));
    }
}

#[test]
fn migrates_wal_db_set_and_preserves_committed_data() {
    let tmp = tempfile::tempdir().unwrap();
    let (old, new) = (tmp.path().join(".clawchat"), tmp.path().join(".cowchat"));
    make_crashed_wal_db(&old);
    fs::write(old.join("auth.key"), "secret").unwrap();
    fs::write(old.join("clawchat.sock"), "").unwrap();

    assert!(matches!(
        migrate_legacy_data_dir(&old, &new).unwrap(),
        MigrationOutcome::Migrated
    ));
    assert_eq!(fs::read_to_string(new.join("auth.key")).unwrap(), "secret");
    assert!(
        new.join("cowchat.db-wal").exists(),
        "WAL sidecar must move with the db"
    );
    assert!(!new.join("clawchat.db").exists());
    assert!(
        !new.join("cowchat.sock").exists(),
        "stale socket must not migrate"
    );
    assert!(old.join("MOVED.txt").exists());

    let conn = rusqlite::Connection::open(new.join("cowchat.db")).unwrap();
    let v: String = conn
        .query_row("SELECT v FROM t LIMIT 1", [], |r| r.get(0))
        .unwrap();
    assert_eq!(v, "survives");
}

#[test]
fn resumes_after_interruption_mid_inner_renames() {
    let tmp = tempfile::tempdir().unwrap();
    let (old, new) = (tmp.path().join(".clawchat"), tmp.path().join(".cowchat"));
    make_crashed_wal_db(&old);
    fs::write(old.join("auth.key"), "secret").unwrap();
    fs::rename(old.join("clawchat.db"), old.join("cowchat.db")).unwrap();

    assert!(matches!(
        migrate_legacy_data_dir(&old, &new).unwrap(),
        MigrationOutcome::Migrated
    ));
    assert!(
        new.join("cowchat.db-wal").exists(),
        "sidecar renamed on resume"
    );
    let conn = rusqlite::Connection::open(new.join("cowchat.db")).unwrap();
    let v: String = conn
        .query_row("SELECT v FROM t LIMIT 1", [], |r| r.get(0))
        .unwrap();
    assert_eq!(v, "survives");
}

#[test]
fn errors_on_ambiguous_half_renamed_db_pair() {
    let tmp = tempfile::tempdir().unwrap();
    let (old, new) = (tmp.path().join(".clawchat"), tmp.path().join(".cowchat"));
    fs::create_dir_all(&old).unwrap();
    fs::write(old.join("clawchat.db"), "old-bytes").unwrap();
    fs::write(old.join("cowchat.db"), "other-bytes").unwrap();

    assert!(migrate_legacy_data_dir(&old, &new).is_err());
    assert_eq!(
        fs::read_to_string(old.join("clawchat.db")).unwrap(),
        "old-bytes"
    );
    assert_eq!(
        fs::read_to_string(old.join("cowchat.db")).unwrap(),
        "other-bytes"
    );
    assert!(!new.exists());
}

#[test]
fn errors_when_marker_only_but_new_dir_missing() {
    let tmp = tempfile::tempdir().unwrap();
    let (old, new) = (tmp.path().join(".clawchat"), tmp.path().join(".cowchat"));
    fs::create_dir_all(&old).unwrap();
    fs::write(old.join("MOVED.txt"), "moved").unwrap();
    assert!(migrate_legacy_data_dir(&old, &new).is_err());
}

#[test]
fn errors_when_old_path_is_a_regular_file() {
    let tmp = tempfile::tempdir().unwrap();
    let (old, new) = (tmp.path().join(".clawchat"), tmp.path().join(".cowchat"));
    fs::write(&old, "not a directory").unwrap();
    assert!(migrate_legacy_data_dir(&old, &new).is_err());
    assert!(!new.exists());
}

#[test]
fn errors_when_old_path_is_a_dangling_symlink() {
    let tmp = tempfile::tempdir().unwrap();
    let (old, new) = (tmp.path().join(".clawchat"), tmp.path().join(".cowchat"));
    std::os::unix::fs::symlink(tmp.path().join("gone"), &old).unwrap();
    assert!(migrate_legacy_data_dir(&old, &new).is_err());
    assert!(!new.exists());
}

#[test]
fn errors_when_new_dir_is_a_dangling_symlink() {
    let tmp = tempfile::tempdir().unwrap();
    let (old, new) = (tmp.path().join(".clawchat"), tmp.path().join(".cowchat"));
    fs::create_dir_all(&old).unwrap();
    fs::write(old.join("auth.key"), "secret").unwrap();
    std::os::unix::fs::symlink(tmp.path().join("gone"), &new).unwrap();

    assert!(migrate_legacy_data_dir(&old, &new).is_err());
    assert!(old.join("auth.key").exists(), "nothing moved");
}

#[test]
fn errors_when_target_db_name_is_a_dangling_symlink() {
    let tmp = tempfile::tempdir().unwrap();
    let (old, new) = (tmp.path().join(".clawchat"), tmp.path().join(".cowchat"));
    fs::create_dir_all(&old).unwrap();
    fs::write(old.join("clawchat.db"), "real-bytes").unwrap();
    std::os::unix::fs::symlink(old.join("gone"), old.join("cowchat.db")).unwrap();

    assert!(migrate_legacy_data_dir(&old, &new).is_err());
    assert_eq!(
        fs::read_to_string(old.join("clawchat.db")).unwrap(),
        "real-bytes"
    );
    assert!(!new.exists());
}

#[test]
fn errors_on_sidecars_without_main_db() {
    let tmp = tempfile::tempdir().unwrap();
    let (old, new) = (tmp.path().join(".clawchat"), tmp.path().join(".cowchat"));
    fs::create_dir_all(&old).unwrap();
    fs::write(old.join("clawchat.db-wal"), "wal-bytes").unwrap();
    fs::write(old.join("clawchat.db-shm"), "shm-bytes").unwrap();

    assert!(migrate_legacy_data_dir(&old, &new).is_err());
    assert!(old.join("clawchat.db-wal").exists(), "nothing moved");
    assert!(!new.exists());
}

#[test]
fn refuses_when_legacy_server_is_listening() {
    let tmp = tempfile::tempdir().unwrap();
    let (old, new) = (tmp.path().join(".clawchat"), tmp.path().join(".cowchat"));
    fs::create_dir_all(&old).unwrap();
    fs::write(old.join("auth.key"), "secret").unwrap();
    let _listener = UnixListener::bind(old.join("clawchat.sock")).unwrap();

    let err = migrate_legacy_data_dir(&old, &new).unwrap_err();
    assert!(err.to_string().contains("still running"));
    assert!(old.join("auth.key").exists(), "nothing moved");
    assert!(!new.exists());
}

#[test]
fn errors_on_conflicting_populated_dirs() {
    let tmp = tempfile::tempdir().unwrap();
    let (old, new) = (tmp.path().join(".clawchat"), tmp.path().join(".cowchat"));
    fs::create_dir_all(&old).unwrap();
    fs::write(old.join("auth.key"), "old").unwrap();
    fs::create_dir_all(&new).unwrap();
    fs::write(new.join("auth.key"), "new").unwrap();

    assert!(migrate_legacy_data_dir(&old, &new).is_err());
    assert_eq!(fs::read_to_string(old.join("auth.key")).unwrap(), "old");
    assert_eq!(fs::read_to_string(new.join("auth.key")).unwrap(), "new");
}

#[test]
fn no_op_for_marker_only_or_missing_old_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let (old, new) = (tmp.path().join(".clawchat"), tmp.path().join(".cowchat"));
    assert!(matches!(
        migrate_legacy_data_dir(&old, &new).unwrap(),
        MigrationOutcome::NothingToDo
    ));
    assert!(!new.exists());

    fs::create_dir_all(&old).unwrap();
    fs::write(old.join("MOVED.txt"), "moved").unwrap();
    fs::create_dir_all(&new).unwrap();
    assert!(matches!(
        migrate_legacy_data_dir(&old, &new).unwrap(),
        MigrationOutcome::NothingToDo
    ));
}
