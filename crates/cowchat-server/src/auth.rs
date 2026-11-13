use rand::Rng;
use std::fs;
use std::io::Write;
use std::path::Path;

const KEY_LENGTH: usize = 32;

/// Generate a random API key as a hex string.
pub fn generate_key() -> String {
    let mut rng = rand::thread_rng();
    let bytes: Vec<u8> = (0..KEY_LENGTH).map(|_| rng.gen()).collect();
    hex::encode(bytes)
}

/// Generate a room-invite token: recognizable prefix + the same entropy as an
/// API key. The raw token is shown once; only its hash is persisted.
pub fn generate_invite_token() -> String {
    format!("cinv_{}", generate_key())
}

/// SHA-256 hex of an invite token — the only form the server stores.
pub fn hash_invite_token(token: &str) -> String {
    sha256_hex(token.as_bytes())
}

/// SHA-256 hex of arbitrary bytes (blob attachment integrity records).
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(bytes).to_vec())
}

/// Encode bytes as hex string (simple implementation to avoid a dependency).
mod hex {
    pub fn encode(bytes: Vec<u8>) -> String {
        bytes.iter().map(|b| format!("{:02x}", b)).collect()
    }
}

/// Load or create the API key at the given path.
pub fn load_or_create_key(path: &Path) -> std::io::Result<String> {
    harden_parent_directory(path)?;
    if path.exists() {
        harden_file_permissions(path)?;
        let key = fs::read_to_string(path)?.trim().to_string();
        if !key.is_empty() {
            return Ok(key);
        }
        // An empty file holds no key, so replacing it loses nothing.
        let key = generate_key();
        write_owner_only_atomic(path, &key, Publish::Replace)?;
        return Ok(key);
    }
    create_key_if_absent(path)
}

/// Publish a fresh key at a path just seen to be absent, without replacing
/// anything that appeared there since: `cowchat invites redeem` may have
/// stored a minted key at the same default path in the meantime, and that key
/// is the only copy. On a conflict the key already there is adopted.
fn create_key_if_absent(path: &Path) -> std::io::Result<String> {
    let key = generate_key();
    match write_owner_only_atomic(path, &key, Publish::NoReplace) {
        Ok(()) => Ok(key),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            harden_file_permissions(path)?;
            let existing = fs::read_to_string(path)?.trim().to_string();
            if existing.is_empty() {
                write_owner_only_atomic(path, &key, Publish::Replace)?;
                Ok(key)
            } else {
                Ok(existing)
            }
        }
        Err(e) => Err(e),
    }
}

/// How `write_owner_only_atomic` puts the finished file in place.
#[derive(Clone, Copy)]
enum Publish {
    /// Rename over whatever is there (rotation, or an empty file).
    Replace,
    /// Hard-link into place, failing with `AlreadyExists` if the path exists.
    NoReplace,
}

/// Rotate the key: generate a new one and overwrite the file.
pub fn rotate_key(path: &Path) -> std::io::Result<String> {
    let key = generate_key();
    harden_parent_directory(path)?;
    write_owner_only_atomic(path, &key, Publish::Replace)?;
    Ok(key)
}

fn write_owner_only_atomic(path: &Path, contents: &str, publish: Publish) -> std::io::Result<()> {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("auth.key");
    let temporary = path.with_file_name(format!(
        ".{file_name}.{}.{}.tmp",
        std::process::id(),
        &generate_key()[..8]
    ));
    let result = (|| {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(contents.as_bytes())?;
        file.sync_all()?;
        harden_file_permissions(&temporary)?;
        match publish {
            Publish::Replace => fs::rename(&temporary, path)?,
            Publish::NoReplace => fs::hard_link(&temporary, path)?,
        }
        harden_file_permissions(path)
    })();
    if result.is_err() || matches!(publish, Publish::NoReplace) {
        let _ = fs::remove_file(&temporary);
    }
    result
}

pub(crate) fn harden_file_permissions(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

pub(crate) fn harden_parent_directory(path: &Path) -> std::io::Result<()> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true).mode(0o700);
        builder.create(parent)?;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(not(unix))]
    fs::create_dir_all(parent)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn mode(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    #[cfg(unix)]
    fn key_create_rotate_and_existing_repair_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.key");
        let first = load_or_create_key(&path).unwrap();
        assert_eq!(mode(dir.path()), 0o700);
        assert_eq!(mode(&path), 0o600);
        let second = rotate_key(&path).unwrap();
        assert_ne!(first, second);
        assert_eq!(mode(&path), 0o600);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(load_or_create_key(&path).unwrap(), second);
        assert_eq!(mode(&path), 0o600);

        let repair_parent = dir.path().join("existing-custom-dir");
        fs::create_dir(&repair_parent).unwrap();
        fs::set_permissions(&repair_parent, fs::Permissions::from_mode(0o755)).unwrap();
        let existing_key = repair_parent.join("auth.key");
        fs::write(&existing_key, "existing-secret").unwrap();
        fs::set_permissions(&existing_key, fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(
            load_or_create_key(&existing_key).unwrap(),
            "existing-secret"
        );
        assert_eq!(mode(&repair_parent), 0o700);
        assert_eq!(mode(&existing_key), 0o600);
    }

    /// A server that saw no auth.key, then lost the race to a CLI that stored a
    /// minted key there, must adopt that key rather than replace its only copy.
    #[test]
    fn a_key_that_appears_before_publication_is_adopted_not_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.key");
        fs::write(&path, "minted-by-the-cli\n").unwrap();

        assert_eq!(create_key_if_absent(&path).unwrap(), "minted-by-the-cli");
        assert_eq!(fs::read_to_string(&path).unwrap(), "minted-by-the-cli\n");
        let leftovers: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name() != "auth.key")
            .collect();
        assert!(
            leftovers.is_empty(),
            "the server's temporary must be removed"
        );
    }

    #[test]
    fn a_key_is_created_when_none_appears() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.key");
        let key = load_or_create_key(&path).unwrap();
        assert_eq!(key.len(), 64);
        assert_eq!(fs::read_to_string(&path).unwrap(), key);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}
