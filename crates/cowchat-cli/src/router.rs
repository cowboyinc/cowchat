//! Decides which threads a room message should reach. Reads the JSON lines that
//! `cowchat wait --loop --drain` prints and writes one decision per wake.
//!
//! An exact pattern relays as is. A topic pattern only makes a thread a
//! candidate: one judge call per message reads each candidate's wants note.
//!
//! `router mention` turns those decisions into room messages: a follow-up that
//! mentions each route the original left out, so routed sessions wake through
//! the normal mention path. A route's id is the agent ID it mentions.
use crate::Cli;
use clap::{Args as ClapArgs, Subcommand};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
    process::Stdio,
    time::{Duration, Instant},
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const ASK_KINDS: [&str; 5] = ["question", "decision", "request", "needs_ryan", "relay"];
const ROUTED_KIND: &str = "routed";
const QUIET_KINDS: [&str; 3] = ["status", "pulse", ROUTED_KIND];
const MAX_MESSAGE_CHARS: usize = 1500;
const JUDGE_TIMEOUT: Duration = Duration::from_secs(120);
const WAKE_WINDOW: Duration = Duration::from_secs(3600);
const DEFAULT_JUDGE_PROMPT: &str = include_str!("router-judge-prompt.txt");

#[derive(ClapArgs)]
pub struct Args {
    #[command(subcommand)]
    action: Action,
}

#[derive(Subcommand)]
enum Action {
    /// Check that a route table parses.
    Check {
        #[arg(long)]
        routes: PathBuf,
    },
    /// Read `wait --loop --drain` output on stdin and print one JSON decision
    /// per message that should reach someone.
    Classify {
        /// Tab-separated rows: id, title, exact pattern, topic pattern, wants note.
        #[arg(long)]
        routes: PathBuf,
        /// Sender name to skip, normally this router's own.
        #[arg(long = "self", value_name = "NAME")]
        self_name: Option<String>,
        /// File of sender names, one per line, that wake only on an ask kind or
        /// an attention name.
        #[arg(long)]
        ignore_senders: Option<PathBuf>,
        /// A word that always wakes the router when a message contains it (repeatable).
        #[arg(long = "attention-name", value_name = "NAME")]
        attention_names: Vec<String>,
        /// Replace the built-in judge: this shell command reads the judge input on
        /// stdin and prints the reply.
        #[arg(long, conflicts_with = "no_judge")]
        judge_command: Option<String>,
        /// Model for the built-in judge.
        #[arg(long, default_value = "sonnet")]
        judge_model: String,
        /// System prompt file for the built-in judge.
        #[arg(long)]
        judge_prompt: Option<PathBuf>,
        /// Report candidates without judging them.
        #[arg(long)]
        no_judge: bool,
    },
    /// Read `router classify` output on stdin and post a follow-up mentioning
    /// each route the message left out. Prints one JSON result per follow-up.
    Mention {
        /// Room ID or name.
        room: String,
        /// Agent or `account:` ID to mention when nothing claims a message (repeatable).
        #[arg(long = "attention-mention", value_name = "ID")]
        attention_mentions: Vec<String>,
        /// Most follow-ups one target gets in an hour. Counted for as long as this
        /// command runs, so feed it from `wait --follow`.
        #[arg(long, default_value = "12")]
        max_wakes_per_hour: usize,
    },
}

#[derive(Debug)]
struct Route {
    id: String,
    title: String,
    exact: Regex,
    topic: Option<Regex>,
    wants: String,
}

struct Rules {
    routes: Vec<Route>,
    self_name: Option<String>,
    ignored: HashSet<String>,
    attention: Option<Regex>,
}

#[derive(Deserialize)]
struct RoomMessage {
    seq: i64,
    #[serde(default)]
    message_id: String,
    #[serde(default)]
    agent_id: Option<String>,
    #[serde(default)]
    agent_name: Option<String>,
    content: String,
    #[serde(default)]
    metadata: serde_json::Value,
}

/// A message that should reach someone, before any judge call.
#[derive(Debug)]
struct Wake {
    seq: i64,
    message_id: String,
    sender: String,
    sender_id: String,
    mentioned: Vec<String>,
    kind: String,
    line: String,
    routes: Vec<usize>,
    candidates: Vec<usize>,
    matched: HashMap<usize, String>,
    flagged: bool,
}

