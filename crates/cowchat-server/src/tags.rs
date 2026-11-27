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
/// A `owner/name#N` whose name ends like this is a page anchor, not a repository.
const DOCUMENT_EXTENSIONS: [&str; 11] = [
    ".html",
    ".htm",
    ".md",
    ".mdx",
    ".markdown",
    ".txt",
    ".pdf",
    ".php",
    ".xml",
    ".rst",
    ".adoc",
];

static GITHUB_LINK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)https?://(?:www\.)?github\.com/([A-Za-z0-9_.-]+)/([A-Za-z0-9_.-]+)/(?:pull|issues)/(\d+)",
    )
    .unwrap()
});
static ANY_LINK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"https?://\S+").unwrap());
static REPOSITORY_REFERENCE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:^|[^A-Za-z0-9_./@#-])(?:([A-Za-z0-9_.-]+)/)?([A-Za-z0-9_.-]+)#(\d+)\b").unwrap()
});
static LINEAR_LINK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)https?://linear\.app/[A-Za-z0-9_-]+/issue/([a-z][a-z0-9]*-\d+)").unwrap()
});

/// What the extractor tags without a link. Links and `owner/repo#N` are tagged
/// for anyone; a bare ticket ID (`COW-123`) only for the listed Linear teams,
/// and a bare `repo#N` only for the listed GitHub repositories, because bare
/// forms are too often something else (`SHA-256`, `bug#3`).
pub struct TagExtraction {
    bare_ticket: Option<Regex>,
    /// Lowercase `(owner, repo)` pairs.
    repositories: Vec<(String, String)>,
}

impl Default for TagExtraction {
    fn default() -> Self {
        Self::new(&["COW".to_owned()], &[]).expect("the default settings are valid")
    }
}

impl TagExtraction {
    /// `linear_teams` are team keys such as `COW`; `github_repositories` are
    /// `owner/repo`. Empty entries are ignored.
    pub fn new(linear_teams: &[String], github_repositories: &[String]) -> Result<Self, String> {
        let mut teams = Vec::new();
        for team in linear_teams.iter().map(|team| team.trim()) {
            if team.is_empty() {
                continue;
            }
            if team.len() > 16
                || !team.starts_with(|c: char| c.is_ascii_alphabetic())
                || !team.chars().all(|c| c.is_ascii_alphanumeric())
            {
                return Err(format!(
                    "Linear team {team:?} must be up to 16 letters and digits, starting with a letter"
                ));
            }
            teams.push(team.to_ascii_uppercase());
        }
        let bare_ticket = (!teams.is_empty()).then(|| {
            Regex::new(&format!(r"\b(?:{})-\d+\b", teams.join("|")))
                .expect("team keys are alphanumeric")
        });
        let mut repositories = Vec::new();
        for repository in github_repositories
            .iter()
            .map(|repository| repository.trim())
        {
            if repository.is_empty() {
                continue;
            }
            match repository.split_once('/') {
                Some((owner, name)) if is_owner_name(owner) && is_repository_name(name) => {
                    repositories.push((owner.to_ascii_lowercase(), name.to_ascii_lowercase()));
                }
                _ => {
                    return Err(format!(
                        "GitHub repository {repository:?} must be owner/repo"
                    ))
                }
            }
        }
        Ok(Self {
            bare_ticket,
            repositories,
        })
    }

    /// The owner of the one listed repository called `name`, if exactly one is.
    fn owner_of(&self, name: &str) -> Option<&str> {
        let mut owners = self
            .repositories
            .iter()
            .filter(|(_, repository)| repository.eq_ignore_ascii_case(name));
        let (owner, _) = owners.next()?;
        owners.next().is_none().then_some(owner.as_str())
    }

    /// `owner/repo#N` for a reference, mapping a bare `repo#N` through the list.
    fn github_tag(&self, owner: Option<&str>, name: &str, number: &str) -> Option<MessageTag> {
        let owner = match owner {
            Some(owner) => owner,
            None => self.owner_of(name)?,
        };
        (is_owner_name(owner) && is_repository_name(name)).then(|| {
            extracted(
                GITHUB_KEY,
                format!("{owner}/{name}#{number}").to_ascii_lowercase(),
            )
        })
    }

