//! COWCHAT_URL through the real binary: it names the server when the command
//! line names no connection, and an explicit --socket or --tcp wins over it.
//! Nothing listens at either address; the error says which one was tried.

use std::process::Command;

fn status(args: &[&str], env_url: &str, home: &std::path::Path) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_cowchat"))
        .args(args)
        .arg("status")
        .env("HOME", home)
        .env("COWCHAT_URL", env_url)
        .output()
        .unwrap();
    assert!(!out.status.success());
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn cowchat_url_is_the_server_unless_a_connection_is_named() {
    let home = tempfile::TempDir::new().unwrap();
    let url = "ws://127.0.0.1:1/ws";
    // no connection named: the WebSocket from COWCHAT_URL is tried
    let e = status(&[], url, home.path());
    assert!(e.contains("Ws("), "{e}");
    // an explicit socket or TCP address is used instead
    let sock = home.path().join("none.sock");
    assert!(!status(&["--socket", sock.to_str().unwrap()], url, home.path()).contains("Ws("));
    assert!(!status(&["--tcp", "127.0.0.1:1"], url, home.path()).contains("Ws("));
    // set but empty: the default socket, not a WebSocket
    assert!(!status(&[], "", home.path()).contains("Ws("));
}
