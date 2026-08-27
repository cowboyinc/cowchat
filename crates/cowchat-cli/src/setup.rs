use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const SKILL: &[u8] = include_bytes!("../../../skills/cowchat/SKILL.md");
const MANIFEST_VERSION: u8 = 1;
const HOME_OVERRIDE: &str = "COWCHAT_SETUP_HOME";
const CONFIG_OVERRIDE: &str = "COWCHAT_SETUP_CONFIG_HOME";
static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, ValueEnum)]
pub(crate) enum SetupTarget {
    Codex,
    Zed,
    ClaudeCode,
}

impl SetupTarget {
    fn display_name(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::Zed => "Zed",
            Self::ClaudeCode => "Claude Code",
        }
    }
}

#[derive(Clone, Debug)]
struct SetupRoots {
    home: PathBuf,
    config: PathBuf,
}

impl SetupRoots {
    fn from_environment() -> Result<Self, Box<dyn Error>> {
        let overridden_home = std::env::var_os(HOME_OVERRIDE).map(PathBuf::from);
        let home = match overridden_home.as_ref() {
            Some(path) => path.clone(),
            None => directories::BaseDirs::new()
                .map(|dirs| dirs.home_dir().to_path_buf())
                .ok_or("could not determine the current user's home directory")?,
        };

        // When tests or callers override the home root, keep every derived path
        // under that root unless they also provide an explicit config root.
        let config = std::env::var_os(CONFIG_OVERRIDE)
            .map(PathBuf::from)
            .or_else(|| {
                overridden_home
                    .is_none()
                    .then(|| std::env::var_os("XDG_CONFIG_HOME"))
                    .flatten()
                    .map(PathBuf::from)
            })
            .unwrap_or_else(|| home.join(".config"));

        Ok(Self { home, config })
    }