    fn tags(&self, content: &str, metadata: &Value) -> Vec<MessageTag> {
        let mut tags = Vec::new();
        let pull_request = &metadata["pull_request"];
        if let (Some(repository), Some(number)) = (
            pull_request["repository"].as_str(),
            pull_request["number"].as_i64(),
        ) {
            let (owner, name) = match repository.split_once('/') {
                Some((owner, name)) => (Some(owner), name),
                None => (None, repository),
            };
            tags.extend(self.github_tag(owner, name, &number.to_string()));
        }
        for link in GITHUB_LINK.captures_iter(content) {
            tags.extend(self.github_tag(Some(&link[1]), &link[2], &link[3]));
        }
        let without_links = ANY_LINK.replace_all(content, " ");
        for reference in REPOSITORY_REFERENCE.captures_iter(&without_links) {
            let owner = reference.get(1).map(|owner| owner.as_str());
            tags.extend(self.github_tag(owner, &reference[2], &reference[3]));
        }
        for link in LINEAR_LINK.captures_iter(content) {
            tags.push(extracted(LINEAR_KEY, link[1].to_ascii_uppercase()));
        }
        if let Some(bare_ticket) = &self.bare_ticket {
            for ticket in bare_ticket.find_iter(content) {
                tags.push(extracted(LINEAR_KEY, ticket.as_str().to_owned()));
            }
        }
        tags.retain(|tag| tag.validate().is_ok());
        tags
    }
}

