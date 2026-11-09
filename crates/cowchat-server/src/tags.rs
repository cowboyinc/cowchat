//! Stamps `metadata.tags` on accepted messages. Author tags are validated and
//! marked `author`; the extractor adds identifiers found in plaintext rooms.

use cowchat_core::tags::{
    MessageTag, MAX_TAGS_PER_MESSAGE, TAG_SOURCE_AUTHOR, TAG_SOURCE_EXTRACTOR,
};
use regex::Regex;
use serde_json::Value;
use std::sync::LazyLock;

const GITHUB_KEY: &str = "github";
const LINEAR_KEY: &str = "linear";
const NOT_REPOSITORIES: [&str; 6] = ["pr", "prs", "issue", "issues", "pull", "ticket"];

static GITHUB_LINK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)https?://(?:www\.)?github\.com/[\w.-]+/([\w.-]+)/(?:pull|issues)/(\d+)")
        .unwrap()
});
static ANY_LINK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"https?://\S+").unwrap());
static REPOSITORY_REFERENCE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:^|[^\w./@#-])(?:[\w.-]+/)?([A-Za-z][\w.-]*[A-Za-z0-9])#(\d+)\b").unwrap()
});
static LINEAR_TICKET: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bCOW-\d+\b").unwrap());

pub(crate) fn stamp_tags(
    metadata: &mut Value,
    content: &str,
    encrypted: bool,
) -> Result<(), String> {
    let mut tags = author_tags(metadata)?;
    if encrypted && !tags.is_empty() {
        return Err("tags are not available in end-to-end encrypted rooms".to_owned());
    }
    if !encrypted {
        for tag in extracted_tags(content, metadata) {
            if tags.len() == MAX_TAGS_PER_MESSAGE {
                break;
            }
            push_unique(&mut tags, tag);
        }
    }
    if let Some(fields) = metadata.as_object_mut() {
        fields.remove("tags");
    }
    if !tags.is_empty() {
        if !metadata.is_object() {
            *metadata = serde_json::json!({});
        }
        metadata["tags"] = serde_json::json!(tags);
    }
    Ok(())
}

fn author_tags(metadata: &Value) -> Result<Vec<MessageTag>, String> {
    let Some(value) = metadata.get("tags") else {
        return Ok(Vec::new());
    };
    let supplied: Vec<MessageTag> = serde_json::from_value(value.clone())
        .map_err(|_| "metadata.tags must be a list of objects with a key".to_owned())?;
    if supplied.len() > MAX_TAGS_PER_MESSAGE {
        return Err(format!(
            "a message can carry at most {MAX_TAGS_PER_MESSAGE} tags"
        ));
    }
    let mut tags = Vec::with_capacity(supplied.len());
    for mut tag in supplied {
        tag.validate()?;
        tag.source = Some(TAG_SOURCE_AUTHOR.to_owned());
        push_unique(&mut tags, tag);
    }
    Ok(tags)
}

fn extracted_tags(content: &str, metadata: &Value) -> Vec<MessageTag> {
    let mut tags = Vec::new();
    let pull_request = &metadata["pull_request"];
    if let (Some(repository), Some(number)) = (
        pull_request["repository"].as_str(),
        pull_request["number"].as_i64(),
    ) {
        let name = repository.rsplit('/').next().unwrap_or(repository);
        tags.push(extracted(GITHUB_KEY, github_value(name, number)));
    }
    for link in GITHUB_LINK.captures_iter(content) {
        tags.push(extracted(GITHUB_KEY, github_value(&link[1], &link[2])));
    }
    let without_links = ANY_LINK.replace_all(content, " ");
    for reference in REPOSITORY_REFERENCE.captures_iter(&without_links) {
        let name = reference[1].to_ascii_lowercase();
        if !NOT_REPOSITORIES.contains(&name.as_str()) {
            tags.push(extracted(GITHUB_KEY, github_value(&name, &reference[2])));
        }
    }
    for ticket in LINEAR_TICKET.find_iter(content) {
        tags.push(extracted(LINEAR_KEY, ticket.as_str().to_owned()));
    }
    tags.retain(|tag| tag.validate().is_ok());
    tags
}

fn github_value(repository: &str, number: impl std::fmt::Display) -> String {
    format!("{}#{number}", repository.to_ascii_lowercase())
}

fn extracted(key: &str, value: String) -> MessageTag {
    MessageTag {
        key: key.to_owned(),
        value: Some(value),
        source: Some(TAG_SOURCE_EXTRACTOR.to_owned()),
    }
}

