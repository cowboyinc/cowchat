//! Credential hygiene: an invite token must never have to pass through argv, and a
//! redeemed API key must land in a mode-0600 file without being printed.
//!
//! Both properties are required by docs/plans/2026-08-24-product-direction.md
//! (Prerequisite) because remote-setup teaches a discipline the tool must be able
//! to honor.

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

/// The key shape the server family actually mints: hex of a 32-byte value.
const MINTED_KEY: &str = "1e2d3c4b5a69788796a5b4c3d2e1f0091e2d3c4b5a69788796a5b4c3d2e1f009";

fn bin() -> PathBuf {
    let mut p = std::env::current_exe().unwrap();
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("cowchat")
}

fn tmpdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "cowchat-credhyg-{}-{}-{}",
        tag,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[cfg(unix)]
fn mode(path: &std::path::Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path).unwrap().permissions().mode() & 0o777
}

/// Serve exactly one HTTP request with a canned response, returning what was
/// received. Just enough HTTP/1.1 for reqwest: read headers, honor
/// Content-Length, answer, close.
fn serve_one(
    listener: std::net::TcpListener,
    status_line: &'static str,
    body: &'static str,
) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("the CLI should connect");
        let request = read_http_request(&mut stream);
        let response = format!(
            "HTTP/1.1 {status_line}\r\nContent-Type: application/json\r\n\
             Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(response.as_bytes()).unwrap();
        request
    })
}

fn read_http_request(stream: &mut std::net::TcpStream) -> String {
    use std::io::Read;
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .unwrap();
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let n = stream.read(&mut chunk).expect("request read");
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&buf[..end]).to_lowercase();
            let content_length = headers
                .lines()
                .find_map(|l| l.strip_prefix("content-length:"))
                .and_then(|v| v.trim().parse::<usize>().ok())
                .unwrap_or(0);
            if buf.len() >= end + 4 + content_length {
                break;
            }
        }
    }
    String::from_utf8_lossy(&buf).into_owned()
}

/// `invites redeem --token-file` must be accepted, so a 0600 sidecar can be
/// consumed without `redeem "$(cat tok)"` putting the token in argv.
#[test]
fn redeem_accepts_a_token_file() {
    let dir = tmpdir("tokenfile");
    let tok = dir.join("token");
    fs::write(&tok, "cinv_from_file\n").unwrap();

    let out = Command::new(bin())
        .args(["invites", "redeem", "--token-file"])
        .arg(&tok)
        .arg("--dry-run")
        .env_remove("COWCHAT_INVITE_TOKEN")
        .output()
        .expect("run cowchat");

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "--token-file should be accepted.\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        !stdout.contains("cinv_from_file") && !stderr.contains("cinv_from_file"),
        "the token must never be printed — this command confirms delivery of a secret \
         and its output reaches terminals, CI logs and agent transcripts.\nstdout: {stdout}"
    );
    assert!(
        stdout.contains("from file") && stdout.contains("sha256:"),
        "dry-run should name the source and fingerprint the value.\nstdout: {stdout}"
    );
}

/// `--token-file -` reads the token from stdin.
#[test]
fn redeem_accepts_a_token_on_stdin() {
    let mut child = Command::new(bin())
        .args(["invites", "redeem", "--token-file", "-", "--dry-run"])
        .env_remove("COWCHAT_INVITE_TOKEN")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn cowchat");
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"cinv_from_stdin\n")
        .unwrap();
    let out = child.wait_with_output().unwrap();

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "stdin token should be accepted: {stdout}"
    );
    assert!(
        !stdout.contains("cinv_from_stdin"),
        "token leaked: {stdout}"
    );
    assert!(
        stdout.contains("from stdin") && stdout.contains("sha256:"),
        "stdout: {stdout}"
    );
}

/// The env fallback exists so a token need not appear on a command line at all.
#[test]
fn redeem_accepts_a_token_from_the_environment() {
    let out = Command::new(bin())
        .args(["invites", "redeem", "--dry-run"])
        .env("COWCHAT_INVITE_TOKEN", "cinv_from_env")
        .output()
        .expect("run cowchat");

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "env token should be accepted: {stdout}"
    );
    assert!(!stdout.contains("cinv_from_env"), "token leaked: {stdout}");
    assert!(
        stdout.contains("COWCHAT_INVITE_TOKEN") && stdout.contains("sha256:"),
        "stdout: {stdout}"
    );
}

