//! Where a minted key is stored, and how later commands find it.
//!
//! `~/.cowchat/auth.key` is the local server's listener key: `cowchat-server`
//! reads it on first start. A key minted by redeeming a remote invite belongs to
//! that remote server only, so it is stored per server and never in that file.

use cowchat_client::CowchatClient;
use cowchat_server::{CowchatServer, ServerConfig};
use std::path::{Path, PathBuf};

fn config(dir: &Path, http_addr: Option<String>, auth_key_path: PathBuf) -> ServerConfig {
    ServerConfig {
        socket_path: dir.join(format!("{}.sock", uuid::Uuid::new_v4())),
        tcp_addr: None,
        http_addr,
        db_path: dir.join(format!("{}.db", uuid::Uuid::new_v4())),
        auth_key_path,
        no_auth: false,
        allow_keyless_local: false,
        allow_private_webhooks: false,
        http_signup_enabled: false,
        http_admin_secret: None,
        http_allowed_origins: vec![],
        trusted_proxy_ips: vec![],
        blob_idle_expiry_seconds: cowchat_server::server::DEFAULT_BLOB_IDLE_EXPIRY_SECS,
        max_rooms: None,
    }
}

/// A remote server with one room and a single-use invite to it, written to a
/// token file. Returns the server task, its URL, the room name and the token file.
async fn remote_with_invite(
    dir: &Path,
    tag: &str,
    key_path: PathBuf,
) -> (tokio::task::JoinHandle<()>, String, String, PathBuf) {
    let server = CowchatServer::new(config(dir, Some("127.0.0.1:0".into()), key_path)).unwrap();
    let url = format!("ws://{}/ws", server.http_addr().unwrap());
    let key = server.api_key().to_owned();
    let task = tokio::spawn(async move { server.run().await.unwrap() });
    let owner = CowchatClient::connect_ws(&url, &key, "Owner", Some("owner"), vec![])
        .await
        .unwrap();
    let room_name = format!("origin-{tag}");
    let room = owner.create_room(&room_name, None, None).await.unwrap();
    let invite = owner.create_invite(&room.room_id, true).await.unwrap();
    let token_file = dir.join(format!("{tag}.tok"));
    std::fs::write(&token_file, format!("{}\n", invite.token)).unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&token_file, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    (task, url, room_name, token_file)
}

async fn cowchat(home: &Path, args: &[&str]) -> std::process::Output {
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_cowchat"));
    command.args(args).env("HOME", home);
    for var in [
        "COWCHAT_URL",
        "COWCHAT_AGENT_ID",
        "COWCHAT_SESSION_FILE",
        "COWCHAT_ROOM_KEY",
        "COWCHAT_INVITE_TOKEN",
        "CODEX_SESSION_ID",
    ] {
        command.env_remove(var);
    }
    command.output().await.unwrap()
}

fn text(output: &std::process::Output) -> String {
    format!(
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// Every regular file under `dir`, recursively.
fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return found;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            found.extend(files_under(&path));
        } else {
            found.push(path);
        }
    }
    found
}