fn push_unique(tags: &mut Vec<MessageTag>, tag: MessageTag) {
    if !tags
        .iter()
        .any(|existing| existing.key == tag.key && existing.value == tag.value)
    {
        tags.push(tag);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cowchat_core::tags::{metadata_tags, MAX_TAGS_PER_MESSAGE};
    use serde_json::{json, Value};

    fn stamped(mut metadata: Value, content: &str) -> Value {
        stamp_tags(&mut metadata, content, false).unwrap();
        metadata
    }

    fn extracted(content: &str) -> Vec<String> {
        metadata_tags(&stamped(json!({}), content))
            .into_iter()
            .map(|tag| format!("{}={}", tag.key, tag.value.unwrap()))
            .collect()
    }

    #[test]
    fn author_tags_are_marked_author_even_when_the_client_claims_otherwise() {
        let metadata = stamped(
            json!({"tags":[{"key":"topic","value":"router","source":"extractor","extra":1}]}),
            "hello",
        );
        assert_eq!(
            metadata["tags"],
            json!([{"key":"topic","value":"router","source":"author"}])
        );
    }

    #[test]
    fn invalid_author_tags_are_rejected() {
        for tags in [
            json!("topic"),
            json!(["topic"]),
            json!([{"key":"Bad Key"}]),
            json!([{"value":"no key"}]),
        ] {
            assert!(stamp_tags(&mut json!({"tags":tags}), "hello", false).is_err());
        }
        let too_many: Vec<Value> = (0..=MAX_TAGS_PER_MESSAGE)
            .map(|index| json!({"key":format!("k{index}")}))
            .collect();
        assert!(stamp_tags(&mut json!({"tags":too_many}), "hello", false).is_err());
    }

    #[test]
    fn repeated_author_tags_collapse() {
        let metadata = stamped(
            json!({"tags":[{"key":"topic","value":"a"},{"key":"topic","value":"a"},{"key":"topic","value":"b"}]}),
            "hello",
        );
        assert_eq!(metadata_tags(&metadata).len(), 2);
    }

    #[test]
    fn a_message_without_tags_gets_no_tags_field() {
        assert_eq!(
            stamped(json!({"kind":"fyi"}), "hello"),
            json!({"kind":"fyi"})
        );
        assert_eq!(stamped(json!({"tags":[]}), "hello"), json!({}));
    }

    #[test]
    fn encrypted_rooms_refuse_author_tags_because_metadata_is_plaintext() {
        let error = stamp_tags(&mut json!({"tags":[{"key":"topic"}]}), "x", true).unwrap_err();
        assert!(error.contains("encrypted"));
    }

    #[test]
    fn encrypted_rooms_get_no_extracted_tags() {
        let mut metadata = json!({});
        stamp_tags(&mut metadata, "cowchat#95 COW-4857", true).unwrap();
        assert_eq!(metadata, json!({}));
    }

    #[test]
    fn extractor_finds_repo_references_and_linear_tickets() {
        assert_eq!(
            extracted("see cowchat#95, cowboyinc/core#1403 and COW-4857."),
            ["github=cowchat#95", "github=core#1403", "linear=COW-4857"]
        );
    }

    #[test]
    fn extractor_reads_github_links_and_linear_links() {
        assert_eq!(
            extracted("https://github.com/cowboyinc/cowchat/pull/95 and https://github.com/cowboyinc/core/issues/7"),
            ["github=cowchat#95", "github=core#7"]
        );
        assert_eq!(
            extracted("https://linear.app/cowboy/issue/COW-4857/room-plan"),
            ["linear=COW-4857"]
        );
    }

    #[test]
    fn extractor_gives_one_tag_per_identifier() {
        assert_eq!(
            extracted("Cowchat#95 cowchat#95 https://github.com/cowboyinc/cowchat/pull/95"),
            ["github=cowchat#95"]
        );
    }

    #[test]
    fn extractor_ignores_lookalikes() {
        for content in [
            "PR#95 and issue#3 and #42",
            "C#7 is a language",
            "https://example.com/page#12",
            "UTF-8 SHA-256 CIP-49 XCOW-12 cow-12",
            "a@b#1",
        ] {
            assert!(extracted(content).is_empty(), "{content}");
        }
    }

    #[test]
    fn extractor_reads_pull_request_metadata() {
        let metadata = stamped(
            json!({"pull_request":{"repository":"cowboyinc/cowchat","number":95}}),
            "ready for review",
        );
        assert_eq!(
            metadata["tags"],
            json!([{"key":"github","value":"cowchat#95","source":"extractor"}])
        );
    }

    #[test]
    fn an_author_tag_wins_over_the_same_extracted_tag() {
        let metadata = stamped(
            json!({"tags":[{"key":"linear","value":"COW-1"}]}),
            "COW-1 and COW-2",
        );
        assert_eq!(
            metadata["tags"],
            json!([
                {"key":"linear","value":"COW-1","source":"author"},
                {"key":"linear","value":"COW-2","source":"extractor"}
            ])
        );
    }

    #[test]
    fn extraction_stops_at_the_message_tag_limit() {
        let content: Vec<String> = (1..100).map(|n| format!("COW-{n}")).collect();
        assert_eq!(extracted(&content.join(" ")).len(), MAX_TAGS_PER_MESSAGE);
    }
}
