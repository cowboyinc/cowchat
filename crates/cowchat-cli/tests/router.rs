use std::io::Write;
use std::process::{Command, Stdio};

const ROUTES: &str = "alpha\tAlpha\tcore#1\\b\tpasskey\tAlpha wants\n";

fn classify(dir: &tempfile::TempDir, judge: &[&str], input: &str) -> Vec<serde_json::Value> {
    let routes = dir.path().join("routes.tsv");
    std::fs::write(&routes, ROUTES).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_cowchat"))
        .args(["router", "classify", "--routes"])
        .arg(&routes)
        .args(judge)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

const INPUT: &str = concat!(
    "wait: alive 30s room=x\n",
    r#"{"seq":1,"agent_name":"coord","content":"core#1 ready"}"#,
    "\n",
    r#"{"seq":2,"agent_name":"coord","content":"merged the lint fix"}"#,
    "\n",
    r#"{"seq":3,"agent_name":"coord","content":"the passkey flow"}"#,
    "\n",
);

#[test]
fn classify_relays_exact_matches_and_what_the_judge_picks() {
    let dir = tempfile::tempdir().unwrap();
    let judge = r#"cat >/dev/null; echo '{"relay":["T1"]}'"#;
    let decisions = classify(&dir, &["--judge-command", judge], INPUT);
    assert_eq!(decisions.len(), 2);
    assert_eq!(decisions[0]["seq"], 1);
    assert_eq!(decisions[0]["relay"], serde_json::json!(["alpha"]));
    assert_eq!(decisions[0]["judge"], "none");
    assert_eq!(decisions[1]["seq"], 3);
    assert_eq!(decisions[1]["relay"], serde_json::json!(["alpha"]));
    assert_eq!(decisions[1]["judge"], "ok");
}

#[test]
fn a_failing_judge_sends_the_message_to_a_person() {
    let dir = tempfile::tempdir().unwrap();
    let decisions = classify(&dir, &["--judge-command", "exit 1"], INPUT);
    assert_eq!(decisions[1]["relay"], serde_json::json!([]));
    assert_eq!(decisions[1]["attention"], true);
    assert_eq!(decisions[1]["judge"], "failed");
}

#[test]
fn no_judge_reports_candidates_without_calling_anything() {
    let dir = tempfile::tempdir().unwrap();
    let decisions = classify(&dir, &["--no-judge"], INPUT);
    assert_eq!(decisions[1]["candidates"], serde_json::json!(["alpha"]));
    assert_eq!(decisions[1]["judge"], "off");
}

#[test]
fn check_reports_a_broken_table() {
    let dir = tempfile::tempdir().unwrap();
    let routes = dir.path().join("routes.tsv");
    std::fs::write(&routes, "alpha\tAlpha\t(\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_cowchat"))
        .args(["router", "check", "--routes"])
        .arg(&routes)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("line 1"));
}

#[test]
fn mention_skips_a_line_it_cannot_read() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_cowchat"))
        .args(["router", "mention", "lobby"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"not json\n\n")
        .unwrap();
    assert!(child.wait().unwrap().success());
}