/// Supplying the token two ways is a mistake worth failing on, not silently
/// resolving by precedence.
#[test]
fn redeem_rejects_conflicting_token_sources() {
    let dir = tmpdir("conflict");
    let tok = dir.join("token");
    fs::write(&tok, "cinv_file").unwrap();

    let out = Command::new(bin())
        .args(["invites", "redeem", "cinv_positional", "--token-file"])
        .arg(&tok)
        .arg("--dry-run")
        .env_remove("COWCHAT_INVITE_TOKEN")
        .output()
        .expect("run cowchat");

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "two explicit token sources must be an error, not a silent precedence rule"
    );
    assert!(
        stderr.contains("exactly one way"),
        "the conflict rule must be what fired, not a missing file or a clap error.\n\
         stderr: {stderr}"
    );
}

/// An *ambient* env var must not conflict with an explicit argument. Treating it
/// as a conflict broke `redeem <token>` for anyone who exported the variable —
/// a regression against v0.9.1, and inconsistent with `resolve_agent_id`.
#[test]
fn an_explicit_token_overrides_the_environment() {
    let out = Command::new(bin())
        .args(["invites", "redeem", "cinv_explicit", "--dry-run"])
        .env("COWCHAT_INVITE_TOKEN", "cinv_ambient")
        .output()
        .expect("run cowchat");

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "an explicit argument must win over the environment.\nstdout: {stdout}"
    );
    assert!(stdout.contains("from argument"), "stdout: {stdout}");
}

/// With no token anywhere, the error must name the ways to supply one.
#[test]
fn redeem_without_a_token_explains_the_options() {
    let out = Command::new(bin())
        .args(["invites", "redeem", "--dry-run"])
        .env_remove("COWCHAT_INVITE_TOKEN")
        .output()
        .expect("run cowchat");

    assert!(!out.status.success(), "missing token must fail");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("--token-file") && stderr.contains("COWCHAT_INVITE_TOKEN"),
        "error should name every way to supply a token.\nstderr: {stderr}"
    );
}

/// The keyless flow — `invites redeem --token-file tok --key-file …` against a
/// remote server — must mint over HTTP in-process and land the key 0600 without
/// ever printing it or passing either secret through a shell.
#[cfg(unix)]
#[test]
fn keyless_redeem_stores_the_minted_key_0600_and_never_prints_it() {
    let dir = tmpdir("keyless");
    let tok = dir.join("tok");
    fs::write(&tok, "cinv_keyless_probe\n").unwrap();
    let key_path = dir.join("nested").join("auth.key");

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = serve_one(
        listener,
        "201 Created",
        r#"{"api_key":"1e2d3c4b5a69788796a5b4c3d2e1f0091e2d3c4b5a69788796a5b4c3d2e1f009","room_id":"r1","room_name":"invite-lab","tier":"free"}"#,
    );

    let out = Command::new(bin())
        .args(["--url", &url, "invites", "redeem", "--token-file"])
        .arg(&tok)
        .arg("--key-file")
        .arg(&key_path)
        .env_remove("COWCHAT_INVITE_TOKEN")
        .output()
        .expect("run cowchat");
    let request = server.join().unwrap();

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "keyless redeem failed.\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        request.starts_with("POST /api/invites/redeem ") && request.contains("cinv_keyless_probe"),
        "the CLI must POST the redeem endpoint with the token in the body.\nrequest: {request}"
    );
    assert_eq!(fs::read_to_string(&key_path).unwrap().trim(), MINTED_KEY);
    assert_eq!(mode(&key_path), 0o600, "key file must be 0600");
    assert_eq!(
        mode(key_path.parent().unwrap()),
        0o700,
        "a directory we created must be 0700"
    );
    assert!(
        !stdout.contains(MINTED_KEY) && !stderr.contains(MINTED_KEY),
        "the key must never be printed.\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stdout.contains("sha256:") && stdout.contains("invite-lab"),
        "success should fingerprint the key and name the granted room.\nstdout: {stdout}"
    );
}

/// An occupied destination must be refused BEFORE the server is contacted:
/// single-use invites burn on redemption, so the only free moment to refuse is
/// while the invite is still live. This ordering is what lets the sidecar-rescue
/// machinery stay deleted.
#[test]
fn an_occupied_destination_refuses_before_the_invite_is_spent() {
    let dir = tmpdir("occupied");
    let tok = dir.join("tok");
    fs::write(&tok, "cinv_precious\n").unwrap();
    let key_path = dir.join("auth.key");
    fs::write(&key_path, "k_original\n").unwrap();

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());

    let out = Command::new(bin())
        .args(["--url", &url, "invites", "redeem", "--token-file"])
        .arg(&tok)
        .arg("--key-file")
        .arg(&key_path)
        .env_remove("COWCHAT_INVITE_TOKEN")
        .output()
        .expect("run cowchat");

    assert!(!out.status.success(), "must refuse an occupied destination");
    assert_eq!(
        fs::read_to_string(&key_path).unwrap().trim(),
        "k_original",
        "the existing key must be untouched"
    );
    listener.set_nonblocking(true).unwrap();
    assert!(
        matches!(listener.accept(), Err(e) if e.kind() == std::io::ErrorKind::WouldBlock),
        "the refusal must happen before the server is ever contacted"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("still live"),
        "the error must say the invite survived the refusal.\nstderr: {stderr}"
    );
    assert!(
        !stderr.contains("\\n"),
        "multi-line errors must render as lines, not Debug-escaped text.\nstderr: {stderr}"
    );
}