#[derive(Serialize)]
struct Decision<'a> {
    seq: i64,
    message_id: &'a str,
    sender: &'a str,
    kind: &'a str,
    message: &'a str,
    routes: Vec<&'a str>,
    candidates: Vec<&'a str>,
    relay: Vec<&'a str>,
    missing: Vec<&'a str>,
    reasons: Vec<Reason>,
    attention: bool,
    judge: &'static str,
}

/// Why a route was relayed to, carried on the follow-up so anyone can check it.
#[derive(Clone, Deserialize, Serialize)]
struct Reason {
    route: String,
    rule: String,
    matched: String,
}

/// The fields of a [`Decision`] that `router mention` acts on.
#[derive(Deserialize)]
struct RoutedMessage {
    seq: i64,
    message_id: String,
    sender: String,
    missing: Vec<String>,
    #[serde(default)]
    reasons: Vec<Reason>,
    attention: bool,
}

enum Judge {
    Off,
    Command(String),
    Claude { model: String, prompt: String },
}

fn pattern(source: &str) -> std::result::Result<Regex, regex::Error> {
    Regex::new(&format!("(?i){source}"))
}

fn parse_routes(text: &str) -> std::result::Result<Vec<Route>, String> {
    let mut routes = Vec::new();
    for (index, row) in text.lines().enumerate() {
        if row.is_empty() || row.starts_with('#') {
            continue;
        }
        let line = index + 1;
        let fields: Vec<&str> = row.split('\t').collect();
        let [id, title, exact, rest @ ..] = fields.as_slice() else {
            return Err(format!("line {line}: expected id, title and exact pattern"));
        };
        if id.is_empty() || exact.is_empty() {
            return Err(format!("line {line}: id and exact pattern are required"));
        }
        let topic = rest.first().filter(|topic| !topic.is_empty());
        routes.push(Route {
            id: (*id).to_owned(),
            title: (*title).to_owned(),
            exact: pattern(exact).map_err(|error| format!("line {line}: exact: {error}"))?,
            topic: topic
                .map(|topic| pattern(topic))
                .transpose()
                .map_err(|error| format!("line {line}: topic: {error}"))?,
            wants: rest.get(1).copied().unwrap_or_default().to_owned(),
        });
    }
    Ok(routes)
}

fn attention_pattern(names: &[String]) -> Option<Regex> {
    if names.is_empty() {
        return None;
    }
    let names: Vec<String> = names.iter().map(|name| regex::escape(name)).collect();
    Regex::new(&format!(r"(?i)(?-u:\b)(?:{})(?-u:\b)", names.join("|"))).ok()
}

fn classify(rules: &Rules, message: &RoomMessage) -> Option<Wake> {
    let sender = message.agent_name.as_deref().unwrap_or("?");
    let kind = message.metadata["kind"].as_str().unwrap_or_default();
    if QUIET_KINDS.contains(&kind) || rules.self_name.as_deref() == Some(sender) {
        return None;
    }
    let content = message.content.replace(['\n', '\r', '\u{1f}'], " ");
    let ask = ASK_KINDS.contains(&kind);
    let named = |text: &str| rules.attention.as_ref().is_some_and(|re| re.is_match(text));
    if rules.ignored.contains(sender) && !ask && !named(&content) {
        return None;
    }
    let (mut routes, mut candidates, mut matched) = (Vec::new(), Vec::new(), HashMap::new());
    for (index, route) in rules.routes.iter().enumerate() {
        if let Some(found) = route.exact.find(&content) {
            routes.push(index);
            matched.insert(index, found.as_str().to_owned());
        } else if let Some(found) = route.topic.as_ref().and_then(|re| re.find(&content)) {
            candidates.push(index);
            matched.insert(index, found.as_str().to_owned());
        }
    }
    if routes.is_empty() && candidates.is_empty() && !ask && !named(&content) {
        return None;
    }
    let shown: String = content.chars().take(MAX_MESSAGE_CHARS).collect();
    let line = format!("#{} {sender} [{kind}]: {shown}", message.seq);
    Some(Wake {
        seq: message.seq,
        message_id: message.message_id.clone(),
        sender: sender.to_owned(),
        sender_id: message.agent_id.clone().unwrap_or_default(),
        mentioned: message.metadata["mentions"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|id| id.as_str().map(str::to_owned))
            .collect(),
        kind: kind.to_owned(),
        flagged: ask || named(&line),
        line,
        routes,
        candidates,
        matched,
    })
}