pub(crate) fn stamp_tags(
    metadata: &mut Value,
    content: &str,
    encrypted: bool,
    extraction: &TagExtraction,
) -> Result<(), String> {
    let mut tags = author_tags(metadata)?;
    if encrypted && !tags.is_empty() {
        return Err("tags are not available in end-to-end encrypted rooms".to_owned());
    }
    if !encrypted {
        for tag in extraction.tags(content, metadata) {
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

/// Validates a subscription's tag filters, held to the same count as a message's tags.
pub(crate) fn validate_tag_filters(filters: &[String]) -> Result<(), String> {
    if filters.len() > MAX_TAGS_PER_MESSAGE {
        return Err(format!(
            "a subscription can filter on at most {MAX_TAGS_PER_MESSAGE} tags"
        ));
    }
    filters
        .iter()
        .try_for_each(|filter| MessageTag::parse(filter).map(drop))
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

/// A GitHub user or organisation name: letters, digits and hyphens.
fn is_owner_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 39
        && !name.starts_with('-')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// A GitHub repository name, the same for a link and for `owner/repo#N`.
fn is_repository_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    !name.is_empty()
        && name != "."
        && name != ".."
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
        && !DOCUMENT_EXTENSIONS
            .iter()
            .any(|extension| lower.ends_with(extension))
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

    /// Settings with two listed repositories, so bare `repo#N` can be tested.
    fn listed() -> TagExtraction {
        TagExtraction::new(
            &["COW".to_owned()],
            &["cowboyinc/cowchat".to_owned(), "cowboyinc/core".to_owned()],
        )
        .unwrap()
    }

    fn stamped(mut metadata: Value, content: &str) -> Value {
        stamp_tags(&mut metadata, content, false, &listed()).unwrap();
        metadata
    }

    fn extracted_with(extraction: &TagExtraction, content: &str) -> Vec<String> {
        let mut metadata = json!({});
        stamp_tags(&mut metadata, content, false, extraction).unwrap();
        metadata_tags(&metadata)
            .into_iter()
            .map(|tag| format!("{}={}", tag.key, tag.value.unwrap()))
            .collect()
    }

    fn extracted(content: &str) -> Vec<String> {
        extracted_with(&listed(), content)
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
            assert!(stamp_tags(&mut json!({"tags":tags}), "hello", false, &listed()).is_err());
        }
        let too_many: Vec<Value> = (0..=MAX_TAGS_PER_MESSAGE)
            .map(|index| json!({"key":format!("k{index}")}))
            .collect();
        assert!(stamp_tags(&mut json!({"tags":too_many}), "hello", false, &listed()).is_err());
    }

    #[test]
    fn subscription_tag_filters_are_capped_at_the_per_message_tag_limit() {
        let filters =
            |count: usize| -> Vec<String> { (0..count).map(|index| format!("k{index}")).collect() };
        assert!(validate_tag_filters(&filters(MAX_TAGS_PER_MESSAGE)).is_ok());
        let error = validate_tag_filters(&filters(MAX_TAGS_PER_MESSAGE + 1)).unwrap_err();
        assert!(error.contains(&MAX_TAGS_PER_MESSAGE.to_string()));
        assert!(validate_tag_filters(&["Bad Key".to_owned()]).is_err());
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
        let error =
            stamp_tags(&mut json!({"tags":[{"key":"topic"}]}), "x", true, &listed()).unwrap_err();
        assert!(error.contains("encrypted"));
    }

    #[test]
    fn encrypted_rooms_get_no_extracted_tags() {
        let mut metadata = json!({});
        stamp_tags(&mut metadata, "cowchat#95 COW-4857", true, &listed()).unwrap();
        assert_eq!(metadata, json!({}));
    }

    #[test]
    fn extractor_finds_repo_references_and_linear_tickets() {
        assert_eq!(
            extracted("see cowchat#95, cowboyinc/core#1403 and COW-4857."),
            [
                "github=cowboyinc/cowchat#95",
                "github=cowboyinc/core#1403",
                "linear=COW-4857"
            ]
        );
    }

    #[test]
    fn extractor_reads_github_links_and_linear_links() {
        assert_eq!(
            extracted("https://github.com/cowboyinc/cowchat/pull/95 and https://github.com/cowboyinc/core/issues/7"),
            ["github=cowboyinc/cowchat#95", "github=cowboyinc/core#7"]
        );
        assert_eq!(
            extracted("https://linear.app/cowboy/issue/COW-4857/room-plan"),
            ["linear=COW-4857"]
        );
    }

    #[test]
    fn extractor_gives_one_tag_per_identifier() {
        assert_eq!(
            extracted("Cowchat#95 cowchat#95 CowboyInc/Cowchat#95 https://github.com/cowboyinc/cowchat/pull/95"),
            ["github=cowboyinc/cowchat#95"]
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
    fn bare_words_and_file_anchors_are_not_tags() {
        for content in [
            "bug#3",
            "step#2",
            "docs/page.html#2",
            "see notes/README.md#3",
        ] {
            assert!(extracted(content).is_empty(), "{content}");
        }
    }

    #[test]
    fn the_owner_stays_in_the_tag() {
        assert_eq!(
            extracted("cowboyinc/core#1 someone/core#1"),
            ["github=cowboyinc/core#1", "github=someone/core#1"]
        );
    }

    #[test]
    fn a_bare_reference_needs_exactly_one_listed_repository() {
        assert!(extracted_with(&TagExtraction::default(), "cowchat#95").is_empty());
        let two_owners =
            TagExtraction::new(&[], &["a/core".to_owned(), "b/core".to_owned()]).unwrap();
        assert!(extracted_with(&two_owners, "core#1").is_empty());
        assert_eq!(extracted_with(&two_owners, "a/core#1"), ["github=a/core#1"]);
    }

    #[test]
    fn a_github_link_and_its_owner_shorthand_give_the_same_tag() {
        for (repository, tag) in [
            ("cowboyinc/cowchat", "github=cowboyinc/cowchat#7"),
            ("org/x", "github=org/x#7"),
            ("org/3d", "github=org/3d#7"),
            ("org/9", "github=org/9#7"),
            ("Org/My.Repo_", "github=org/my.repo_#7"),
            ("org/pr", "github=org/pr#7"),
        ] {
            let link = format!("https://github.com/{repository}/pull/7");
            let shorthand = format!("see {repository}#7.");
            assert_eq!(extracted(&link), [tag], "{link}");
            assert_eq!(extracted(&shorthand), [tag], "{shorthand}");
        }
    }

    #[test]
    fn linear_links_are_tagged_for_any_team() {
        assert_eq!(
            extracted("https://linear.app/acme/issue/ENG-42/fix-login"),
            ["linear=ENG-42"]
        );
        assert!(extracted("ENG-42").is_empty());
    }

    #[test]
    fn bare_ticket_teams_are_a_setting_that_defaults_to_cow() {
        assert_eq!(
            extracted_with(&TagExtraction::default(), "COW-1 ENG-2"),
            ["linear=COW-1"]
        );
        let eng = TagExtraction::new(&["eng".to_owned(), "OPS".to_owned()], &[]).unwrap();
        assert_eq!(
            extracted_with(&eng, "COW-1 ENG-2 OPS-3"),
            ["linear=ENG-2", "linear=OPS-3"]
        );
        let none = TagExtraction::new(&[String::new()], &[]).unwrap();
        assert!(extracted_with(&none, "COW-1").is_empty());
    }

    #[test]
    fn invalid_settings_are_refused() {
        for team in ["C W", "1COW", "COW-1", &"A".repeat(17)] {
            assert!(
                TagExtraction::new(&[team.to_owned()], &[]).is_err(),
                "{team}"
            );
        }
        for repository in ["cowchat", "a/b/c", "o.k/repo", "owner/"] {
            assert!(
                TagExtraction::new(&[], &[repository.to_owned()]).is_err(),
                "{repository}"
            );
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
            json!([{"key":"github","value":"cowboyinc/cowchat#95","source":"extractor"}])
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