/// A server refusal must store nothing and leave the destination free for a
/// retry — the invite was not consumed.
#[test]
fn a_server_error_stores_nothing() {
    let dir = tmpdir("serverr");
    let tok = dir.join("tok");
    fs::write(&tok, "cinv_rejected\n").unwrap();
    let key_path = dir.join("auth.key");

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = serve_one(listener, "404 Not Found", r#"{"error":"not found"}"#);

    let out = Command::new(bin())
        .args(["--url", &url, "invites", "redeem", "--token-file"])
        .arg(&tok)
        .arg("--key-file")
        .arg(&key_path)
        .env_remove("COWCHAT_INVITE_TOKEN")
        .output()
        .expect("run cowchat");
    server.join().unwrap();

    assert!(!out.status.success(), "a 404 must fail the redeem");
    assert!(
        !key_path.exists(),
        "nothing may be stored, and the reservation must be released for a retry"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("404"),
        "stderr should name the status: {stderr}"
    );
    // A 404 can also be the aftermath of a redemption that raced a room
    // destroy — the server burns the invite BEFORE that check — so the CLI
    // must not promise the invite survived.
    assert!(
        !stderr.contains("still live"),
        "a non-201 must not claim the invite is still live: {stderr}"
    );
}

/// A 5xx arrives AFTER the server has already burned a single-use invite
/// (redeem_invite commits before create_api_key/add_room_grant can fail), so
/// the CLI must say the invite may be spent — and must never echo a body that
/// looks like it carries a credential.
#[test]
fn a_late_server_failure_says_the_invite_may_be_spent() {
    let dir = tmpdir("latefail");
    let tok = dir.join("tok");
    fs::write(&tok, "cinv_latefail\n").unwrap();
    let key_path = dir.join("auth.key");

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = serve_one(
        listener,
        "500 Internal Server Error",
        r#"{"error":"grant failed","api_key":"leakcanary123"}"#,
    );

    let out = Command::new(bin())
        .args(["--url", &url, "invites", "redeem", "--token-file"])
        .arg(&tok)
        .arg("--key-file")
        .arg(&key_path)
        .env_remove("COWCHAT_INVITE_TOKEN")
        .output()
        .expect("run cowchat");
    server.join().unwrap();

    assert!(!out.status.success(), "a 500 must fail the redeem");
    assert!(!key_path.exists(), "nothing may be stored");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("invites list"),
        "a late failure must send the operator to check invite state: {stderr}"
    );
    assert!(
        !stderr.contains("still live"),
        "the server may have burned the invite already: {stderr}"
    );
    assert!(
        !stderr.contains("leakcanary123"),
        "a body that looks like it carries a credential must not be echoed: {stderr}"
    );
}

/// A credential can ride in any JSON shape, not just an object: a non-object
/// JSON body (array, scalar) must be withheld wholesale, never truncated-echoed
/// — only a top-level `error` string is ever shown from JSON.
#[test]
fn a_non_object_json_error_body_is_withheld() {
    let dir = tmpdir("jsonarray");
    let tok = dir.join("tok");
    fs::write(&tok, "cinv_jsonarray\n").unwrap();
    let key_path = dir.join("auth.key");

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = serve_one(
        listener,
        "502 Bad Gateway",
        r#"[{"apiKey":"leakarray456"}]"#,
    );

    let out = Command::new(bin())
        .args(["--url", &url, "invites", "redeem", "--token-file"])
        .arg(&tok)
        .arg("--key-file")
        .arg(&key_path)
        .env_remove("COWCHAT_INVITE_TOKEN")
        .output()
        .expect("run cowchat");
    server.join().unwrap();

    assert!(!out.status.success(), "a 502 must fail the redeem");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("leakarray456"),
        "JSON that is not an object must be withheld, whatever its field names: {stderr}"
    );
    assert!(
        stderr.contains("withheld"),
        "the operator should be told the body was withheld deliberately: {stderr}"
    );
}

