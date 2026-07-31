use std::fs;
use std::io;
use std::os::unix::net::UnixStream;
use std::path::Path;

#[derive(Debug)]
pub enum MigrationOutcome {
    Migrated,
    NothingToDo,
}

/// Entry-level existence: does any directory entry exist at `p`, including a
/// dangling symlink. Only NotFound is absence; other errors propagate.
fn entry_exists(p: &Path) -> io::Result<bool> {
    match fs::symlink_metadata(p) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

/// Persist renames within a directory before the next ordering-dependent step.
fn fsync_dir(p: &Path) -> io::Result<()> {
    fs::File::open(p)?.sync_all()
}

/// One-time migration of a legacy ClawChat data dir (v0.3.x and earlier).
///
/// Inner renames are idempotent and happen before the atomic directory move,
/// making interrupted migrations resumable. Ambiguous states fail closed.
pub fn migrate_legacy_data_dir(old_dir: &Path, new_dir: &Path) -> io::Result<MigrationOutcome> {
    match fs::symlink_metadata(old_dir) {
        Ok(meta) if meta.is_dir() => {}
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "{} exists but is not a plain directory (file or symlink); refusing to migrate",
                    old_dir.display()
                ),
            ));
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Ok(MigrationOutcome::NothingToDo);
        }
        Err(e) => return Err(e),
    }

    let mut names = Vec::new();
    for entry in fs::read_dir(old_dir)? {
        names.push(entry?.file_name());
    }
    if names.is_empty() {
        return Ok(MigrationOutcome::NothingToDo);
    }

    let new_exists = entry_exists(new_dir)?;
    if names.iter().all(|n| n == "MOVED.txt") {
        return match fs::metadata(new_dir) {
            Ok(m) if m.is_dir() => Ok(MigrationOutcome::NothingToDo),
            Ok(_) => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "{} says data moved to {}, but that path is not a directory; resolve manually",
                    old_dir.join("MOVED.txt").display(),
                    new_dir.display()
                ),
            )),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!(
                    "{} says data moved to {}, but that directory is missing; restore it (or remove the marker dir) and restart",
                    old_dir.join("MOVED.txt").display(),
                    new_dir.display()
                ),
            )),
            Err(e) => Err(e),
        };
    }
    if new_exists {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!(
                "both {} and {} exist and contain data; resolve manually (usually: remove the one you don't want) and restart",
                old_dir.display(),
                new_dir.display()
            ),
        ));
    }

    let legacy_sock = old_dir.join("clawchat.sock");
    if entry_exists(&legacy_sock)? && UnixStream::connect(&legacy_sock).is_ok() {
        return Err(io::Error::new(
            io::ErrorKind::ResourceBusy,
            format!(
                "a clawchat-server is still running against {}; stop it and re-run",
                old_dir.display()
            ),
        ));
    }

    let main_exists =
        entry_exists(&old_dir.join("clawchat.db"))? || entry_exists(&old_dir.join("cowchat.db"))?;
    if !main_exists {
        for base in ["clawchat.db", "cowchat.db"] {
            for ext in ["-wal", "-shm", "-journal"] {
                if entry_exists(&old_dir.join(format!("{base}{ext}")))? {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!(
                            "{} contains SQLite sidecar files but no main database; resolve manually",
                            old_dir.display()
                        ),
                    ));
                }
            }
        }
    }

    for ext in ["", "-wal", "-shm", "-journal"] {
        let from = old_dir.join(format!("clawchat.db{ext}"));
        let to = old_dir.join(format!("cowchat.db{ext}"));
        if entry_exists(&from)? {
            if entry_exists(&to)? {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!(
                        "both {} and {} exist; resolve manually",
                        from.display(),
                        to.display()
                    ),
                ));
            }
            fs::rename(&from, &to)?;
        }
    }
    if entry_exists(&legacy_sock)? {
        fs::remove_file(&legacy_sock)?;
    }

    // Persist every inner rename before the commit-point directory move.
    fsync_dir(old_dir)?;

    // Best-effort re-probe immediately before the commit point. Detection is
    // point-in-time; users must stop legacy servers before migrating.
    if entry_exists(&legacy_sock)? && UnixStream::connect(&legacy_sock).is_ok() {
        return Err(io::Error::new(
            io::ErrorKind::ResourceBusy,
            format!(
                "a clawchat-server is still running against {}; stop it and re-run",
                old_dir.display()
            ),
        ));
    }

    fs::rename(old_dir, new_dir)?;
    if let Some(parent) = new_dir.parent() {
        fsync_dir(parent)?;
    }

    fs::create_dir_all(old_dir)?;
    fs::write(
        old_dir.join("MOVED.txt"),
        "ClawChat is now Cowchat. Your data moved to ~/.cowchat (v0.4.0).\n",
    )?;
    Ok(MigrationOutcome::Migrated)
}