fn judge_input(routes: &[Route], wake: &Wake) -> String {
    let mut input = String::from("Threads:");
    for (key, index) in wake.candidates.iter().enumerate() {
        let route = &routes[*index];
        input.push_str(&format!(
            "\nT{} ({}): {}",
            key + 1,
            route.title,
            route.wants
        ));
    }
    let (header, text) = wake.line.split_once(": ").unwrap_or((&wake.line, ""));
    input.push_str(&format!("\n\nMessage {header}:\n{text}"));
    input
}

/// Candidate positions the judge picked, or None when the reply has no verdict.
fn parse_verdict(reply: &str, candidates: usize) -> Option<Vec<usize>> {
    let start = reply.find('{')?;
    let end = reply.rfind('}')?;
    let verdict: serde_json::Value = serde_json::from_str(reply.get(start..=end)?).ok()?;
    let mut picked = Vec::new();
    for key in verdict.get("relay")?.as_array()? {
        let position = key
            .as_str()
            .and_then(|key| key.strip_prefix('T'))
            .and_then(|number| number.parse::<usize>().ok())
            .filter(|number| (1..=candidates).contains(number));
        if let Some(position) = position {
            if !picked.contains(&(position - 1)) {
                picked.push(position - 1);
            }
        }
    }
    Some(picked)
}

/// The routes a message relays to, and whether it needs a person because
/// nothing claimed it. A judge's no drops a message unless it asks or names someone.
fn place(wake: &Wake, verdict: Option<&[usize]>) -> (Vec<usize>, bool) {
    let mut relay = wake.routes.clone();
    let Some(picked) = verdict.filter(|_| !wake.candidates.is_empty()) else {
        let attention = relay.is_empty();
        return (relay, attention);
    };
    relay.extend(picked.iter().map(|position| wake.candidates[*position]));
    let attention = relay.is_empty() && wake.flagged;
    (relay, attention)
}

async fn call_judge(judge: &Judge, input: &str) -> Option<String> {
    let mut command = match judge {
        Judge::Off => return None,
        Judge::Command(shell) => {
            let mut command = tokio::process::Command::new("sh");
            command.args(["-c", shell]);
            command
        }
        Judge::Claude { model, prompt } => {
            let mut command = tokio::process::Command::new("claude");
            command.args(["-p", "--model", model, "--setting-sources", ""]);
            command.args(["--strict-mcp-config", "--tools", ""]);
            command.args(["--no-session-persistence", "--system-prompt", prompt]);
            command.args(["--output-format", "json"]);
            command
        }
    };
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let mut child = command.spawn().ok()?;
    let mut stdin = child.stdin.take()?;
    stdin.write_all(input.as_bytes()).await.ok()?;
    drop(stdin);
    let output = tokio::time::timeout(JUDGE_TIMEOUT, child.wait_with_output())
        .await
        .ok()?
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8(output.stdout).ok()?;
    if matches!(judge, Judge::Command(_)) {
        return Some(stdout);
    }
    let reply: serde_json::Value = serde_json::from_str(&stdout).ok()?;
    if reply["is_error"].as_bool() != Some(false) {
        return None;
    }
    reply["result"].as_str().map(str::to_owned)
}

async fn decide(rules: &Rules, judge: &Judge, wake: &Wake) -> String {
    let ids = |indexes: &[usize]| -> Vec<&str> {
        indexes
            .iter()
            .map(|index| rules.routes[*index].id.as_str())
            .collect()
    };
    let (verdict, outcome) = if wake.candidates.is_empty() {
        (None, "none")
    } else if matches!(judge, Judge::Off) {
        (None, "off")
    } else {
        let reply = call_judge(judge, &judge_input(&rules.routes, wake)).await;
        match reply.and_then(|reply| parse_verdict(&reply, wake.candidates.len())) {
            Some(picked) => (Some(picked), "ok"),
            None => (None, "failed"),
        }
    };
    let (relay, attention) = place(wake, verdict.as_deref());
    let reasons = relay
        .iter()
        .map(|index| Reason {
            route: rules.routes[*index].id.clone(),
            rule: if wake.routes.contains(index) {
                "exact"
            } else {
                "judge"
            }
            .to_owned(),
            matched: wake.matched[index].clone(),
        })
        .collect();
    let relay = ids(&relay);
    let missing = relay
        .iter()
        .copied()
        .filter(|id| *id != wake.sender_id && !wake.mentioned.iter().any(|seen| seen == id))
        .collect();
    serde_json::to_string(&Decision {
        seq: wake.seq,
        message_id: &wake.message_id,
        sender: &wake.sender,
        kind: &wake.kind,
        message: &wake.line,
        routes: ids(&wake.routes),
        candidates: ids(&wake.candidates),
        relay,
        missing,
        reasons,
        attention,
        judge: outcome,
    })
    .unwrap_or_default()
}