/// An interrupted redeem leaves a zero-byte reservation behind. That file must
/// not wedge the flow with a claim that "a key already exists" — the error must
/// say it is empty, holds no secret, and name the exact removal command.
#[test]
fn an_empty_leftover_reservation_gets_removal_guidance() {
    let dir = tmpdir("leftover");
    let tok = dir.join("tok");
    fs::write(&tok, "cinv_leftover\n").unwrap();
    let key_path = dir.join("auth.key");
    fs::write(&key_path, "").unwrap();

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());

    let out = Command::new(bin())
        .args(["--url", &url, "invites", "redeem", "--token-file"])
        .arg(&tok)
        .arg("--key-file")
        .arg(&key_path)
        .env_remove("COWCHAT_INVITE_TOKEN")
        .output()
        .expect("run cowchat");

    // Still a refusal: an empty file may be another in-flight redeem's live
    // reservation, so it is never silently reclaimed.
    assert!(!out.status.success(), "must still refuse");
    listener.set_nonblocking(true).unwrap();
    assert!(
        matches!(listener.accept(), Err(e) if e.kind() == std::io::ErrorKind::WouldBlock),
        "the refusal must happen before the server is contacted"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("empty") && stderr.contains("rm "),
        "the error must say the file is empty and name the removal command: {stderr}"
    );
    assert!(
        !stderr.contains("Redeem with that key"),
        "must not tell the operator to use a key that does not exist: {stderr}"
    );
}

/// Regression: the secret writer used to chmod the destination's parent
/// unconditionally, so a key path in an existing directory silently made that
/// directory 0700.
#[cfg(unix)]
#[test]
fn an_existing_parent_directory_keeps_its_mode() {
    let dir = tmpdir("existingparent");
    let shared = dir.join("shared");
    fs::create_dir(&shared).unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&shared, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let tok = dir.join("tok");
    fs::write(&tok, "cinv_sharedparent\n").unwrap();

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = serve_one(
        listener,
        "201 Created",
        r#"{"api_key":"1e2d3c4b5a69788796a5b4c3d2e1f0091e2d3c4b5a69788796a5b4c3d2e1f009"}"#,
    );

    let out = Command::new(bin())
        .args(["--url", &url, "invites", "redeem", "--token-file"])
        .arg(&tok)
        .arg("--key-file")
        .arg(shared.join("auth.key"))
        .env_remove("COWCHAT_INVITE_TOKEN")
        .output()
        .expect("run cowchat");
    server.join().unwrap();

    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        mode(&shared),
        0o755,
        "a directory the user already had must not be re-permissioned"
    );
    assert_eq!(mode(&shared.join("auth.key")), 0o600);
}

/// A bare relative --key-file names a real destination and must work.
#[test]
fn keyless_redeem_accepts_a_bare_relative_key_file() {
    let dir = tmpdir("relative");
    let tok = dir.join("tok");
    fs::write(&tok, "cinv_relative\n").unwrap();

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = serve_one(
        listener,
        "201 Created",
        r#"{"api_key":"1e2d3c4b5a69788796a5b4c3d2e1f0091e2d3c4b5a69788796a5b4c3d2e1f009"}"#,
    );

    let out = Command::new(bin())
        .args(["--url", &url, "invites", "redeem", "--token-file"])
        .arg(&tok)
        .args(["--key-file", "mykey"])
        .current_dir(&dir)
        .env_remove("COWCHAT_INVITE_TOKEN")
        .output()
        .expect("run cowchat");
    server.join().unwrap();

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "bare relative path failed: {stderr}");
    assert_eq!(
        fs::read_to_string(dir.join("mykey")).unwrap().trim(),
        MINTED_KEY
    );
}

/// Minting needs the server's HTTP surface; without --url the error must say so
/// rather than surfacing a connection failure.
#[test]
fn keyless_redeem_without_a_url_explains_the_requirement() {
    let dir = tmpdir("nourl");
    let tok = dir.join("tok");
    fs::write(&tok, "cinv_nourl\n").unwrap();

    let out = Command::new(bin())
        .args(["invites", "redeem", "--token-file"])
        .arg(&tok)
        .arg("--key-file")
        .arg(dir.join("auth.key"))
        .env_remove("COWCHAT_INVITE_TOKEN")
        .output()
        .expect("run cowchat");

    assert!(
        !out.status.success(),
        "keyless redeem without --url must fail"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("--url"),
        "the error should point at --url.\nstderr: {stderr}"
    );
    assert!(
        !dir.join("auth.key").exists(),
        "no reservation may be left behind"
    );
}

