#![cfg(feature = "hosted-bootstrap")]

use std::{fs, os::unix::fs::PermissionsExt, process::Command};

#[test]
fn missing_chain_credentials_exit_without_local_fallback_or_key_disclosure() {
    let dir = tempfile::Builder::new()
        .prefix("cc")
        .tempdir_in("/tmp")
        .unwrap();
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let credentials = dir.path().join("credentials");
    fs::create_dir(&credentials).unwrap();
    let worker = dir.path().join("worker");
    let api_key = dir.path().join("api.key");
    let secret = "test-only-never-print-this-key";
    fs::write(&api_key, secret).unwrap();
    fs::set_permissions(&api_key, fs::Permissions::from_mode(0o600)).unwrap();
    let config = dir.path().join("config.json");
    fs::write(&config, serde_json::to_vec(&serde_json::json!({
        "worker_dir":worker, "cbfs_state_dir":credentials,
        "rpc_url":"http://127.0.0.1:1", "trusted_checkpoint_file":dir.path().join("checkpoint.bin"),
        "owner_address":"01".repeat(20), "chain_instance_id":"02".repeat(32),
        "stream_id":"03".repeat(32), "provider_address":"04".repeat(20),
        "admin_key_file":dir.path().join("admin.seed"),
        "broker_url":"wss://broker.example/ws", "broker_pin":null,
        "archive_volume":"archive", "control_volume":"control", "api_key_file":api_key,
        "http_addr":"127.0.0.1:19440", "http_origins":[], "session_seconds":600
    })).unwrap()).unwrap();
    for command in ["hosted-serve", "hosted-init"] {
        let mut process = Command::new(env!("CARGO_BIN_EXE_cowchat-server"));
        process.arg(command).arg("--config").arg(&config);
        if command == "hosted-serve" {
            process.args(["--expected-epoch", "0"]);
        } else {
            process.args(["--reserve-wei", "1"]);
        }
        let output = process.output().unwrap();
        assert!(!output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!stdout.contains(secret) && !stderr.contains(secret));
        assert!(!worker.join("auth.sqlite").exists());
        assert!(!worker.join("intents.sqlite").exists());
        assert!(!worker.join("server.sock").exists());
        // Local preflight did run; failure is in authenticated bootstrap.
        assert!(worker.join("worker.lock").exists());
    }
}