fn load_routes(path: &Path) -> Result<Vec<Route>> {
    Ok(parse_routes(&std::fs::read_to_string(path)?)?)
}

/// Caps how often one target is woken, so two sessions that route to each other
/// can't wake one another forever.
struct WakeLimit {
    per_hour: usize,
    recent: HashMap<String, VecDeque<Instant>>,
}

impl WakeLimit {
    fn new(per_hour: usize) -> Self {
        Self {
            per_hour,
            recent: HashMap::new(),
        }
    }

    fn allow(&mut self, target: &str, now: Instant) -> bool {
        let recent = self.recent.entry(target.to_owned()).or_default();
        while recent
            .front()
            .is_some_and(|woken| now.duration_since(*woken) >= WAKE_WINDOW)
        {
            recent.pop_front();
        }
        if recent.len() >= self.per_hour {
            return false;
        }
        recent.push_back(now);
        true
    }
}

/// Metadata is plaintext even in an encrypted room, so the matched text stays out.
fn follow_up_metadata(routed: &RoutedMessage, targets: &[String]) -> serde_json::Value {
    let reasons: Vec<_> = routed
        .reasons
        .iter()
        .filter(|reason| targets.contains(&reason.route))
        .map(|reason| serde_json::json!({ "route": reason.route, "rule": reason.rule }))
        .collect();
    serde_json::json!({ "kind": ROUTED_KIND, "reasons": reasons })
}

fn targets(routed: &RoutedMessage, attention_mentions: &[String]) -> Vec<String> {
    let mut targets = routed.missing.clone();
    if routed.attention {
        for id in attention_mentions {
            if !targets.contains(id) {
                targets.push(id.clone());
            }
        }
    }
    targets
}

async fn post_follow_up(
    cli: &Cli,
    room: &str,
    routed: &RoutedMessage,
    targets: &[String],
) -> Result<()> {
    let content = format!("Routed #{} from {}.", routed.seq, routed.sender);
    let reply_to = Some(routed.message_id.as_str()).filter(|id| !id.is_empty());
    let metadata = follow_up_metadata(routed, targets);
    crate::post_message(cli, room, &content, reply_to, targets, metadata).await?;
    Ok(())
}

/// The server refuses a whole message over one mention it can't accept, such as
/// a session that has ended. Sending each target alone keeps the rest.
async fn mention(
    cli: &Cli,
    room: &str,
    routed: &RoutedMessage,
    targets: Vec<String>,
) -> (Vec<String>, Vec<String>) {
    if targets.is_empty() {
        return (Vec::new(), Vec::new());
    }
    if post_follow_up(cli, room, routed, &targets).await.is_ok() {
        return (targets, Vec::new());
    }
    if targets.len() == 1 {
        return (Vec::new(), targets);
    }
    let (mut mentioned, mut failed) = (Vec::new(), Vec::new());
    for target in targets {
        match post_follow_up(cli, room, routed, std::slice::from_ref(&target)).await {
            Ok(()) => mentioned.push(target),
            Err(_) => failed.push(target),
        }
    }
    (mentioned, failed)
}