/// Remote redemption, then the local server's first start, then a second
/// server: the minted key stays with the server that minted it.
#[tokio::test]
async fn a_minted_key_is_kept_for_its_server_and_never_becomes_the_listener_key() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let (remote, url, room_name, token_file) =
        remote_with_invite(dir.path(), "a", dir.path().join("remote-a.key")).await;

    let redeemed = cowchat(
        &home,
        &[
            "--url",
            &url,
            "invites",
            "redeem",
            "--token-file",
            token_file.to_str().unwrap(),
        ],
    )
    .await;
    assert!(redeemed.status.success(), "{}", text(&redeemed));
    let listener_key_path = home.join(".cowchat").join("auth.key");
    assert!(
        !listener_key_path.exists(),
        "a minted key must not be stored as the local listener key"
    );
    let stored: Vec<PathBuf> = files_under(&home.join(".cowchat"))
        .into_iter()
        .filter(|path| {
            std::fs::read_to_string(path).is_ok_and(|content| content.trim().len() == 64)
        })
        .collect();
    assert_eq!(stored.len(), 1, "exactly one stored key: {stored:?}");
    let minted = std::fs::read_to_string(&stored[0])
        .unwrap()
        .trim()
        .to_string();
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&stored[0]).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "{}", stored[0].display());
    }
    assert!(
        String::from_utf8_lossy(&redeemed.stdout).contains(&*stored[0].to_string_lossy()),
        "the output names where the key went: {}",
        text(&redeemed)
    );

    // The local server's first start makes its own listener key.
    let local = CowchatServer::new(config(dir.path(), None, listener_key_path.clone())).unwrap();
    let local_key = local.api_key().to_owned();
    drop(local);
    assert_ne!(local_key, minted, "the local server adopted the minted key");

    // Plain commands for the redeeming server use its key, beside the local one.
    let listed = cowchat(&home, &["--url", &url, "rooms", "list"]).await;
    assert!(listed.status.success(), "{}", text(&listed));
    assert!(
        String::from_utf8_lossy(&listed.stdout).contains(&room_name),
        "{}",
        text(&listed)
    );

    // A second server that would accept the minted key never receives it.
    let other_key_path = dir.path().join("remote-b.key");
    std::fs::write(&other_key_path, format!("{minted}\n")).unwrap();
    let other = CowchatServer::new(config(
        dir.path(),
        Some("127.0.0.1:0".into()),
        other_key_path,
    ))
    .unwrap();
    assert_eq!(other.api_key(), minted);
    let other_url = format!("ws://{}/ws", other.http_addr().unwrap());
    let other_task = tokio::spawn(async move { other.run().await.unwrap() });
    // Wait for the second server to accept connections before testing the CLI.
    let mut ready = false;
    for _ in 0..100 {
        if CowchatClient::connect_ws(&other_url, &minted, "probe", Some("probe"), vec![])
            .await
            .is_ok()
        {
            ready = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(ready, "the second server never came up");
    let crossed = cowchat(&home, &["--url", &other_url, "rooms", "list"]).await;
    assert!(
        !crossed.status.success(),
        "the CLI sent one server's minted key to another server: {}",
        text(&crossed)
    );

    remote.abort();
    other_task.abort();
}

/// `redeem --key-file` stores the key elsewhere; `--key-file` on any later
/// command reads it, while the local listener key stays in place.
#[tokio::test]
async fn a_key_file_from_redeem_is_usable_by_ordinary_commands() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let data = home.join(".cowchat");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(data.join("auth.key"), "local-listener-key\n").unwrap();
    let (remote, url, room_name, token_file) =
        remote_with_invite(dir.path(), "kf", dir.path().join("remote-kf.key")).await;
    let key_file = dir.path().join("remote.key");

    let redeemed = cowchat(
        &home,
        &[
            "--url",
            &url,
            "invites",
            "redeem",
            "--token-file",
            token_file.to_str().unwrap(),
            "--key-file",
            key_file.to_str().unwrap(),
        ],
    )
    .await;
    assert!(redeemed.status.success(), "{}", text(&redeemed));
    assert!(
        String::from_utf8_lossy(&redeemed.stdout).contains("--key-file"),
        "the output says how to use the key: {}",
        text(&redeemed)
    );
    assert_eq!(
        std::fs::read_to_string(data.join("auth.key")).unwrap(),
        "local-listener-key\n"
    );

    let listed = cowchat(
        &home,
        &[
            "--url",
            &url,
            "--key-file",
            key_file.to_str().unwrap(),
            "rooms",
            "list",
        ],
    )
    .await;
    assert!(listed.status.success(), "{}", text(&listed));
    assert!(
        String::from_utf8_lossy(&listed.stdout).contains(&room_name),
        "{}",
        text(&listed)
    );

    // Without it, the CLI falls back to the local key, which this server rejects.
    let fallback = cowchat(&home, &["--url", &url, "rooms", "list"]).await;
    assert!(!fallback.status.success(), "{}", text(&fallback));

    remote.abort();
}

/// `--key-file` must name a readable regular file holding a key.
#[tokio::test]
async fn a_missing_or_empty_key_file_is_an_error_before_connecting() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let empty = dir.path().join("empty.key");
    std::fs::write(&empty, "\n").unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("ws://{}/ws", listener.local_addr().unwrap());
    listener.set_nonblocking(true).unwrap();

    for path in [dir.path().join("absent.key"), empty] {
        let out = cowchat(
            &home,
            &[
                "--url",
                &url,
                "--key-file",
                path.to_str().unwrap(),
                "rooms",
                "list",
            ],
        )
        .await;
        assert!(!out.status.success(), "{}", text(&out));
        assert!(
            String::from_utf8_lossy(&out.stderr).contains(&*path.to_string_lossy()),
            "{}",
            text(&out)
        );
        assert!(
            matches!(listener.accept(), Err(e) if e.kind() == std::io::ErrorKind::WouldBlock),
            "the CLI connected with no key"
        );
    }
}
