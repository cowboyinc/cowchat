use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn repo_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

fn read_repo_file(relative: &str) -> String {
    fs::read_to_string(repo_path(relative)).unwrap()
}

fn cowchat_output(args: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_cowchat"))
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "cowchat {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn shell_blocks(document: &str) -> Vec<(usize, String)> {
    let mut blocks = Vec::new();
    let mut start_line = 0;
    let mut lines = Vec::new();
    let mut in_shell = false;

    for (index, line) in document.lines().enumerate() {
        let trimmed = line.trim();
        if !in_shell && matches!(trimmed, "```bash" | "```sh" | "```shell") {
            in_shell = true;
            start_line = index + 2;
            continue;
        }
        if in_shell && trimmed == "```" {
            blocks.push((start_line, lines.join("\n")));
            lines.clear();
            in_shell = false;
            continue;
        }
        if in_shell {
            lines.push(line);
        }
    }

    assert!(!in_shell, "unterminated fenced shell block");
    blocks
}

fn assert_named_examples_have_stable_identity(label: &str, document: &str) {
    for (start_line, block) in shell_blocks(document) {
        let block_sets_identity = block.contains("COWCHAT_AGENT_ID");
        for (offset, line) in block.lines().enumerate() {
            if line.contains("cowchat --name")
                && !line.contains("--agent-id")
                && !block_sets_identity
            {
                panic!(
                    "{label}:{} uses --name without --agent-id or COWCHAT_AGENT_ID: {line}",
                    start_line + offset
                );
            }
        }
    }
}

#[test]
fn embedded_skill_and_protocol_match_their_canonical_files() {
    let skill = read_repo_file("skills/cowchat/SKILL.md");
    let protocol = read_repo_file("SKILLS.md");

    assert_eq!(cowchat_output(&["skill"]), skill);
    assert_eq!(cowchat_output(&["skill", "--full"]), protocol);
}

#[test]
fn behavioral_skill_is_discoverable_bounded_and_lower_noise() {
    let skill = read_repo_file("skills/cowchat/SKILL.md");
    let protocol = read_repo_file("SKILLS.md");
    let mut frontmatter = skill.splitn(3, "---");

    assert_eq!(frontmatter.next(), Some(""));
    let header = frontmatter
        .next()
        .expect("skill must have YAML frontmatter");
    assert!(header.lines().any(|line| line == "name: cowchat"));
    assert!(header.lines().any(|line| line.starts_with("description:")));
    for trigger in [
        "Codex",
        "Claude Code",
        "Zed",
        "review",
        "handoff",
        "blockers",
    ] {
        assert!(header.contains(trigger), "missing trigger {trigger:?}");
    }

    assert!(skill.contains("cowchat rooms list --json"));
    assert!(skill.contains("run `cowchat rooms list --json` once"));
    assert!(skill.contains("untrusted metadata"));
    assert!(skill.contains("stay in it"));
    assert!(!skill.contains("~/.cowchat/auth.key"));
    assert!(!skill.to_lowercase().contains("flood"));
    assert!(skill.split_whitespace().count() <= 1_400);
    assert!(skill.len() < protocol.len());
}

#[test]
fn fenced_shell_examples_keep_named_agent_identity_stable() {
    let skill = read_repo_file("skills/cowchat/SKILL.md");
    let protocol = read_repo_file("SKILLS.md");

    assert_named_examples_have_stable_identity("skills/cowchat/SKILL.md", &skill);
    assert_named_examples_have_stable_identity("SKILLS.md", &protocol);
}