    fn manifest_path(&self) -> PathBuf {
        self.home.join(".cowchat/agent-skill-installs.json")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InstallKind {
    AgentSkills,
    ClaudeCode,
}

impl InstallKind {
    fn key(self) -> &'static str {
        match self {
            Self::AgentSkills => "agent-skills",
            Self::ClaudeCode => "claude-code",
        }
    }
}

#[derive(Clone, Debug)]
struct InstallUnit {
    kind: InstallKind,
    label: String,
    path: PathBuf,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct OwnedInstall {
    path: PathBuf,
    sha256: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct OwnershipManifest {
    version: u8,
    installs: BTreeMap<String, OwnedInstall>,
}

impl Default for OwnershipManifest {
    fn default() -> Self {
        Self {
            version: MANIFEST_VERSION,
            installs: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Action {
    Install,
    Update,
    Current,
    PresentUnmanaged,
    Conflict(String),
    InspectError(String),
    Remove,
    ForgetMissing,
    Absent,
    PreserveUnmanaged,
}

impl Action {
    fn description(&self) -> String {
        match self {
            Self::Install => "install".to_string(),
            Self::Update => "update Cowchat-owned file".to_string(),
            Self::Current => "already current".to_string(),
            Self::PresentUnmanaged => "already present but unmanaged; leave unchanged".to_string(),
            Self::Conflict(reason) => format!("blocked; {reason}"),
            Self::InspectError(reason) => format!("blocked; cannot inspect file: {reason}"),
            Self::Remove => "remove matching Cowchat-owned file".to_string(),
            Self::ForgetMissing => "forget ownership record for missing file".to_string(),
            Self::Absent => "no Cowchat-owned file to remove".to_string(),
            Self::PreserveUnmanaged => "preserve unmanaged file".to_string(),
        }
    }

    fn mutates(&self) -> bool {
        matches!(
            self,
            Self::Install | Self::Update | Self::Remove | Self::ForgetMissing
        )
    }

    fn incomplete(&self) -> bool {
        matches!(self, Self::Conflict(_) | Self::InspectError(_))
    }
}

#[derive(Clone, Debug)]
struct Plan {
    unit: InstallUnit,
    action: Action,
}

struct SetupIncomplete {
    count: usize,
}

impl fmt::Debug for SetupIncomplete {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl fmt::Display for SetupIncomplete {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Cowchat skill setup is incomplete for {} destination{}; existing files were preserved",
            self.count,
            if self.count == 1 { "" } else { "s" }
        )
    }
}

impl Error for SetupIncomplete {}

enum FileState {
    Missing,
    Regular(String),
    Special,
}

pub(crate) fn run(
    requested_targets: &[SetupTarget],
    dry_run: bool,
    yes: bool,
    remove: bool,
) -> Result<(), Box<dyn Error>> {
    let roots = SetupRoots::from_environment()?;
    let manifest = load_manifest(&roots.manifest_path())?;
    let targets = select_targets(&roots, requested_targets, remove, &manifest);

    if targets.is_empty() {
        print_no_targets(remove);
        return Ok(());
    }

    let units = install_units(&roots, &targets);
    let plans = plan_units(&units, remove, &manifest, SKILL);

    println!(
        "Cowchat agent skill {} plan:",
        if remove { "removal" } else { "setup" }
    );
    for plan in &plans {
        println!(
            "  {}: {}\n    {}",
            plan.unit.label,
            plan.action.description(),
            plan.unit.path.display()
        );
    }

    let planned_incomplete = plans.iter().filter(|plan| plan.action.incomplete()).count();

    if dry_run {
        println!("Dry run: no files changed.");
        if planned_incomplete > 0 {
            return Err(Box::new(SetupIncomplete {
                count: planned_incomplete,
            }));
        }
        return Ok(());
    }

    let has_changes = plans.iter().any(|plan| plan.action.mutates());
    if !has_changes {
        if planned_incomplete > 0 {
            return Err(Box::new(SetupIncomplete {
                count: planned_incomplete,
            }));
        }
        println!("No changes needed.");
        if !remove {
            print_reload_hints(&targets);
        }
        return Ok(());
    }

    if !yes && !confirm()? {
        println!("Cancelled; no files changed.");
        return Ok(());
    }

    let incomplete = apply_plans(&roots, &plans, remove, manifest, SKILL)?;
    if !remove {
        print_reload_hints(&targets);
    }
    if incomplete > 0 {
        return Err(Box::new(SetupIncomplete { count: incomplete }));
    }

    Ok(())
}

fn select_targets(
    roots: &SetupRoots,
    requested: &[SetupTarget],
    remove: bool,
    manifest: &OwnershipManifest,
) -> BTreeSet<SetupTarget> {
    if !requested.is_empty() {
        return requested.iter().copied().collect();
    }

    let mut targets = detect_targets(roots);
    if remove {
        // Owned paths remain removable after an agent application is itself
        // uninstalled. Agent Skills is a shared Codex/Zed destination, so one
        // representative target is enough to select that install unit.
        if manifest
            .installs
            .contains_key(InstallKind::AgentSkills.key())
        {
            targets.insert(SetupTarget::Codex);
        }
        if manifest
            .installs
            .contains_key(InstallKind::ClaudeCode.key())
        {
            targets.insert(SetupTarget::ClaudeCode);
        }
    }
    targets
}

fn detect_targets(roots: &SetupRoots) -> BTreeSet<SetupTarget> {
    let mut targets = BTreeSet::new();
    if roots.home.join(".codex").is_dir() {
        targets.insert(SetupTarget::Codex);
    }
    if roots.home.join(".claude").is_dir() {
        targets.insert(SetupTarget::ClaudeCode);
    }
    if roots.config.join("zed").is_dir()
        || roots.home.join("Library/Application Support/Zed").is_dir()
    {
        targets.insert(SetupTarget::Zed);
    }
    targets
}

fn install_units(roots: &SetupRoots, targets: &BTreeSet<SetupTarget>) -> Vec<InstallUnit> {
    let mut units = Vec::new();
    let agent_targets: Vec<_> = [SetupTarget::Codex, SetupTarget::Zed]
        .into_iter()
        .filter(|target| targets.contains(target))
        .collect();
    if !agent_targets.is_empty() {
        units.push(InstallUnit {
            kind: InstallKind::AgentSkills,
            label: match agent_targets.as_slice() {
                [target] => format!("{} (shared Agent Skills path)", target.display_name()),
                _ => "Codex + Zed (shared Agent Skills path)".to_string(),
            },
            path: roots.home.join(".agents/skills/cowchat/SKILL.md"),
        });
    }
    if targets.contains(&SetupTarget::ClaudeCode) {
        units.push(InstallUnit {
            kind: InstallKind::ClaudeCode,
            label: SetupTarget::ClaudeCode.display_name().to_string(),
            path: roots.home.join(".claude/skills/cowchat/SKILL.md"),
        });
    }
    units
}

fn plan_units(
    units: &[InstallUnit],
    remove: bool,
    manifest: &OwnershipManifest,
    desired: &[u8],
) -> Vec<Plan> {
    units
        .iter()
        .cloned()
        .map(|unit| {
            let action = plan_action(&unit, remove, manifest, desired);
            Plan { unit, action }
        })
        .collect()
}

fn plan_action(
    unit: &InstallUnit,
    remove: bool,
    manifest: &OwnershipManifest,
    desired: &[u8],
) -> Action {
    let record = manifest.installs.get(unit.kind.key());
    if let Some(record) = record {
        if record.path != unit.path {
            return Action::Conflict(format!(
                "ownership metadata points to {}, not the expected path",
                record.path.display()
            ));
        }
    }

    let current = match inspect_file(&unit.path) {
        Ok(state) => state,
        Err(error) => return Action::InspectError(error.to_string()),
    };

    if remove {
        return match (record, current) {
            (None, FileState::Missing) => Action::Absent,
            (None, _) => Action::PreserveUnmanaged,
            (Some(_), FileState::Missing) => Action::ForgetMissing,
            (Some(record), FileState::Regular(hash)) if hash == record.sha256 => Action::Remove,
            (Some(_), FileState::Regular(_)) => Action::Conflict(
                "file changed since Cowchat installed it; leave it for the user".to_string(),
            ),
            (Some(_), FileState::Special) => Action::Conflict(
                "path is no longer a regular Cowchat-owned file; leave it for the user".to_string(),
            ),
        };
    }

    let desired_hash = sha256(desired);
    match (record, current) {
        (None, FileState::Missing) => Action::Install,
        (None, FileState::Regular(hash)) if hash == desired_hash => Action::PresentUnmanaged,
        (None, _) => Action::Conflict(
            "an unmanaged file already exists; review or move it before installing".to_string(),
        ),
        (Some(_), FileState::Missing) => Action::Install,
        (Some(record), FileState::Regular(hash)) if hash == record.sha256 => {
            if hash == desired_hash {
                Action::Current
            } else {
                Action::Update
            }
        }
        (Some(_), FileState::Regular(_)) => Action::Conflict(
            "file changed since Cowchat installed it; preserve the user's edits".to_string(),
        ),
        (Some(_), FileState::Special) => Action::Conflict(
            "path is no longer a regular Cowchat-owned file; preserve it".to_string(),
        ),
    }
}

fn inspect_file(path: &Path) -> io::Result<FileState> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => {
            let bytes = fs::read(path)?;
            Ok(FileState::Regular(sha256(&bytes)))
        }
        Ok(_) => Ok(FileState::Special),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(FileState::Missing),
        Err(error) => Err(error),
    }
}

fn apply_plans(
    roots: &SetupRoots,
    plans: &[Plan],
    remove: bool,
    mut manifest: OwnershipManifest,
    desired: &[u8],
) -> Result<usize, Box<dyn Error>> {
    // Establish writable ownership storage before touching a skill file. This
    // keeps a metadata permission error from producing an unmanaged install.
    if plans.iter().any(|plan| plan.action.mutates()) {
        save_manifest(&roots.manifest_path(), &manifest)?;
    }

    let desired_hash = sha256(desired);
    let mut incomplete = 0;
    let mut manifest_changed = false;

    for plan in plans {
        if plan.action.incomplete() {
            incomplete += 1;
            println!("  {}: preserved (blocked)", plan.unit.label);
            continue;
        }

        if !plan.action.mutates() {
            println!("  {}: {}", plan.unit.label, plan.action.description());
            continue;
        }

        // Re-evaluate immediately before the mutation. If the file moved or
        // changed after preview, fail closed for this target and continue.
        let refreshed = plan_action(&plan.unit, remove, &manifest, desired);
        if refreshed != plan.action {
            incomplete += 1;
            println!(
                "  {}: preserved because filesystem state changed after preview",
                plan.unit.label
            );
            continue;
        }

        let result = match &plan.action {
            Action::Install | Action::Update => {
                write_skill_and_verify(&plan.unit.path, desired, &desired_hash).map(|()| {
                    manifest.installs.insert(
                        plan.unit.kind.key().to_string(),
                        OwnedInstall {
                            path: plan.unit.path.clone(),
                            sha256: desired_hash.clone(),
                        },
                    );
                    manifest_changed = true;
                    if plan.action == Action::Install {
                        "installed"
                    } else {
                        "updated"
                    }
                })
            }
            Action::Remove => fs::remove_file(&plan.unit.path).map(|()| {
                manifest.installs.remove(plan.unit.kind.key());
                manifest_changed = true;
                "removed"
            }),
            Action::ForgetMissing => {
                manifest.installs.remove(plan.unit.kind.key());
                manifest_changed = true;
                Ok("forgot missing owned file")
            }
            _ => unreachable!("only mutating actions reach apply"),
        };

        match result {
            Ok(status) => println!("  {}: {status}", plan.unit.label),
            Err(error) => {
                incomplete += 1;
                println!("  {}: failed: {error}", plan.unit.label);
            }
        }
    }

    if manifest_changed {
        if let Err(error) = save_manifest(&roots.manifest_path(), &manifest) {
            incomplete += 1;
            println!("  Ownership metadata update failed: {error}");
        }
    }

    Ok(incomplete)
}

fn load_manifest(path: &Path) -> Result<OwnershipManifest, Box<dyn Error>> {
    let contents = match fs::read(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(OwnershipManifest::default());
        }
        Err(error) => return Err(error.into()),
    };
    let manifest: OwnershipManifest = serde_json::from_slice(&contents)?;
    if manifest.version != MANIFEST_VERSION {
        return Err(format!(
            "unsupported Cowchat skill ownership manifest version {} at {}",
            manifest.version,
            path.display()
        )
        .into());
    }
    Ok(manifest)
}

fn save_manifest(path: &Path, manifest: &OwnershipManifest) -> io::Result<()> {
    let mut bytes = serde_json::to_vec_pretty(manifest).map_err(io::Error::other)?;
    bytes.push(b'\n');
    atomic_write(path, &bytes)
}

fn atomic_write(path: &Path, contents: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no parent"))?;
    fs::create_dir_all(parent)?;

    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("cowchat");
    let nonce = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temporary = parent.join(format!(
        ".{file_name}.cowchat-{}-{nonce}.tmp",
        std::process::id()
    ));

    let write_result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        file.write_all(contents)?;
        file.sync_all()?;
        fs::rename(&temporary, path)
    })();

