use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::tempdir;

fn repo_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

fn run_setup(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cowchat"))
        .arg("setup")
        .args(args)
        .env("COWCHAT_SETUP_HOME", home)
        .env("COWCHAT_SETUP_CONFIG_HOME", home.join(".config"))
        .output()
        .unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).unwrap()
}

#[test]
fn setup_previews_installs_idempotently_and_preserves_edits() {
    let temp = tempdir().unwrap();
    let skill_path = temp.path().join(".agents/skills/cowchat/SKILL.md");
    let manifest_path = temp.path().join(".cowchat/agent-skill-installs.json");

    let preview = run_setup(temp.path(), &["--target", "codex", "--dry-run"]);
    assert!(preview.status.success());
    assert!(stdout(&preview).contains(&skill_path.display().to_string()));
    assert!(stdout(&preview).contains("Dry run: no files changed."));
    assert!(!skill_path.exists());
    assert!(!manifest_path.exists());

    let cancelled = run_setup(temp.path(), &["--target", "codex"]);
    assert!(cancelled.status.success());
    assert!(stdout(&cancelled).contains("Cancelled; no files changed."));
    assert!(!skill_path.exists());

    let installed = run_setup(temp.path(), &["--target", "codex", "--yes"]);
    assert!(installed.status.success());
    assert!(stdout(&installed).contains("Codex (shared Agent Skills path): installed"));
    assert_eq!(
        fs::read(&skill_path).unwrap(),
        fs::read(repo_path("skills/cowchat/SKILL.md")).unwrap()
    );

    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    assert_eq!(manifest["version"], 1);
    assert_eq!(
        manifest["installs"]["agent-skills"]["path"],
        skill_path.display().to_string()
    );
    assert_eq!(
        manifest["installs"]["agent-skills"]["sha256"]
            .as_str()
            .unwrap()
            .len(),
        64
    );

    let repeated = run_setup(temp.path(), &["--target", "zed", "--yes"]);
    assert!(repeated.status.success());
    assert!(stdout(&repeated).contains("already current"));

    fs::write(&skill_path, b"user-owned edit\n").unwrap();
    let update = run_setup(temp.path(), &["--target", "codex", "--yes"]);
    assert!(!update.status.success());
    assert!(stdout(&update).contains("blocked; file changed since Cowchat installed it"));
    assert!(String::from_utf8_lossy(&update.stderr).contains("setup is incomplete"));
    assert_eq!(fs::read(&skill_path).unwrap(), b"user-owned edit\n");

    let remove = run_setup(temp.path(), &["--target", "codex", "--remove", "--yes"]);
    assert!(!remove.status.success());
    assert!(stdout(&remove).contains("blocked; file changed since Cowchat installed it"));
    assert_eq!(fs::read(&skill_path).unwrap(), b"user-owned edit\n");
}

#[test]
fn setup_detects_hosts_and_remove_keeps_parent_directories() {
    let temp = tempdir().unwrap();
    fs::create_dir_all(temp.path().join(".codex")).unwrap();
    let skill_path = temp.path().join(".agents/skills/cowchat/SKILL.md");
    let unrelated_skill = temp.path().join(".agents/skills/unrelated/SKILL.md");
    fs::create_dir_all(unrelated_skill.parent().unwrap()).unwrap();
    fs::write(&unrelated_skill, b"unrelated\n").unwrap();

    let installed = run_setup(temp.path(), &["--yes"]);
    assert!(installed.status.success());
    assert!(skill_path.is_file());
    assert!(stdout(&installed).contains("Codex (shared Agent Skills path): installed"));

    // Default removal also consults Cowchat's ownership manifest, so it still
    // works if the host application's own marker directory later disappears.
    fs::remove_dir(temp.path().join(".codex")).unwrap();
    let removed = run_setup(temp.path(), &["--remove", "--yes"]);
    assert!(removed.status.success());
    assert!(!skill_path.exists());
    assert!(skill_path.parent().unwrap().is_dir());
    assert_eq!(fs::read(unrelated_skill).unwrap(), b"unrelated\n");
}

#[test]
fn setup_without_detected_hosts_makes_no_changes() {
    let temp = tempdir().unwrap();
    let output = run_setup(temp.path(), &["--yes"]);

    assert!(output.status.success());
    assert!(stdout(&output).contains("No supported agent installations were detected"));
    assert!(stdout(&output).contains("--target codex"));
    assert!(!temp.path().join(".cowchat").exists());
    assert!(!temp.path().join(".agents").exists());
    assert!(!temp.path().join(".claude").exists());
}

#[test]
fn setup_reports_partial_failure_and_continues_other_destinations() {
    let temp = tempdir().unwrap();
    fs::write(temp.path().join(".agents"), b"not a directory\n").unwrap();

    let output = run_setup(
        temp.path(),
        &["--target", "codex", "--target", "claude-code", "--yes"],
    );

    assert!(!output.status.success());
    assert!(stdout(&output).contains("Codex (shared Agent Skills path): preserved (blocked)"));
    assert!(stdout(&output).contains("Claude Code: installed"));
    assert_eq!(
        fs::read(temp.path().join(".claude/skills/cowchat/SKILL.md")).unwrap(),
        fs::read(repo_path("skills/cowchat/SKILL.md")).unwrap()
    );
}