pub async fn run(cli: &Cli, args: &Args) -> Result<()> {
    match &args.action {
        Action::Check { routes } => {
            println!("{} routes ok", load_routes(routes)?.len());
        }
        Action::Classify {
            routes,
            self_name,
            ignore_senders,
            attention_names,
            judge_command,
            judge_model,
            judge_prompt,
            no_judge,
        } => {
            let ignored = match ignore_senders {
                Some(path) => std::fs::read_to_string(path)?
                    .lines()
                    .map(str::to_owned)
                    .collect(),
                None => HashSet::new(),
            };
            let rules = Rules {
                routes: load_routes(routes)?,
                self_name: self_name.clone(),
                ignored,
                attention: attention_pattern(attention_names),
            };
            let judge = match (no_judge, judge_command) {
                (true, _) => Judge::Off,
                (false, Some(shell)) => Judge::Command(shell.clone()),
                (false, None) => Judge::Claude {
                    model: judge_model.clone(),
                    prompt: match judge_prompt {
                        Some(path) => std::fs::read_to_string(path)?,
                        None => DEFAULT_JUDGE_PROMPT.to_owned(),
                    },
                },
            };
            let mut lines = BufReader::new(tokio::io::stdin()).lines();
            while let Some(line) = lines.next_line().await? {
                let Ok(message) = serde_json::from_str::<RoomMessage>(&line) else {
                    continue;
                };
                if let Some(wake) = classify(&rules, &message) {
                    println!("{}", decide(&rules, &judge, &wake).await);
                }
            }
        }
        Action::Mention {
            room,
            attention_mentions,
            max_wakes_per_hour,
        } => {
            let mut limit = WakeLimit::new(*max_wakes_per_hour);
            let mut lines = BufReader::new(tokio::io::stdin()).lines();
            while let Some(line) = lines.next_line().await? {
                let Ok(routed) = serde_json::from_str::<RoutedMessage>(&line) else {
                    eprintln!("router mention: skipped an unreadable line");
                    continue;
                };
                let targets = targets(&routed, attention_mentions);
                if targets.is_empty() {
                    continue;
                }
                let now = Instant::now();
                let (allowed, limited): (Vec<_>, Vec<_>) = targets
                    .into_iter()
                    .partition(|target| limit.allow(target, now));
                let (mentioned, failed) = mention(cli, room, &routed, allowed).await;
                let result = serde_json::json!({
                    "seq": routed.seq,
                    "mentioned": mentioned,
                    "failed": failed,
                    "limited": limited,
                });
                println!("{result}");
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const TABLE: &str = "# id\ttitle\texact\ttopic\twants\n\
        a\tAlpha\tcore#1\\b\tpasskey\tAlpha wants\n\
        b\tBeta\tbeta-branch\tpasskey|feeds\tBeta wants\n";

    fn rules() -> Rules {
        Rules {
            routes: parse_routes(TABLE).unwrap(),
            self_name: Some("router".to_owned()),
            ignored: HashSet::from(["noisy".to_owned()]),
            attention: attention_pattern(&["ryan".to_owned()]),
        }
    }

    fn message(sender: &str, kind: &str, content: &str) -> RoomMessage {
        RoomMessage {
            seq: 7,
            message_id: "original".to_owned(),
            agent_id: Some(format!("{sender}-id")),
            agent_name: Some(sender.to_owned()),
            content: content.to_owned(),
            metadata: if kind.is_empty() {
                json!({})
            } else {
                json!({ "kind": kind })
            },
        }
    }

    fn wake(sender: &str, kind: &str, content: &str) -> Option<Wake> {
        classify(&rules(), &message(sender, kind, content))
    }

    #[test]
    fn routes_skip_comments_and_keep_every_column() {
        let routes = parse_routes(TABLE).unwrap();
        assert_eq!(routes.len(), 2);
        assert_eq!(routes[0].id, "a");
        assert_eq!(routes[1].title, "Beta");
        assert_eq!(routes[1].wants, "Beta wants");
    }

    #[test]
    fn a_route_may_omit_its_topic_and_wants() {
        let routes = parse_routes("a\tAlpha\tcore#1\n").unwrap();
        assert!(routes[0].topic.is_none());
        assert_eq!(routes[0].wants, "");
    }

    #[test]
    fn bad_rows_name_their_line() {
        for (table, expected) in [
            ("# c\na\tAlpha\n", "line 2"),
            ("a\tAlpha\t(\n", "line 1"),
            ("a\tAlpha\tok\t(\n", "line 1"),
            ("a\tAlpha\t\n", "line 1"),
        ] {
            let error = parse_routes(table).unwrap_err();
            assert!(error.contains(expected), "{table:?}: {error}");
        }
    }

    #[test]
    fn an_exact_match_routes_and_ignores_case() {
        let wake = wake("coord", "", "CORE#1 is ready").unwrap();
        assert_eq!(wake.routes, [0]);
        assert!(wake.candidates.is_empty());
    }

    #[test]
    fn a_topic_match_only_makes_a_candidate() {
        let wake = wake("coord", "", "the passkey flow").unwrap();
        assert!(wake.routes.is_empty());
        assert_eq!(wake.candidates, [0, 1]);
    }

    #[test]
    fn an_exact_match_beats_that_routes_own_topic() {
        let wake = wake("coord", "", "core#1 and the passkey flow").unwrap();
        assert_eq!(wake.routes, [0]);
        assert_eq!(wake.candidates, [1]);
    }

    #[test]
    fn quiet_kinds_own_posts_and_unrelated_messages_do_not_wake() {
        assert!(wake("coord", "status", "core#1").is_none());
        assert!(wake("coord", "pulse", "core#1").is_none());
        assert!(wake("router", "", "core#1").is_none());
        assert!(wake("coord", "", "merged the lint fix").is_none());
    }

    #[test]
    fn ask_kinds_and_attention_names_wake_without_a_route() {
        assert!(wake("coord", "question", "anyone around").is_some());
        assert!(wake("coord", "", "ping Ryan about it").is_some());
        assert!(wake("coord", "", "booked on ryanair").is_none());
    }

    #[test]
    fn ignored_senders_wake_only_on_an_ask_or_a_name() {
        assert!(wake("noisy", "", "core#1").is_none());
        assert!(wake("noisy", "question", "core#1").is_some());
        assert!(wake("noisy", "", "core#1 for ryan").is_some());
    }

    #[test]
    fn newlines_fold_so_patterns_and_the_message_line_stay_on_one_line() {
        let wake = wake("coord", "fyi", "first\ncore#1\rlast").unwrap();
        assert_eq!(wake.routes, [0]);
        assert_eq!(wake.line, "#7 coord [fyi]: first core#1 last");
    }

    #[test]
    fn long_messages_are_cut_on_character_boundaries() {
        let content = format!("core#1 {}", "é".repeat(MAX_MESSAGE_CHARS));
        let wake = wake("coord", "", &content).unwrap();
        let text = wake.line.split_once(": ").unwrap().1;
        assert_eq!(text.chars().count(), MAX_MESSAGE_CHARS);
    }

    #[test]
    fn the_judge_sees_only_candidates_under_stable_keys() {
        let rules = rules();
        let wake = classify(&rules, &message("coord", "", "core#1: the feeds")).unwrap();
        assert_eq!(
            judge_input(&rules.routes, &wake),
            "Threads:\nT1 (Beta): Beta wants\n\nMessage #7 coord []:\ncore#1: the feeds"
        );
    }

    #[test]
    fn verdicts_survive_prose_and_drop_keys_that_are_not_candidates() {
        assert_eq!(
            parse_verdict(
                "Sure:\n{\"relay\":\n[\"T2\",\"T9\",\"T0\",\"x\",3,\"T2\"]}\nDone",
                2
            ),
            Some(vec![1])
        );
        assert_eq!(parse_verdict(r#"{"relay":[]}"#, 2), Some(vec![]));
    }

    #[test]
    fn a_reply_without_a_relay_list_is_not_a_verdict() {
        for reply in ["", "no", "{bad}", r#"{"other":1}"#, r#"{"relay":"T1"}"#] {
            assert_eq!(parse_verdict(reply, 2), None, "{reply}");
        }
    }

    #[test]
    fn exact_routes_relay_with_no_judge_involved() {
        let wake = wake("coord", "", "core#1").unwrap();
        assert_eq!(place(&wake, None), (vec![0], false));
    }

    #[test]
    fn the_judge_adds_only_the_candidates_it_picks() {
        let wake = wake("coord", "", "core#1 and the feeds passkey").unwrap();
        assert_eq!(place(&wake, Some(&[0])), (vec![0, 1], false));
        let wake = self::wake("coord", "", "the passkey flow").unwrap();
        assert_eq!(place(&wake, Some(&[1])), (vec![1], false));
    }

    #[test]
    fn a_judge_no_drops_a_plain_message() {
        let wake = wake("coord", "", "the passkey flow").unwrap();
        assert_eq!(place(&wake, Some(&[])), (vec![], false));
    }

    #[test]
    fn a_judge_no_still_needs_a_person_for_an_ask_or_a_name() {
        let ask = wake("coord", "question", "the passkey flow").unwrap();
        assert_eq!(place(&ask, Some(&[])), (vec![], true));
        let named = wake("coord", "", "ryan: the passkey flow").unwrap();
        assert_eq!(place(&named, Some(&[])), (vec![], true));
    }

    #[test]
    fn a_failed_judge_sends_unclaimed_messages_to_a_person() {
        let wake = wake("coord", "", "the passkey flow").unwrap();
        assert_eq!(place(&wake, None), (vec![], true));
        let routed = self::wake("coord", "", "core#1 and feeds").unwrap();
        assert_eq!(place(&routed, None), (vec![0], false));
    }

    #[test]
    fn an_ask_with_no_route_needs_a_person() {
        let wake = wake("coord", "question", "anyone around").unwrap();
        assert_eq!(place(&wake, None), (vec![], true));
    }

    #[test]
    fn follow_ups_never_wake_a_router() {
        assert!(classify(&rules(), &message("other", "routed", "core#1 ready")).is_none());
    }

    #[tokio::test]
    async fn only_routes_the_message_left_out_get_a_follow_up() {
        let mut mentioned = message("coord", "", "core#1 beta-branch");
        mentioned.metadata = json!({ "mentions": ["a"] });
        let wake = classify(&rules(), &mentioned).unwrap();
        let decision: serde_json::Value =
            serde_json::from_str(&decide(&rules(), &Judge::Off, &wake).await).unwrap();
        assert_eq!(decision["relay"], json!(["a", "b"]));
        assert_eq!(decision["missing"], json!(["b"]));
        assert_eq!(decision["message_id"], "original");
    }

    #[tokio::test]
    async fn a_session_is_never_routed_its_own_message() {
        let wake = classify(&rules(), &message("a", "", "core#1 beta-branch")).unwrap();
        let wake = Wake {
            sender_id: "a".to_owned(),
            ..wake
        };
        let decision: serde_json::Value =
            serde_json::from_str(&decide(&rules(), &Judge::Off, &wake).await).unwrap();
        assert_eq!(decision["missing"], json!(["b"]));
    }

    #[test]
    fn a_target_past_its_hourly_limit_waits_for_the_window_to_pass() {
        let start = Instant::now();
        let mut limit = WakeLimit::new(2);
        assert!(limit.allow("a", start));
        assert!(limit.allow("a", start));
        assert!(!limit.allow("a", start + Duration::from_secs(60)));
        assert!(limit.allow("b", start + Duration::from_secs(60)));
        assert!(limit.allow("a", start + Duration::from_secs(3601)));
    }

    #[tokio::test]
    async fn each_relay_records_the_rule_and_text_that_chose_it() {
        let wake = classify(&rules(), &message("coord", "", "core#1 and passkey")).unwrap();
        let judge = Judge::Command(r#"cat >/dev/null; echo '{"relay":["T1"]}'"#.to_owned());
        let decision: serde_json::Value =
            serde_json::from_str(&decide(&rules(), &judge, &wake).await).unwrap();
        assert_eq!(
            decision["reasons"],
            json!([
                {"route": "a", "rule": "exact", "matched": "core#1"},
                {"route": "b", "rule": "judge", "matched": "passkey"},
            ])
        );
    }

    #[test]
    fn follow_up_reasons_carry_no_message_text() {
        let routed = RoutedMessage {
            seq: 7,
            message_id: "original".to_owned(),
            sender: "coord".to_owned(),
            missing: vec!["a".to_owned()],
            reasons: vec![
                Reason {
                    route: "a".to_owned(),
                    rule: "exact".to_owned(),
                    matched: "secret text".to_owned(),
                },
                Reason {
                    route: "b".to_owned(),
                    rule: "judge".to_owned(),
                    matched: "other".to_owned(),
                },
            ],
            attention: false,
        };
        assert_eq!(
            follow_up_metadata(&routed, &["a".to_owned()]),
            json!({"kind": "routed", "reasons": [{"route": "a", "rule": "exact"}]})
        );
    }

    #[test]
    fn a_target_named_twice_is_mentioned_once() {
        let routed = RoutedMessage {
            seq: 7,
            message_id: String::new(),
            sender: "coord".to_owned(),
            missing: vec!["a".to_owned()],
            reasons: Vec::new(),
            attention: true,
        };
        assert_eq!(
            targets(&routed, &["a".to_owned(), "owner".to_owned()]),
            ["a", "owner"]
        );
    }
}
