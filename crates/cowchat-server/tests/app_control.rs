#![cfg(unix)]

use std::{
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

fn spawn_server(temporary: &tempfile::TempDir, standard_input: Stdio) -> (ChildGuard, PathBuf) {
    let socket = temporary.path().join("cowchat.sock");
    let database = temporary.path().join("cowchat.db");
    let key = temporary.path().join("auth.key");

    let child = Command::new(env!("CARGO_BIN_EXE_cowchat-server"))
        .args(["serve", "--app-control-stdin", "--socket"])
        .arg(&socket)
        .args(["--tcp", "127.0.0.1:0", "--db"])
        .arg(&database)
        .arg("--key-file")
        .arg(&key)
        .stdin(standard_input)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .env("RUST_LOG", "error")
        .spawn()
        .expect("real cowchat-server should launch");

    (ChildGuard(child), socket)
}

fn wait_for_ready(child: &mut Child, socket: &Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        assert!(
            child
                .try_wait()
                .expect("server status should be readable")
                .is_none(),
            "server exited before its Unix listener became ready"
        );
        if UnixStream::connect(socket).is_ok() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "server did not become ready before the deadline"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn wait_for_exit(child: &mut Child) -> ExitStatus {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(status) = child.try_wait().expect("server status should be readable") {
            return status;
        }
        assert!(
            Instant::now() < deadline,
            "server remained alive after its owning control channel reached EOF"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn held_app_control_stdin_keeps_server_alive_then_eof_cleans_up() {
    let temporary = tempfile::tempdir().expect("temporary server directory should be created");
    let (mut child, socket) = spawn_server(&temporary, Stdio::piped());
    wait_for_ready(&mut child.0, &socket);

    // Keeping the inherited writer open proves the private control reader does
    // not affect ordinary server lifetime.
    thread::sleep(Duration::from_millis(100));
    assert!(
        child
            .0
            .try_wait()
            .expect("server status should be readable")
            .is_none(),
        "server must remain alive while its app-control stdin is owned"
    );

    drop(
        child
            .0
            .stdin
            .take()
            .expect("app-control stdin should be piped"),
    );

    let status = wait_for_exit(&mut child.0);
    assert!(status.success(), "server should exit cleanly, got {status}");
    assert!(!socket.exists(), "owned Unix socket should be removed");
}

#[test]
fn pre_ready_app_control_eof_exits_cleanly_and_removes_socket() {
    let temporary = tempfile::tempdir().expect("temporary server directory should be created");
    let (mut child, socket) = spawn_server(&temporary, Stdio::null());

    let status = wait_for_exit(&mut child.0);
    assert!(
        status.success(),
        "pre-ready EOF should use clean async shutdown, got {status}"
    );
    assert!(!socket.exists(), "owned Unix socket should be removed");
}

#[test]
fn key_rotation_refuses_a_live_database_without_touching_the_key() {
    let temporary = tempfile::tempdir().expect("temporary server directory should be created");
    let (mut child, socket) = spawn_server(&temporary, Stdio::piped());
    wait_for_ready(&mut child.0, &socket);
    let database = temporary.path().join("cowchat.db");
    let key_file = temporary.path().join("auth.key");
    let deadline = Instant::now() + Duration::from_secs(3);
    while !key_file.exists() {
        assert!(
            Instant::now() < deadline,
            "server did not create its key before the deadline"
        );
        thread::sleep(Duration::from_millis(10));
    }
    let original_key = std::fs::read_to_string(&key_file).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_cowchat-server"))
        .args(["auth", "rotate-key", "--key-file"])
        .arg(&key_file)
        .arg("--db")
        .arg(&database)
        .output()
        .expect("rotation command should launch");
    assert!(
        !output.status.success(),
        "rotation must fail while the server owns the database lock"
    );
    assert_eq!(
        std::fs::read_to_string(&key_file).unwrap(),
        original_key,
        "a rejected rotation must not modify the credential"
    );

    drop(child.0.stdin.take());
    assert!(wait_for_exit(&mut child.0).success());
}