/// A refused destination must be judged without being opened: a FIFO would
/// block the open forever, and a symlink would be followed elsewhere. Only a
/// zero-length regular file earns the "leftover reservation" hint.
#[cfg(unix)]
#[test]
fn a_non_regular_refused_destination_is_never_opened() {
    let dir = tmpdir("fifo");
    let tok = dir.join("tok");
    fs::write(&tok, "cinv_fifo\n").unwrap();
    let fifo = dir.join("auth.key");
    assert!(
        Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success(),
        "mkfifo failed"
    );

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());

    // If the CLI opened the FIFO (no writer exists), it would block forever —
    // so run it under a deadline and kill it, making a reintroduced bug a
    // failure rather than a hung suite.
    let mut child = Command::new(bin())
        .args(["--url", &url, "invites", "redeem", "--token-file"])
        .arg(&tok)
        .arg("--key-file")
        .arg(&fifo)
        .env_remove("COWCHAT_INVITE_TOKEN")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn cowchat");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    let status = loop {
        match child.try_wait().expect("poll cowchat") {
            Some(status) => break status,
            None if std::time::Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                panic!(
                    "the CLI hung on a FIFO destination — a refused destination \
                     must never be opened"
                );
            }
            None => std::thread::sleep(std::time::Duration::from_millis(50)),
        }
    };
    let out = child.wait_with_output().expect("collect output");

    assert!(!status.success(), "a FIFO destination must be refused");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("empty"),
        "a FIFO is not a leftover reservation and must not get the rm hint: {stderr}"
    );
}

/// Regression for the create-path reservation leak: when the server refuses to
/// mint, the reserved token file must be released — otherwise the next attempt
/// is refused over a possibly-unredeemed token that never existed. Uses a
/// minimal NDJSON server: register → ok, room_info → ok, create_invite → error.
#[test]
fn a_failed_mint_releases_the_token_reservation() {
    use std::io::{BufRead, BufReader};

    let dir = tmpdir("mintfail");
    let token_path = dir.join("tok");

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let reservation_probe = token_path.clone();
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().expect("the CLI should connect");
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut writer = stream;
        let mut exchange = |frame_type: &str, payload: &str, before_reply: &dyn Fn()| {
            let mut line = String::new();
            reader.read_line(&mut line).expect("read frame");
            let frame: serde_json::Value = serde_json::from_str(&line).expect("parse frame");
            before_reply();
            let id = frame["id"].as_str().expect("frame id");
            let response = format!(
                "{{\"id\":\"srv\",\"reply_to\":\"{id}\",\"type\":\"{frame_type}\",\"payload\":{payload}}}\n"
            );
            writer.write_all(response.as_bytes()).unwrap();
            frame
        };
        let register = exchange("ok", r#"{"agent_id":"probe"}"#, &|| {});
        assert_eq!(register["type"], "register");
        exchange("ok", r#"{"room_id":"r1"}"#, &|| {}); // room_info fast path
        let create = exchange(
            "error",
            r#"{"code":"internal_error","message":"mint refused by test server"}"#,
            // The reservation must exist NOW — after the reserve, before the
            // error reply that triggers the release. Without this probe the
            // test stays green even if the create path never reserves at all.
            &|| {
                assert!(
                    reservation_probe.exists(),
                    "the token destination must be reserved before minting is attempted"
                );
            },
        );
        assert_eq!(create["type"], "create_invite");
    });

    let out = Command::new(bin())
        .args([
            "--tcp",
            &addr,
            "--key",
            "testkey",
            "invites",
            "create",
            "r1",
            "--token-file",
        ])
        .arg(&token_path)
        .output()
        .expect("run cowchat");
    server.join().unwrap();

    assert!(
        !out.status.success(),
        "a refused mint must fail the command"
    );
    assert!(
        !token_path.exists(),
        "the reservation must be released when minting fails, or the next \
         attempt is refused over a token that never existed"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("mint refused by test server"),
        "the server's error should surface: {stderr}"
    );
}

/// `invites create --token-file` exists so the token never has to be retyped onto
/// a command line.
///
/// This asserts only that the flag is offered. The write itself needs a live
/// server to mint an invite, so it is **not** covered here — do not read this as
/// coverage of the 0600 mode or the never-printed property.
#[test]
fn create_accepts_a_token_file_destination() {
    let out = Command::new(bin())
        .args(["invites", "create", "--help"])
        .output()
        .expect("run cowchat");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("--token-file"),
        "create must offer a file destination for the token.\nstdout: {stdout}"
    );
}