    if write_result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    write_result
}

fn write_skill_and_verify(path: &Path, contents: &[u8], expected_hash: &str) -> io::Result<()> {
    atomic_write(path, contents)?;
    match inspect_file(path)? {
        FileState::Regular(actual_hash) if actual_hash == expected_hash => Ok(()),
        FileState::Regular(_) => Err(io::Error::other(
            "installed file hash does not match the embedded skill",
        )),
        FileState::Missing => Err(io::Error::other(
            "installed file is missing after the write completed",
        )),
        FileState::Special => Err(io::Error::other(
            "installed path is not a regular file after the write completed",
        )),
    }
}

fn sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use fmt::Write as _;
        write!(&mut encoded, "{byte:02x}").expect("writing to a String cannot fail");
    }
    encoded
}

fn confirm() -> io::Result<bool> {
    print!("Apply this plan? [y/N] ");
    io::stdout().flush()?;
    let mut response = String::new();
    io::stdin().read_line(&mut response)?;
    Ok(matches!(
        response.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

fn print_no_targets(remove: bool) {
    println!(
        "No supported agent installations were detected; no files {}.",
        if remove { "removed" } else { "changed" }
    );
    println!("Select destinations explicitly, for example:");
    println!("  cowchat setup --target codex --target zed --target claude-code");
    if !remove {
        println!("Or install the portable skill with:");
        println!("  npx skills add cowboyinc/cowchat --skill cowchat --global");
    }
}

fn print_reload_hints(targets: &BTreeSet<SetupTarget>) {
    println!("Agent discovery for installed files:");
    if targets.contains(&SetupTarget::Codex) {
        println!("  Codex detects skill changes automatically; restart if Cowchat is not listed.");
    }
    if targets.contains(&SetupTarget::Zed) {
        println!("  Zed reloads Agent Skills immediately.");
    }
    if targets.contains(&SetupTarget::ClaudeCode) {
        println!(
            "  Claude Code reloads existing skill directories live; restart if this created ~/.claude/skills."
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn roots(path: &Path) -> SetupRoots {
        SetupRoots {
            home: path.to_path_buf(),
            config: path.join(".config"),
        }
    }

    fn selected(targets: &[SetupTarget]) -> BTreeSet<SetupTarget> {
        targets.iter().copied().collect()
    }

    #[test]
    fn codex_and_zed_share_one_agent_skills_install() {
        let temp = tempdir().unwrap();
        let units = install_units(
            &roots(temp.path()),
            &selected(&[SetupTarget::Codex, SetupTarget::Zed]),
        );

        assert_eq!(units.len(), 1);
        assert_eq!(units[0].kind, InstallKind::AgentSkills);
        assert_eq!(units[0].label, "Codex + Zed (shared Agent Skills path)");
        assert_eq!(
            units[0].path,
            temp.path().join(".agents/skills/cowchat/SKILL.md")
        );
    }

    #[test]
    fn owned_files_update_but_edited_files_fail_closed() {
        let temp = tempdir().unwrap();
        let roots = roots(temp.path());
        let units = install_units(&roots, &selected(&[SetupTarget::Codex]));

        let plans = plan_units(&units, false, &OwnershipManifest::default(), b"version one");
        assert_eq!(plans[0].action, Action::Install);
        assert_eq!(
            apply_plans(
                &roots,
                &plans,
                false,
                OwnershipManifest::default(),
                b"version one"
            )
            .unwrap(),
            0
        );

        let manifest = load_manifest(&roots.manifest_path()).unwrap();
        let plans = plan_units(&units, false, &manifest, b"version two");
        assert_eq!(plans[0].action, Action::Update);
        assert_eq!(
            apply_plans(&roots, &plans, false, manifest, b"version two").unwrap(),
            0
        );
        assert_eq!(fs::read(&units[0].path).unwrap(), b"version two");

        fs::write(&units[0].path, b"user edit").unwrap();
        let manifest = load_manifest(&roots.manifest_path()).unwrap();
        let plans = plan_units(&units, false, &manifest, b"version three");
        assert!(matches!(plans[0].action, Action::Conflict(_)));
        assert_eq!(
            apply_plans(&roots, &plans, false, manifest, b"version three").unwrap(),
            1
        );
        assert_eq!(fs::read(&units[0].path).unwrap(), b"user edit");
    }

    #[test]
    fn remove_deletes_only_an_unchanged_owned_file_and_keeps_directories() {
        let temp = tempdir().unwrap();
        let roots = roots(temp.path());
        let units = install_units(&roots, &selected(&[SetupTarget::ClaudeCode]));
        let plans = plan_units(&units, false, &OwnershipManifest::default(), b"owned");
        apply_plans(
            &roots,
            &plans,
            false,
            OwnershipManifest::default(),
            b"owned",
        )
        .unwrap();

        let manifest = load_manifest(&roots.manifest_path()).unwrap();
        let plans = plan_units(&units, true, &manifest, b"new binary contents");
        assert_eq!(plans[0].action, Action::Remove);
        assert_eq!(
            apply_plans(&roots, &plans, true, manifest, b"new binary contents").unwrap(),
            0
        );
        assert!(!units[0].path.exists());
        assert!(units[0].path.parent().unwrap().is_dir());
    }

    #[test]
    fn one_target_failure_does_not_stop_another_target() {
        let temp = tempdir().unwrap();
        let roots = roots(temp.path());
        fs::write(temp.path().join(".agents"), b"not a directory").unwrap();
        let units = install_units(
            &roots,
            &selected(&[SetupTarget::Codex, SetupTarget::ClaudeCode]),
        );
        let plans = plan_units(&units, false, &OwnershipManifest::default(), b"skill");

        assert_eq!(
            apply_plans(
                &roots,
                &plans,
                false,
                OwnershipManifest::default(),
                b"skill"
            )
            .unwrap(),
            1
        );
        assert_eq!(
            fs::read(temp.path().join(".claude/skills/cowchat/SKILL.md")).unwrap(),
            b"skill"
        );
        let manifest = load_manifest(&roots.manifest_path()).unwrap();
        assert!(!manifest
            .installs
            .contains_key(InstallKind::AgentSkills.key()));
        assert!(manifest
            .installs
            .contains_key(InstallKind::ClaudeCode.key()));
    }
}
