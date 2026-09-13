use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-env-changed=COWCHAT_RELEASE_CHECKPOINT_PATH");
    println!("cargo:rerun-if-env-changed=COWCHAT_RELEASE_CHECKPOINT_SHA256");
    let source = match (
        env::var("COWCHAT_RELEASE_CHECKPOINT_PATH").ok(),
        env::var("COWCHAT_RELEASE_CHECKPOINT_SHA256").ok(),
    ) {
        (None, None) => {
            "pub const CHECKPOINT: &[u8] = &[]; pub const DIGEST: [u8;32] = [0;32];".to_owned()
        }
        (Some(path), Some(digest)) => {
            let path = fs::canonicalize(path).expect("release checkpoint path");
            println!("cargo:rerun-if-changed={}", path.display());
            assert!(
                digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit()),
                "checkpoint digest must be 32-byte hex"
            );
            let digest: Vec<u8> = (0..64)
                .step_by(2)
                .map(|i| u8::from_str_radix(&digest[i..i + 2], 16).unwrap())
                .collect();
            let bytes = fs::read(path).expect("read release checkpoint");
            assert!(
                !bytes.is_empty() && bytes.len() <= 65536,
                "invalid checkpoint size"
            );
            format!(
                "pub const CHECKPOINT: &[u8] = &{bytes:?}; pub const DIGEST: [u8;32] = {digest:?};"
            )
        }
        _ => panic!("checkpoint path and SHA256 must both be fixed by the release build"),
    };
    fs::write(
        PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("actor_checkpoint.rs"),
        source,
    )
    .unwrap();
}
